import { forwardRef, useEffect, useImperativeHandle, useRef } from "react";
import { browserCommand, browserViewportVisible, hasNativeBrowser, registerBrowserPageValidator, type BrowserAction, type BrowserPageSnapshot } from "../browserRuntime";
import { createCoalescedRefresh, startSerialPoll } from "../polling";

export type BrowserPageHandle = {
  navigate(url: string): Promise<void>;
  action(action: BrowserAction): Promise<void>;
  snapshot(): Promise<BrowserPageSnapshot | undefined>;
};
export type BrowserPageProps = {
  sessionId: string;
  tabId: string;
  initialUrl: string;
  visible: boolean;
  onSnapshot(snapshot: BrowserPageSnapshot): void;
  onError(message: string): void;
};

/** A page survives tab and dock switches. Only closing it destroys its engine. */
export const BrowserPage = forwardRef<BrowserPageHandle, BrowserPageProps>(function BrowserPage(props, ref) {
  const container = useRef<HTMLDivElement>(null);
  const current = useRef(props);
  current.current = props;
  const engine = useRef<BrowserPageHandle>();
  const ready = useRef<Promise<void>>(Promise.resolve());
  useImperativeHandle(ref, () => ({
    navigate: async url => { await ready.current; await engine.current?.navigate(url); },
    action: async action => { await ready.current; await engine.current?.action(action); },
    snapshot: async () => { await ready.current; return engine.current?.snapshot(); },
  }), []);

  useEffect(() => {
    const host = container.current!;
    const { sessionId, initialUrl } = current.current;
    // Each mount owns its native handle, including overlapping StrictMode cleanup.
    const tabId = `${current.current.tabId}-${crypto.randomUUID()}`;
    let disposed = false;
    const unregister = registerBrowserPageValidator(sessionId, current.current.tabId, async () => {
      await ready.current;
      if (disposed) return undefined;
      const result = await engine.current?.snapshot();
      if (disposed) return undefined;
      if (result) current.current.onSnapshot(result);
      return result;
    });
    let stop = () => {};
    let dispose = () => {};
    const fail = (error: unknown) => { if (!disposed) current.current.onError(error instanceof Error ? error.message : String(error)); };
    if (hasNativeBrowser()) {
      ready.current = browserCommand("create", sessionId, tabId, { url: initialUrl }).then(async () => {
        if (disposed) { await browserCommand("close", sessionId, tabId); return; }
        const snapshot = () => browserCommand<BrowserPageSnapshot>("snapshot", sessionId, tabId);
        engine.current = {
          navigate: url => browserCommand("navigate", sessionId, tabId, { url }),
          action: action => browserCommand("action", sessionId, tabId, { action }),
          snapshot,
        };
        let lastLayout = "";
        const layout = createCoalescedRefresh(async () => {
          if (disposed) return;
          const rect = host.getBoundingClientRect();
          const bounds = { x: Math.max(0, rect.left), y: Math.max(0, rect.top), width: Math.max(1, rect.width), height: Math.max(1, rect.height), visible: browserViewportVisible(host, current.current.visible) };
          const signature = JSON.stringify(bounds);
          if (signature === lastLayout) return;
          lastLayout = signature;
          try { await browserCommand("layout", sessionId, tabId, bounds); }
          catch (error) { lastLayout = ""; fail(error); }
        });
        const resize = new ResizeObserver(() => void layout());
        resize.observe(host);
        const mutations = new MutationObserver(() => void layout());
        mutations.observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ["class", "style", "aria-hidden", "open"] });
        window.addEventListener("resize", layout);
        document.addEventListener("visibilitychange", layout);
        let lastSnapshot = 0;
        stop = startSerialPoll(async () => {
          await layout();
          if (disposed || (!current.current.visible && Date.now() - lastSnapshot < 2000)) return;
          lastSnapshot = Date.now();
          try {
            const result = await snapshot();
            if (!disposed) current.current.onSnapshot(result);
          } catch (error) { fail(error); }
        }, 350);
        dispose = () => {
          resize.disconnect(); mutations.disconnect();
          window.removeEventListener("resize", layout);
          document.removeEventListener("visibilitychange", layout);
          void browserCommand("close", sessionId, tabId).catch(() => {});
        };
      });
      // Report initialization errors without converting readiness to a success:
      // subsequent navigation must fail visibly instead of silently doing nothing.
      void ready.current.catch(fail);
    } else {
      // Web-only preview: browser security limits cross-origin iframe inspection.
      // The desktop uses independent native pages and is not subject to framing.
      const frame = document.createElement("iframe");
      frame.title = "Browser page";
      frame.className = "h-full w-full border-0 bg-background";
      frame.setAttribute("sandbox", "allow-scripts allow-same-origin allow-forms allow-popups allow-popups-to-escape-sandbox");
      let state: BrowserPageSnapshot = { url: initialUrl, title: "", canGoBack: false, canGoForward: false, loading: !!initialUrl, navigationId: 0 };
      const publish = () => { if (!disposed) current.current.onSnapshot({ ...state }); };
      frame.onload = () => {
        try {
          const url = frame.contentWindow?.location.href;
          if (url && url !== "about:blank") state.url = url;
          state.title = frame.contentDocument?.title ?? "";
        } catch { /* Cross-origin DOM access is intentionally unavailable. */ }
        state.loading = false; state.error = undefined; publish();
      };
      frame.onerror = () => { state.loading = false; state.error = "Page could not load in this preview. Retry or open it in your browser."; publish(); };
      engine.current = {
        navigate: async url => { state = { ...state, url, loading: true, navigationId: state.navigationId + 1, error: undefined }; frame.src = url; publish(); },
        action: async action => {
          if (action === "reload") { state.loading = true; state.error = undefined; state.navigationId++; frame.src = state.url; publish(); }
          else if (action === "stop") { try { frame.contentWindow?.stop(); } catch { /* cross-origin */ } state.loading = false; publish(); }
          else if (action === "inspect") throw new Error("Element selection is available in the Bridge desktop app. Web previews cannot inspect arbitrary cross-origin pages.");
          else if (action !== "cancel_inspect") throw new Error("Use address history in the web preview.");
        },
        snapshot: async () => ({ ...state }),
      };
      if (initialUrl) frame.src = initialUrl;
      host.appendChild(frame);
      stop = startSerialPoll(async () => {
        if (!current.current.visible) return;
        try {
          const url = frame.contentWindow?.location.href;
          if (url && url !== "about:blank" && url !== state.url) {
            state.url = url; state.navigationId++; publish();
          }
        } catch { /* no cross-origin privileges in preview */ }
      }, 350);
      dispose = () => frame.remove();
    }
    return () => { disposed = true; unregister(); stop(); dispose(); engine.current = undefined; };
  }, [props.sessionId, props.tabId]);
  return <div ref={container} className="h-full min-h-0 w-full" data-browser-viewport={props.tabId} />;
});
