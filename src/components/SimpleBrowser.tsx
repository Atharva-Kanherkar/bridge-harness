import { useCallback, useRef, useState } from "react";
import { ArrowLeft, ArrowRight, ExternalLink, Globe, RotateCw } from "lucide-react";
import { openExternalUrl } from "../externalLinks";
import { PaneState } from "@/components/ui/pane";
import { cn } from "@/lib/utils";

// A plain in-app browser, the VS Code "Simple Browser" idea: an address bar,
// back/forward/reload, and a sandboxed iframe. No extension, no relay, no
// lease — just a webview for pointing at docs, previews, or localhost while
// staying inside the dock. Sites that refuse to be framed (X-Frame-Options,
// CSP frame-ancestors) fail silently in an iframe, so a load timeout falls
// back to an inline notice with a system-browser escape hatch.

const DEFAULT_URL_SCHEME = /^https?:\/\//i;

/** Best-effort normalization: bare host or search-shaped input becomes a
 *  navigable URL, the way an OS omnibox would treat it. */
function normalizeUrl(input: string): string | undefined {
  const trimmed = input.trim();
  if (!trimmed) return undefined;
  if (DEFAULT_URL_SCHEME.test(trimmed)) return trimmed;
  if (/^[\w-]+(\.[\w-]+)+(:\d+)?(\/.*)?$/.test(trimmed) || /^localhost(:\d+)?(\/.*)?$/.test(trimmed)) {
    return `https://${trimmed}`;
  }
  return `https://www.google.com/search?q=${encodeURIComponent(trimmed)}`;
}

/** An iframe blocked by X-Frame-Options/CSP never fires `load` with content —
 *  it either fires `load` on an empty document or never settles. Either way,
 *  a short grace period after navigation is the only signal available from
 *  outside the frame (cross-origin content is unreadable), so a stall past
 *  this window reads as "probably blocked" rather than "still loading". */
const LOAD_GRACE_MS = 4000;

export function SimpleBrowser({ initialUrl = "" }: { initialUrl?: string }) {
  const [draft, setDraft] = useState(initialUrl);
  const [url, setUrl] = useState<string | undefined>(normalizeUrl(initialUrl));
  const [history, setHistory] = useState<string[]>(url ? [url] : []);
  const [index, setIndex] = useState(url ? 0 : -1);
  const [loading, setLoading] = useState(false);
  const [blocked, setBlocked] = useState(false);
  const [reloadKey, setReloadKey] = useState(0);
  const graceTimer = useRef<ReturnType<typeof setTimeout>>();

  const navigate = useCallback((target: string, mode: "push" | "replace" = "push") => {
    const next = normalizeUrl(target);
    if (!next) return;
    setDraft(target);
    setUrl(next);
    setBlocked(false);
    setLoading(true);
    setReloadKey(k => k + 1);
    if (mode === "push") {
      setHistory(previous => {
        const trimmed = previous.slice(0, index + 1);
        if (trimmed[trimmed.length - 1] === next) return trimmed;
        return [...trimmed, next];
      });
      setIndex(previous => previous + 1);
    }
    clearTimeout(graceTimer.current);
    graceTimer.current = setTimeout(() => { setLoading(false); setBlocked(true); }, LOAD_GRACE_MS);
  }, [index]);

  const canBack = index > 0;
  const canForward = index >= 0 && index < history.length - 1;

  const goBack = () => {
    if (!canBack) return;
    const target = history[index - 1];
    setIndex(index - 1);
    setDraft(target);
    setUrl(target);
    setBlocked(false);
    setLoading(true);
    setReloadKey(k => k + 1);
    clearTimeout(graceTimer.current);
    graceTimer.current = setTimeout(() => { setLoading(false); setBlocked(true); }, LOAD_GRACE_MS);
  };
  const goForward = () => {
    if (!canForward) return;
    const target = history[index + 1];
    setIndex(index + 1);
    setDraft(target);
    setUrl(target);
    setBlocked(false);
    setLoading(true);
    setReloadKey(k => k + 1);
    clearTimeout(graceTimer.current);
    graceTimer.current = setTimeout(() => { setLoading(false); setBlocked(true); }, LOAD_GRACE_MS);
  };
  const reload = () => {
    if (!url) return;
    setBlocked(false);
    setLoading(true);
    setReloadKey(k => k + 1);
    clearTimeout(graceTimer.current);
    graceTimer.current = setTimeout(() => { setLoading(false); setBlocked(true); }, LOAD_GRACE_MS);
  };

  const onSubmit = (event: React.FormEvent) => {
    event.preventDefault();
    navigate(draft);
  };

  const onFrameLoad = () => {
    clearTimeout(graceTimer.current);
    setLoading(false);
  };

  return <div className="flex h-full flex-col bg-code">
    <form onSubmit={onSubmit} className="flex min-h-10 shrink-0 items-center gap-1 border-b border-border px-1.5">
      <button
        type="button"
        onClick={goBack}
        disabled={!canBack}
        aria-label="Back"
        title="Back"
        className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground disabled:pointer-events-none disabled:opacity-40"
      ><ArrowLeft size={13} strokeWidth={1.8} aria-hidden="true" /></button>
      <button
        type="button"
        onClick={goForward}
        disabled={!canForward}
        aria-label="Forward"
        title="Forward"
        className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground disabled:pointer-events-none disabled:opacity-40"
      ><ArrowRight size={13} strokeWidth={1.8} aria-hidden="true" /></button>
      <button
        type="button"
        onClick={reload}
        disabled={!url}
        aria-label="Reload"
        title="Reload"
        className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground disabled:pointer-events-none disabled:opacity-40"
      ><RotateCw size={12} strokeWidth={1.8} aria-hidden="true" className={cn(loading && "animate-spin")} /></button>
      <input
        type="text"
        value={draft}
        onChange={event => setDraft(event.target.value)}
        placeholder="Enter a URL"
        aria-label="Address"
        className="h-7.5 min-w-0 flex-1 rounded-md border border-input bg-background px-2.5 text-[12.5px] text-foreground outline-none placeholder:text-muted-foreground focus-visible:border-ring"
      />
      {url && <button
        type="button"
        onClick={() => void openExternalUrl(url)}
        aria-label="Open in system browser"
        title="Open in system browser"
        className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground"
      ><ExternalLink size={13} strokeWidth={1.8} aria-hidden="true" /></button>}
    </form>
    <div className="relative min-h-0 flex-1">
      {!url ? (
        <PaneState icon={Globe} title="No page open">Type a URL above and press Enter.</PaneState>
      ) : blocked ? (
        <PaneState icon={Globe} title="This page can't be embedded" action={
          <button
            type="button"
            onClick={() => void openExternalUrl(url)}
            className="u-glass-soft min-h-8 rounded-lg px-3 text-[13px] text-foreground transition-colors hover:bg-accent"
          >
            Open in system browser
          </button>
        }>Some sites block being shown inside another page. Open it in your default browser instead.</PaneState>
      ) : (
        <iframe
          key={reloadKey}
          src={url}
          title="Simple browser"
          onLoad={onFrameLoad}
          sandbox="allow-scripts allow-same-origin allow-forms allow-popups allow-popups-to-escape-sandbox"
          className="h-full w-full border-0 bg-background"
        />
      )}
    </div>
  </div>;
}
