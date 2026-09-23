import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import { ArrowLeft, ArrowRight, ExternalLink, Globe, MousePointer2, Plus, RotateCw, Square, Undo2, X } from "lucide-react";
import { openExternalUrl } from "../externalLinks";
import { browserWorkspaceReducer, defaultBrowserWorkspace, normalizeBrowserUrl, readBrowserWorkspace, writeBrowserWorkspace } from "../browserWorkspace";
import { sanitizeBrowserSelection, type BrowserSelectionContext } from "../browserSelection";
import { hasNativeBrowser, type BrowserAction, type BrowserPageSnapshot } from "../browserRuntime";
import { BrowserPage, type BrowserPageHandle } from "./BrowserPage";
import { PaneState } from "./ui/pane";
import { cn } from "../lib/utils";

export type SimpleBrowserProps = {
  sessionId?: string;
  initialUrl?: string;
  visible?: boolean;
  onAttachSelection?: (context: BrowserSelectionContext) => void;
  onInvalidateSelection?: (tabId: string, navigationId?: number) => void;
};
const control = "grid size-7 shrink-0 place-items-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground disabled:pointer-events-none disabled:opacity-40";

export function SimpleBrowser({ sessionId = "preview", initialUrl = "", visible = true, onAttachSelection, onInvalidateSelection }: SimpleBrowserProps) {
  const [workspace, dispatch] = useReducer(browserWorkspaceReducer, undefined, () => initialUrl ? defaultBrowserWorkspace(initialUrl) : readBrowserWorkspace(sessionId));
  const [snapshots, setSnapshots] = useState<Record<string, BrowserPageSnapshot>>({});
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string>();
  const [inspecting, setInspecting] = useState(false);
  const [selection, setSelection] = useState<BrowserSelectionContext>();
  const [annotation, setAnnotation] = useState("");
  const [attaching, setAttaching] = useState(false);
  const [slow, setSlow] = useState(false);
  const pages = useRef(new Map<string, BrowserPageHandle>());
  const pendingNavigation = useRef(new Map<string, number>());
  const requestedNavigation = useRef(new Map<string, string>());
  const selectionRevision = useRef(0);
  const root = useRef<HTMLDivElement>(null);
  const address = useRef<HTMLInputElement>(null);
  const addressFocusFrame = useRef<number>();
  const addressFocusRevision = useRef(0);
  const stateRef = useRef(workspace);
  stateRef.current = workspace;
  const snapshotsRef = useRef(snapshots);
  snapshotsRef.current = snapshots;
  const callbacks = useRef({ onAttachSelection, onInvalidateSelection });
  callbacks.current = { onAttachSelection, onInvalidateSelection };
  const active = workspace.tabs.find(tab => tab.id === workspace.activeTabId)!;
  const live = snapshots[active.id];
  const mounted = useRef(true);
  const cancelAddressFocus = useCallback(() => {
    addressFocusRevision.current++;
    if (addressFocusFrame.current !== undefined) cancelAnimationFrame(addressFocusFrame.current);
    addressFocusFrame.current = undefined;
  }, []);
  const queueAddressFocus = useCallback((select = false) => {
    cancelAddressFocus();
    const revision = addressFocusRevision.current;
    addressFocusFrame.current = requestAnimationFrame(() => {
      if (!mounted.current || addressFocusRevision.current !== revision) return;
      addressFocusFrame.current = undefined;
      address.current?.focus();
      if (select) address.current?.select();
    });
  }, [cancelAddressFocus]);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; selectionRevision.current++; cancelAddressFocus(); }; }, [cancelAddressFocus]);
  useEffect(() => { writeBrowserWorkspace(sessionId, workspace); }, [sessionId, workspace]);
  useEffect(() => { selectionRevision.current++; setDraft(active.url); setError(undefined); setSelection(undefined); setInspecting(false); setAttaching(false); }, [active.id, active.url]);
  useEffect(() => {
    setSlow(false);
    if (!live?.loading) return;
    const timer = setTimeout(() => setSlow(true), 8000);
    return () => clearTimeout(timer);
  }, [active.id, live?.loading, live?.navigationId]);

  const cancelSelection = useCallback(() => {
    cancelAddressFocus();
    selectionRevision.current++;
    setSelection(undefined); setInspecting(false); setAnnotation(""); setAttaching(false);
    void pages.current.get(stateRef.current.activeTabId)?.action("cancel_inspect").catch(() => {});
  }, [cancelAddressFocus]);
  useEffect(() => { if (!visible) cancelSelection(); }, [visible, cancelSelection]);
  const invalidate = (tabId: string, navigationId?: number) => {
    callbacks.current.onInvalidateSelection?.(tabId, navigationId);
    if (stateRef.current.activeTabId === tabId) cancelSelection();
  };
  const run = async (task: () => Promise<unknown>) => {
    setError(undefined);
    try { await task(); }
    catch (reason) { if (mounted.current) setError(reason instanceof Error ? reason.message : String(reason)); }
  };
  const navigate = (input: string) => {
    const url = normalizeBrowserUrl(input);
    if (!url) { setError("Enter a valid HTTP or HTTPS address, or a search query."); return; }
    const tabId = active.id;
    invalidate(tabId);
    pendingNavigation.current.set(tabId, snapshotsRef.current[tabId]?.navigationId ?? -1);
    requestedNavigation.current.set(tabId, url);
    dispatch({ type: "navigate", tabId, url });
    void run(async () => {
      try { await pages.current.get(tabId)?.navigate(url); }
      catch (reason) { pendingNavigation.current.delete(tabId); requestedNavigation.current.delete(tabId); throw reason; }
    });
  };
  const createTab = (url = "") => {
    cancelSelection(); dispatch({ type: "create", url });
    queueAddressFocus();
  };
  const closeTab = (tabId: string) => {
    cancelAddressFocus();
    invalidate(tabId); pendingNavigation.current.delete(tabId); requestedNavigation.current.delete(tabId); dispatch({ type: "close", tabId });
    setSnapshots(previous => { const next = { ...previous }; delete next[tabId]; return next; });
  };
  const selectTab = (tabId: string) => { cancelSelection(); dispatch({ type: "select", tabId }); };
  const act = (action: BrowserAction) => {
    const tabId = active.id;
    if (action === "inspect") { selectionRevision.current++; setInspecting(true); setSelection(undefined); setAnnotation(""); setAttaching(false); }
    else invalidate(tabId);
    if (["back", "forward", "reload"].includes(action)) pendingNavigation.current.set(tabId, live?.navigationId ?? -1);
    void run(async () => {
      try {
        if ((action === "back" && (!live?.canGoBack || live.backUrl !== active.history[active.historyIndex - 1])) || (action === "forward" && (!live?.canGoForward || live.forwardUrl !== active.history[active.historyIndex + 1]))) {
        const step = action === "back" ? -1 : 1;
        const target = active.history[active.historyIndex + step];
        if (target) { dispatch({ type: action, tabId }); await pages.current.get(tabId)?.navigate(target); }
      } else {
        if (action === "back" || action === "forward") dispatch({ type: action, tabId });
        await pages.current.get(tabId)?.action(action);
      }
      } catch (reason) { pendingNavigation.current.delete(tabId); if (action === "inspect") setInspecting(false); throw reason; }
    });
  };
  const acceptSnapshot = (tabId: string, next: BrowserPageSnapshot) => {
    if (!mounted.current || !stateRef.current.tabs.some(tab => tab.id === tabId)) return;
    const pending = pendingNavigation.current.get(tabId);
    if (pending !== undefined && next.navigationId <= pending) return;
    pendingNavigation.current.delete(tabId);
    const previous = snapshotsRef.current[tabId];
    if (previous && next.navigationId < previous.navigationId) return;
    if (previous && previous.navigationId !== next.navigationId) {
      callbacks.current.onInvalidateSelection?.(tabId, next.navigationId);
      if (stateRef.current.activeTabId === tabId) { selectionRevision.current++; setSelection(undefined); setInspecting(false); setAttaching(false); }
    }
    setSnapshots(old => ({ ...old, [tabId]: next }));
    if (next.url && normalizeBrowserUrl(next.url) === next.url) {
      const requested = requestedNavigation.current.get(tabId);
      const tab = stateRef.current.tabs.find(item => item.id === tabId)!;
      // A native Back/Forward triggered inside the page should move the saved
      // cursor too. A normal link to an older URL has no forward branch.
      if (next.historyAction === "traverse") {
        dispatch({ type: "traverse", tabId, url: next.url });
        dispatch({ type: "update-title", tabId, title: next.title });
      } else {
        if (!requested && previous && next.url !== tab.url && next.canGoForward && tab.history[tab.historyIndex - 1] === next.url) dispatch({ type: "back", tabId });
        else if (!requested && previous?.canGoForward && next.url !== tab.url && tab.history[tab.historyIndex + 1] === next.url) dispatch({ type: "forward", tabId });
        dispatch({ type: "committed-navigation", tabId, url: next.url, title: next.title, replace: next.historyAction === "replace" || (!!requested && tab.url === requested && next.url !== requested) });
      }
      if (requested && next.loading) requestedNavigation.current.set(tabId, next.url);
      else if (!next.loading) requestedNavigation.current.delete(tabId);
    }
    else if (next.title) dispatch({ type: "update-title", tabId, title: next.title });
    if ((next.loading || next.error) && stateRef.current.activeTabId === tabId) {
      selectionRevision.current++; setSelection(undefined); setInspecting(false); setAttaching(false);
    }
    if (next.selection && !next.loading && !next.error && stateRef.current.activeTabId === tabId) {
      const captured = sanitizeBrowserSelection({ ...next.selection, url: next.url, title: next.title }, { sessionId, tabId, navigationId: next.navigationId });
      setInspecting(false);
      if (captured) { selectionRevision.current++; setSelection(captured); setAnnotation(""); setAttaching(false); }
      else setError("This element could not be safely captured. Select another element.");
    }
    if (next.cancelled && !next.selection && stateRef.current.activeTabId === tabId) {
      selectionRevision.current++; setSelection(undefined); setInspecting(false); setAttaching(false);
    }
    if (next.shortcut && stateRef.current.activeTabId === tabId && visible) {
      if (hasNativeBrowser()) void import("@tauri-apps/api/webview").then(({ getCurrentWebview }) => getCurrentWebview().setFocus()).catch(() => {});
      if (next.shortcut === "address") queueAddressFocus(true);
      else if (next.shortcut === "new_tab") createTab();
      else if (next.shortcut === "close_tab") closeTab(tabId);
      else dispatch({ type: "reopen" });
    }
    if (next.popupUrl) {
      const url = normalizeBrowserUrl(next.popupUrl);
      if (url) dispatch({ type: "create", url });
    }
  };
  const attach = async () => {
    if (!selection || !onAttachSelection) return;
    const expected = selection;
    const revision = selectionRevision.current;
    setAttaching(true);
    await run(async () => {
      let current: BrowserPageSnapshot | undefined;
      try { current = await pages.current.get(expected.tabId)?.snapshot(); }
      catch (reason) {
        if (!mounted.current || revision !== selectionRevision.current) return;
        throw reason;
      }
      // Cancellation is final even if an earlier snapshot resolves with the
      // same navigation ID. It must not attach or disturb a newer selection.
      if (!mounted.current || revision !== selectionRevision.current) return;
      if (stateRef.current.activeTabId !== expected.tabId || pendingNavigation.current.has(expected.tabId) || !current || current.loading || current.error || current.navigationId !== expected.navigationId) {
        cancelSelection(); throw new Error("The page changed. Select the element again before attaching it.");
      }
      const context = sanitizeBrowserSelection({ ...expected, annotations: annotation.trim() ? [{ kind: "note", text: annotation.trim() }, { kind: "rectangle", points: [expected.bounds.x, expected.bounds.y, expected.bounds.width, expected.bounds.height] }] : [{ kind: "rectangle", points: [expected.bounds.x, expected.bounds.y, expected.bounds.width, expected.bounds.height] }] }, { sessionId, tabId: expected.tabId, navigationId: expected.navigationId });
      if (!context) throw new Error("The selected context is no longer valid.");
      callbacks.current.onAttachSelection?.(context); cancelSelection();
    });
    if (mounted.current && revision === selectionRevision.current) setAttaching(false);
  };

  const canBack = active.historyIndex > 0;
  const canForward = active.historyIndex < active.history.length - 1;
  const pageError = live?.error;
  return <div ref={root} className="flex h-full min-w-0 flex-col bg-background" onKeyDown={event => {
    if (!visible || event.defaultPrevented) return;
    const command = event.metaKey || event.ctrlKey;
    if (event.key === "Escape") { cancelSelection(); return; }
    if (!command) return;
    if (event.key.toLowerCase() === "l") { event.preventDefault(); cancelAddressFocus(); address.current?.focus(); address.current?.select(); }
    else if (event.key.toLowerCase() === "t") { event.preventDefault(); if (event.shiftKey) dispatch({ type: "reopen" }); else createTab(); }
    else if (event.key.toLowerCase() === "w") { event.preventDefault(); closeTab(active.id); }
  }}>
    <div className="flex min-h-9 shrink-0 items-center gap-1 border-b border-border px-1">
      <div role="tablist" aria-label="Browser pages" className="flex min-w-0 flex-1 gap-1 overflow-x-auto">
        {workspace.tabs.map((tab, index) => <div key={tab.id} className={cn("flex shrink-0 items-center rounded-md", tab.id === active.id ? "bg-accent" : "hover:bg-accent/50")}>
          <button type="button" role="tab" id={`browser-tab-${tab.id}`} aria-controls={`browser-page-${tab.id}`} aria-selected={tab.id === active.id} tabIndex={tab.id === active.id ? 0 : -1} className="flex h-7 max-w-40 items-center gap-1.5 px-2 text-xs" title={tab.url || "New tab"} onClick={() => selectTab(tab.id)} onKeyDown={event => {
            let target = index;
            if (event.key === "ArrowRight") target = (index + 1) % workspace.tabs.length;
            else if (event.key === "ArrowLeft") target = (index + workspace.tabs.length - 1) % workspace.tabs.length;
            else if (event.key === "Home") target = 0;
            else if (event.key === "End") target = workspace.tabs.length - 1;
            else if (event.key === "Delete") { event.preventDefault(); closeTab(tab.id); return; }
            else return;
            event.preventDefault(); selectTab(workspace.tabs[target].id);
            document.getElementById(`browser-tab-${workspace.tabs[target].id}`)?.focus();
          }}>{snapshots[tab.id]?.loading ? <RotateCw size={12} className="shrink-0 animate-spin" aria-label="Loading page" /> : <Globe size={12} className="shrink-0" />}<span className="truncate">{tab.title || (tab.url ? new URL(tab.url).hostname : "New tab")}</span></button>
          <button type="button" aria-label={`Close ${tab.title || "browser tab"}`} title="Close tab" className="mr-1 grid size-5 place-items-center rounded text-muted-foreground hover:bg-muted" onClick={() => closeTab(tab.id)}><X size={11} /></button>
        </div>)}
      </div>
      <button type="button" className={control} aria-label="New browser tab" title="New tab" disabled={workspace.tabs.length >= 20} onClick={() => createTab()}><Plus size={14} /></button>
      <button type="button" className={control} aria-label="Reopen closed browser tab" title="Reopen closed tab" disabled={!workspace.recentlyClosed.length || workspace.tabs.length >= 20} onClick={() => dispatch({ type: "reopen" })}><Undo2 size={13} /></button>
    </div>
    <form className="flex min-h-10 shrink-0 items-center gap-1 border-b border-border px-1.5" onSubmit={event => { event.preventDefault(); navigate(draft); }}>
      <button type="button" className={control} aria-label="Back" disabled={!canBack} onClick={() => act("back")}><ArrowLeft size={13} /></button>
      <button type="button" className={control} aria-label="Forward" disabled={!canForward} onClick={() => act("forward")}><ArrowRight size={13} /></button>
      <button type="button" className={control} aria-label={live?.loading ? "Stop loading" : "Reload"} disabled={!active.url} onClick={() => act(live?.loading ? "stop" : "reload")}>{live?.loading ? <Square size={12} /> : <RotateCw size={12} />}</button>
      <input ref={address} value={draft} onChange={event => setDraft(event.target.value)} aria-label="Address" placeholder="Enter a URL or search" className="h-7 min-w-0 flex-1 rounded-md border border-input bg-background px-2 text-xs outline-none focus-visible:border-ring" />
      <button type="button" className={cn(control, inspecting && "bg-selection text-selection-foreground")} aria-label="Select page element" aria-pressed={inspecting} disabled={!active.url || !!pageError} title="Select an element to edit" onClick={() => inspecting ? cancelSelection() : act("inspect")}><MousePointer2 size={14} /></button>
      <button type="button" className={control} aria-label="Open in system browser" disabled={!active.url} onClick={() => void run(() => openExternalUrl(live?.url || active.url))}><ExternalLink size={13} /></button>
    </form>
    {!hasNativeBrowser() && <p className="border-b border-border px-3 py-1.5 text-[11px] text-muted-foreground">Web preview: some pages cannot be embedded. Full navigation and element selection are available in the desktop app.</p>}
    {error && <div role="alert" className="flex items-start gap-2 border-b border-border px-3 py-2 text-xs text-destructive"><span className="flex-1">{error}</span><button type="button" onClick={() => setError(undefined)} aria-label="Dismiss browser error"><X size={13} /></button></div>}
    {slow && <div role="status" className="border-b border-border px-3 py-2 text-xs text-muted-foreground">Still loading. You can wait, stop loading, or open this page in your browser.</div>}
    {inspecting && <div role="status" className="flex items-center justify-between border-b border-border px-3 py-2 text-xs">Select an element in the page. Escape cancels.<button type="button" onClick={cancelSelection}>Cancel</button></div>}
    <div className="relative min-h-0 flex-1">
      {workspace.tabs.map(tab => <div key={tab.id} id={`browser-page-${tab.id}`} role="tabpanel" aria-labelledby={`browser-tab-${tab.id}`} className={cn("h-full", tab.id !== active.id && "hidden")}>
        <BrowserPage ref={page => { if (page) pages.current.set(tab.id, page); else pages.current.delete(tab.id); }} sessionId={sessionId} tabId={tab.id} initialUrl={tab.url} visible={visible && tab.id === active.id && !!tab.url && !snapshots[tab.id]?.error && !selection} onSnapshot={next => acceptSnapshot(tab.id, next)} onError={message => { if (stateRef.current.activeTabId === tab.id) setError(message); }} />
      </div>)}
      {!active.url && <div className="absolute inset-0 bg-background"><PaneState icon={Globe} title="New browser tab">Enter an address above to open a page.</PaneState></div>}
      {pageError && <div className="absolute inset-0 overflow-auto bg-background"><PaneState role="alert" icon={Globe} title="Page could not load" action={<><button type="button" className="u-glass-soft rounded-md px-3 py-2 text-xs" onClick={() => act("reload")}>Retry</button><button type="button" className="u-glass-soft rounded-md px-3 py-2 text-xs" onClick={() => void run(() => openExternalUrl(active.url))}>Open in system browser</button></>}>{pageError}</PaneState></div>}
      {selection && <section aria-label="Selected page element" className="absolute inset-0 flex flex-col gap-3 overflow-auto bg-background p-4">
        <h3 className="font-display text-sm font-semibold">Selected element</h3>
        <p className="break-all text-xs text-muted-foreground">{selection.url}</p>
        <code className="rounded-md bg-muted p-2 text-xs">{selection.selector}</code>
        <pre className="max-h-40 overflow-auto whitespace-pre-wrap break-words rounded-md bg-muted p-2 text-xs">{selection.snippet}</pre>
        <p className="text-xs text-muted-foreground">The element’s outline and your annotation will be attached as page context. Write the change you want in the composer.</p>
        <label className="flex flex-col gap-1 text-xs">Annotation<textarea aria-label="Selection annotation" maxLength={1000} value={annotation} onChange={event => setAnnotation(event.target.value)} placeholder="Describe the area or mark what should change…" className="u-glass-soft min-h-20 resize-y rounded-md p-2 outline-none focus-visible:ring-1 focus-visible:ring-ring" /></label>
        <div className="flex gap-2"><button type="button" disabled={!onAttachSelection || attaching} onClick={() => void attach()} className="rounded-md bg-primary px-3 py-2 text-xs text-primary-foreground disabled:opacity-40">{attaching ? "Attaching…" : "Attach to prompt"}</button><button type="button" onClick={cancelSelection} className="u-glass-soft rounded-md px-3 py-2 text-xs">Cancel</button></div>
        <p className="text-[11px] text-muted-foreground">Select the element again after navigating to a different page.</p>
      </section>}
    </div>
  </div>;
}
