import { createContext, useCallback, useContext, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { ExternalLink, ImageIcon, ImageOff, Play, X } from "lucide-react";
import { bridgeApi } from "../../api";
import { cn } from "@/lib/utils";
import { Skeleton } from "@/components/ui/skeleton";
import type { GithubMediaItem } from "../githubMarkdown";
import { Tip } from "@/components/ui/tooltip";

// Images and videos inside a GitHub body.
//
// Two rules decide how a piece of media loads:
//
// 1. Media on a GitHub-owned host loads on its own, as it does on github.com.
//    Anything else waits for a click. A comment body is written by someone
//    else, and an image on their server is a request that tells them who
//    opened the pull request and when. GitHub hides that behind its proxy; the
//    pane hides it behind a button.
// 2. The webview tries the address first. When that fails (an attachment on a
//    private repository, or a video served without a media type) the backend
//    fetches it with the signed-in account and the pane plays the bytes.

/** Which workspace the media belongs to, and which host counts as GitHub. */
export const GithubMediaScope = createContext<{ workspaceId: string; host: string } | null>(null);

export function isGithubHosted(url: string, repositoryHost = "github.com"): boolean {
  try {
    const parsed = new URL(url);
    if (parsed.protocol !== "https:") return false;
    const host = parsed.hostname.toLowerCase();
    return host === repositoryHost.toLowerCase()
      || host === "github.com"
      || host.endsWith(".githubusercontent.com")
      || host.endsWith(".githubassets.com");
  } catch {
    return false;
  }
}

const hostOf = (url: string): string => {
  try { return new URL(url).hostname; } catch { return "another site"; }
};

// One fetch per address for the life of the window: a re-render, a tab switch
// or a second copy of the same screenshot must not download it again.
const fetched = new Map<string, Promise<string>>();

function fetchThroughBackend(workspaceId: string, url: string): Promise<string> {
  const key = `${workspaceId}\n${url}`;
  let pending = fetched.get(key);
  if (!pending) {
    pending = bridgeApi.githubMedia(workspaceId, url).then(media => {
      const binary = atob(media.data);
      const bytes = new Uint8Array(binary.length);
      for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
      return URL.createObjectURL(new Blob([bytes], { type: media.mimeType }));
    });
    // A failure is not remembered, so Retry can actually retry.
    pending.catch(() => fetched.delete(key));
    fetched.set(key, pending);
  }
  return pending;
}

type Phase = "waiting" | "loading" | "ready" | "failed";

/** The source to show and what happened to it. `direct` is the address as
 *  written; a load error swaps in the backend's copy once, then gives up. */
function useMediaSource(url: string) {
  const scope = useContext(GithubMediaScope);
  const trusted = isGithubHosted(url, scope?.host);
  const [phase, setPhase] = useState<Phase>(trusted ? "loading" : "waiting");
  const [source, setSource] = useState(url);
  const usedBackend = useRef(false);
  const alive = useRef(true);
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);

  const onError = useCallback(() => {
    if (usedBackend.current || !scope || !trusted) { setPhase("failed"); return; }
    usedBackend.current = true;
    setPhase("loading");
    fetchThroughBackend(scope.workspaceId, url).then(
      blob => { if (alive.current) setSource(blob); },
      () => { if (alive.current) setPhase("failed"); },
    );
  }, [scope, trusted, url]);
  const retry = useCallback(() => { usedBackend.current = false; setSource(url); setPhase("loading"); }, [url]);

  return { phase, source, trusted, onLoad: () => setPhase("ready"), onError, allow: () => setPhase("loading"), retry };
}

const FRAME = "relative overflow-hidden rounded-lg border border-border bg-muted/40";
const OPEN_LINK = "inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-caption text-muted-foreground transition-colors hover:bg-accent hover:text-foreground";

function MediaNotice({ icon: Icon, children }: { icon: typeof ImageIcon; children: React.ReactNode }) {
  return <div className={cn(FRAME, "animate-page-mount flex min-h-16 flex-wrap items-center gap-2 px-3 py-2.5 text-meta text-muted-foreground")}>
    <Icon size={14} className="shrink-0" aria-hidden="true" />
    {children}
  </div>;
}

function OpenOriginal({ url, what }: { url: string; what: string }) {
  return <a href={url} target="_blank" rel="noreferrer" data-system-browser className={OPEN_LINK}>
    <ExternalLink size={10.5} aria-hidden="true" /> Open {what}
  </a>;
}

function Lightbox({ source, alt, original, onClose }: { source: string; alt: string; original: string; onClose: () => void }) {
  const reduced = useReducedMotion();
  const close = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    const before = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    close.current?.focus();
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault(); event.stopPropagation();
      onClose();
    };
    document.addEventListener("keydown", onKey, true);
    return () => { document.removeEventListener("keydown", onKey, true); before?.focus(); };
  }, [onClose]);
  return <motion.div
    role="dialog"
    aria-modal="true"
    aria-label={alt || "Image"}
    className="fixed inset-0 z-[90] grid place-items-center bg-background/90 p-6 backdrop-blur-md"
    initial={{ opacity: 0 }}
    animate={{ opacity: 1 }}
    exit={{ opacity: 0 }}
    transition={{ duration: reduced ? 0 : 0.18 }}
    onClick={onClose}
  >
    <motion.img
      src={source}
      alt={alt}
      className="max-h-[88vh] max-w-[92vw] rounded-lg border border-border object-contain shadow-2xl"
      initial={reduced ? false : { scale: 0.94, opacity: 0 }}
      animate={{ scale: 1, opacity: 1 }}
      exit={reduced ? undefined : { scale: 0.97, opacity: 0 }}
      transition={{ duration: reduced ? 0 : 0.26, ease: [0.22, 1, 0.36, 1] }}
      onClick={event => event.stopPropagation()}
    />
    <div className="absolute right-4 top-4 flex items-center gap-1.5" onClick={event => event.stopPropagation()}>
      <a href={original} target="_blank" rel="noreferrer" data-system-browser className="inline-flex h-8 items-center gap-1.5 rounded-md border border-border bg-card px-2.5 text-meta text-foreground shadow-xs transition-colors hover:bg-accent">
        <ExternalLink size={12} aria-hidden="true" /> Open original
      </a>
      <button ref={close} type="button" onClick={onClose} aria-label="Close image" className="grid size-8 place-items-center rounded-md border border-border bg-card text-foreground shadow-xs transition-colors hover:bg-accent">
        <X size={14} aria-hidden="true" />
      </button>
    </div>
  </motion.div>;
}

