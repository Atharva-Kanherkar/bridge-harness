/**
 * The host half of MCP Apps for Bridge's visual and personal-app previews.
 *
 * The view runs in `<iframe sandbox="allow-scripts">` with no
 * `allow-same-origin`, so it has an opaque origin, and under a CSP injected
 * ahead of its own markup. Personal content additionally sits inside a trusted
 * relay frame: its frame-src policy blocks the inner view's self-navigation.
 * Visuals receive validated data; personal views receive stored, hashed HTML.
 *
 * Messages go through the official ext-apps `AppBridge` over a
 * `PostMessageTransport` that accepts only messages whose source is this
 * frame's window. The bridge connects before the document loads, so the
 * view's `ui/initialize` can never arrive to nobody.
 */

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { McpUiHostContext } from "@modelcontextprotocol/ext-apps/app-bridge";
import type { CallToolResult } from "@modelcontextprotocol/client";
import { cn } from "@/lib/utils";
import { personalViewDocument, viewDocument } from "./viewDocument";
import { useLatest } from "../useLatest";
export { VIEW_CSP, viewDocument } from "./viewDocument";

export type DisplayMode = "inline" | "fullscreen";

export interface FrameActions {
  /** A follow-up the view proposes. Fills the composer; never sends. */
  ask?: (text: string) => void;
  /** A source link the reader clicked. */
  open?: (url: string) => void;
}

/** Bridge's tokens under the MCP Apps standard variable names. */
function styleVariables(): Record<string, string> {
  if (typeof document === "undefined") return {};
  const css = getComputedStyle(document.documentElement);
  const read = (name: string) => css.getPropertyValue(name).trim();
  const entries: [string, string][] = [
    ["--color-background-primary", read("--card")],
    ["--color-background-secondary", read("--muted")],
    ["--color-text-primary", read("--foreground")],
    ["--color-text-secondary", read("--muted-foreground")],
    ["--color-border-primary", read("--border")],
    ["--color-ring-primary", read("--ring")],
    ["--font-sans", read("--font-sans")],
    ["--font-mono", read("--font-mono")],
  ];
  return Object.fromEntries(entries.filter(([, value]) => value));
}

function hostContext(theme: "light" | "dark", displayMode: DisplayMode): McpUiHostContext {
  return {
    theme,
    displayMode,
    availableDisplayModes: ["inline", "fullscreen"],
    platform: "desktop",
    locale: typeof navigator === "undefined" ? "en-US" : navigator.language,
    timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    // Only the variables Bridge has a token for; the rest stay unset.
    styles: { variables: styleVariables() as NonNullable<McpUiHostContext["styles"]>["variables"] },
  };
}

interface Props {
  /** The tool input: the `VisualSpec` the model sent. */
  input: Record<string, unknown>;
  theme: "light" | "dark";
  displayMode?: DisplayMode;
  actions?: FrameActions;
  className?: string;
  /** Height to hold before the view reports its own. */
  initialHeight?: number;
  onHeight?: (height: number) => void;
  title: string;
  /** Personal content is always nested under the trusted navigation guard. */
  personalView?: { html: string; resourceUri: string };
  onToolCall?: (tool: string, args: Record<string, unknown>) => Promise<CallToolResult>;
}

type Bridge = {
  setHostContext(context: Record<string, unknown>): void;
  sendToolInput(params: { arguments: Record<string, unknown> }): Promise<void>;
  close(): Promise<void>;
};