function GithubImage({ item }: { item: GithubMediaItem }) {
  const media = useMediaSource(item.url);
  const [zoomed, setZoomed] = useState(false);
  const closeZoom = useCallback(() => setZoomed(false), []);
  const label = item.alt || "image";

  if (media.phase === "waiting") return <MediaNotice icon={ImageIcon}>
    <span className="min-w-0 flex-1 truncate">{label}</span>
    <button type="button" onClick={media.allow} className="rounded-md border border-border bg-card px-2 py-0.5 text-caption font-medium text-foreground shadow-xs transition-colors hover:bg-accent active:scale-[0.98]">
      Load from {hostOf(item.url)}
    </button>
  </MediaNotice>;
  if (media.phase === "failed") return <MediaNotice icon={ImageOff}>
    <span className="min-w-0 flex-1 truncate">Couldn’t load {label}</span>
    <button type="button" onClick={media.retry} className={OPEN_LINK}>Retry</button>
    <OpenOriginal url={item.href ?? item.url} what="original" />
  </MediaNotice>;

  const ready = media.phase === "ready";
  const image = <img
    src={media.source}
    alt={item.alt}
    loading="lazy"
    decoding="async"
    referrerPolicy="no-referrer"
    onLoad={media.onLoad}
    onError={media.onError}
    className={cn("block h-auto max-h-[32rem] w-full object-contain transition-[opacity,transform,filter] duration-300 ease-out motion-reduce:transition-none", ready ? "opacity-100" : "scale-[0.985] opacity-0 blur-[3px]")}
  />;
  return <>
    <div className={cn(FRAME, !ready && "min-h-28")}>
      {!ready && <Skeleton className="absolute inset-0 h-full w-full rounded-none" />}
      {item.href
        ? <Tip label={item.alt || undefined}><a href={item.href} target="_blank" rel="noreferrer">{image}</a></Tip>
        : <button type="button" onClick={() => ready && setZoomed(true)} aria-label={`Enlarge ${label}`} className="block w-full cursor-zoom-in">{image}</button>}
    </div>
    {createPortal(<AnimatePresence>
      {zoomed && <Lightbox key="lightbox" source={media.source} alt={item.alt} original={item.url} onClose={closeZoom} />}
    </AnimatePresence>, document.body)}
  </>;
}

function GithubVideo({ item }: { item: GithubMediaItem }) {
  const media = useMediaSource(item.url);
  if (media.phase === "waiting") return <MediaNotice icon={Play}>
    <span className="min-w-0 flex-1 truncate">{item.alt || "video"}</span>
    <button type="button" onClick={media.allow} className="rounded-md border border-border bg-card px-2 py-0.5 text-caption font-medium text-foreground shadow-xs transition-colors hover:bg-accent active:scale-[0.98]">
      Load from {hostOf(item.url)}
    </button>
  </MediaNotice>;
  if (media.phase === "failed") return <MediaNotice icon={ImageOff}>
    <span className="min-w-0 flex-1 truncate">Couldn’t play this video</span>
    <button type="button" onClick={media.retry} className={OPEN_LINK}>Retry</button>
    <OpenOriginal url={item.url} what="original" />
  </MediaNotice>;
  const ready = media.phase === "ready";
  return <div className={cn(FRAME, !ready && "min-h-40")}>
    {!ready && <Skeleton className="absolute inset-0 h-full w-full rounded-none" />}
    <video
      // A new source is a new element: WebKit does not always re-probe a
      // `src` swapped under a video that has already errored.
      key={media.source}
      src={media.source}
      controls
      playsInline
      preload="metadata"
      aria-label={item.alt || "Video"}
      onLoadedMetadata={media.onLoad}
      onError={media.onError}
      className={cn("block max-h-[32rem] w-full transition-opacity duration-300 motion-reduce:transition-none", ready ? "opacity-100" : "opacity-0")}
    />
  </div>;
}

const COLUMNS = ["grid-cols-1", "grid-cols-1", "grid-cols-2", "grid-cols-3"] as const;

/** One row of media: a lone screenshot at full width, a before/after pair
 *  side by side. */
export function MediaRow({ items }: { items: GithubMediaItem[] }) {
  return <div className={cn("my-2 grid items-start gap-2", COLUMNS[Math.min(items.length, 3)])}>
    {items.map((item, index) => <figure key={`${index}-${item.url}`} className="min-w-0">
      {item.kind === "video" ? <GithubVideo item={item} /> : <GithubImage item={item} />}
      {item.caption && <figcaption className="mt-1 truncate text-center text-caption text-muted-foreground">{item.caption}</figcaption>}
    </figure>)}
  </div>;
}