export function McpAppFrame({ input, theme, displayMode = "inline", actions, className, initialHeight = 320, onHeight, title, personalView, onToolCall }: Props) {
  const frame = useRef<HTMLIFrameElement>(null);
  const bridge = useRef<Bridge | null>(null);
  const [height, setHeight] = useState(initialHeight);
  const latest = useLatest({ input, theme, displayMode, actions, onHeight, onToolCall });

  useLayoutEffect(() => {
    const node = frame.current;
    if (!node) return;
    let cancelled = false;
    void (async () => {
      // Both load on the first visual, not with the app: the view alone is
      // over half a megabyte.
      const [{ AppBridge, PostMessageTransport }, viewHtml] = await Promise.all([
        import("@modelcontextprotocol/ext-apps/app-bridge"),
        personalView ? Promise.resolve(personalView.html) : import("./visual/generated/visual.html?raw").then(module => module.default),
      ]);
      const target = node.contentWindow;
      if (cancelled || !target) return;
      const app = new AppBridge(
        null,
        { name: "bridge", version: "1" },
        personalView ? { serverTools: {} } : { openLinks: {}, message: { text: {} } },
        { hostContext: hostContext(latest.current.theme, latest.current.displayMode) },
      );
      app.oninitialized = () => {
        void app.sendToolInput({ arguments: latest.current.input });
      };
      app.onsizechange = ({ height: next }) => {
        if (typeof next !== "number" || !Number.isFinite(next)) return;
        const clamped = Math.max(48, Math.min(personalView ? 1200 : Infinity, Math.ceil(next)));
        setHeight(clamped);
        latest.current.onHeight?.(clamped);
      };
      app.onmessage = async ({ content }) => {
        if (personalView) throw new Error("Use the native Revise button");
        const text = (Array.isArray(content) ? content : [])
          .map(part => (part && typeof part === "object" && "text" in part && typeof part.text === "string" ? part.text : ""))
          .join("\n")
          .trim();
        if (text) latest.current.actions?.ask?.(text);
        return {};
      };
      app.onopenlink = async ({ url }) => {
        if (personalView) throw new Error("Personal previews cannot open external links");
        if (/^https?:\/\//i.test(url)) latest.current.actions?.open?.(url);
        return {};
      };
      app.onrequestdisplaymode = async () => ({ mode: latest.current.displayMode });
      if (personalView) app.oncalltool = async ({ name, arguments: args }) => {
        if (!latest.current.onToolCall) throw new Error("Live tools are unavailable in this preview");
        return latest.current.onToolCall(name, args ?? {});
      };
      await app.connect(new PostMessageTransport(target, target));
      if (cancelled) {
        void app.close();
        return;
      }
      bridge.current = app as unknown as Bridge;
      // Only now that the bridge listens does the view get its document.
      const initial = { theme: latest.current.theme, variables: styleVariables() };
      node.srcdoc = personalView ? personalViewDocument(viewHtml, initial) : viewDocument(viewHtml, initial);
    })();
    return () => {
      cancelled = true;
      void bridge.current?.close();
      bridge.current = null;
    };
  }, [personalView?.html, personalView?.resourceUri]);

  useEffect(() => {
    bridge.current?.setHostContext(hostContext(theme, displayMode));
  }, [theme, displayMode]);

  return (
    <iframe
      ref={frame}
      title={title}
      sandbox="allow-scripts"
      referrerPolicy="no-referrer"
      className={cn("block w-full border-0 bg-transparent", theme === "dark" ? "[color-scheme:dark]" : "[color-scheme:light]", className)}
      style={displayMode === "inline" ? { height } : undefined}
      data-mcp-app={personalView?.resourceUri ?? "ui://bridge/visual"}
    />
  );
}

/**
 * Mount the frame only while the card is near the viewport. A long chat with
 * many visuals keeps a handful of live frames, and an unmounted card holds
 * the height its view last reported so nothing jumps.
 */
export function useNearViewport<T extends Element>(margin = "800px"): [React.RefObject<T>, boolean] {
  const ref = useRef<T>(null);
  const [near, setNear] = useState(typeof IntersectionObserver === "undefined");
  useEffect(() => {
    const node = ref.current;
    if (!node || typeof IntersectionObserver === "undefined") return;
    const observer = new IntersectionObserver(entries => {
      for (const entry of entries) setNear(entry.isIntersecting);
    }, { rootMargin: `${margin} 0px` });
    observer.observe(node);
    return () => observer.disconnect();
  }, [margin]);
  return [ref, near];
}
