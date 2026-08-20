import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowRight, Bot, CircleDot, Hammer, LoaderCircle, Maximize2, Minimize2, RefreshCw, User, X } from "lucide-react";
import { bridgeApi } from "../api";
import type { AgentEvent, Session, WorkerRuntimeRecord } from "../types";
import { cn } from "@/lib/utils";
import { formatElapsed } from "../utils";
import { workerStatus, type WorkerTone } from "./workerStatus";
import { Button } from "@/components/ui/button";

// The feed is bounded the same way the global live stream is (ae01432f):
// a fixed item count and a byte budget, oldest dropped first, so a long
// worker run cannot grow this overlay without limit.
const MAX_FEED_ITEMS = 500;
const MAX_FEED_TEXT_BYTES = 500_000;
const BACKFILL_LIMIT = 1000;
const MAX_CACHED_WORKERS = 8;
const workerFeedCache = new Map<string, AgentEvent[]>();

const toneText: Record<WorkerTone, string> = {
  working: "text-success",
  waiting: "text-warning",
  attention: "text-warning",
  warm: "text-info",
  done: "text-muted-foreground",
  failed: "text-destructive",
  stalled: "text-destructive",
  idle: "text-muted-foreground",
};

function eventKey(event: AgentEvent): string {
  return event.sequence > 0 ? `seq:${event.sequence}` : `id:${event.id}`;
}

function projectFeedEvent(event: AgentEvent): AgentEvent {
  return { ...event, data: {}, providerMeta: {} };
}

function boundFeed(events: AgentEvent[]): AgentEvent[] {
  let kept = events.slice(-MAX_FEED_ITEMS);
  let bytes = 0;
  const encoder = new TextEncoder();
  for (let index = kept.length - 1; index >= 0; index -= 1) {
    bytes += encoder.encode(kept[index].text ?? "").byteLength + encoder.encode(kept[index].title ?? "").byteLength;
    if (bytes > MAX_FEED_TEXT_BYTES) {
      kept = kept.slice(index + 1);
      break;
    }
  }
  return kept;
}

function cachedFeed(sessionId: string): AgentEvent[] | undefined {
  const cached = workerFeedCache.get(sessionId);
  if (!cached) return undefined;
  workerFeedCache.delete(sessionId);
  workerFeedCache.set(sessionId, cached);
  return cached;
}

function cacheFeed(sessionId: string, events: AgentEvent[]) {
  workerFeedCache.delete(sessionId);
  workerFeedCache.set(sessionId, boundFeed(events.map(projectFeedEvent)));
  while (workerFeedCache.size > MAX_CACHED_WORKERS) {
    const oldest = workerFeedCache.keys().next().value;
    if (oldest === undefined) break;
    workerFeedCache.delete(oldest);
  }
}

function FeedIcon({ event }: { event: AgentEvent }) {
  if (event.kind.startsWith("tool.")) return <Hammer size={11} className="text-muted-foreground" aria-hidden="true" />;
  if (event.role === "user") return <User size={11} className="text-muted-foreground" aria-hidden="true" />;
  if (event.role === "assistant") return <Bot size={11} className="text-muted-foreground" aria-hidden="true" />;
  return <CircleDot size={11} className="text-muted-foreground/60" aria-hidden="true" />;
}

function feedLabel(event: AgentEvent): string | null {
  const title = (event.title ?? "").trim();
  const text = (event.text ?? "").trim();
  if (event.kind === "tool.started") return title ? `Running ${title}` : text ? `Running ${text}` : "Running a tool";
  if (event.kind === "tool.completed") return title || text || "Tool finished";
  return text || title || null;
}

/** Full activity view for one worker: durable backfill merged with the live
 * stream, the runtime's lifecycle facts, and the final result envelope once
 * it exists. Rendered as an overlay inside Mission Control. */
export function WorkerDetail({
  session,
  runtime,
  liveEvents,
  now,
  fullscreen,
  onToggleFullscreen,
  onClose,
  onFocusSession,
  initialEvents,
}: {
  session: Session;
  runtime?: WorkerRuntimeRecord;
  liveEvents: AgentEvent[];
  now: number;
  fullscreen?: boolean;
  onToggleFullscreen?: () => void;
  onClose: () => void;
  onFocusSession: (sessionId: string) => void;
  /** Test seam: pre-loaded durable events, skipping the backfill fetch. */
  initialEvents?: AgentEvent[];
}) {
  const seed = initialEvents ?? cachedFeed(session.id);
  const [backfill, setBackfill] = useState<AgentEvent[]>(() => (seed ?? []).map(projectFeedEvent));
  const [loading, setLoading] = useState(seed === undefined);
  const [failure, setFailure] = useState<string>();
  const feedRef = useRef<HTMLDivElement>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  const stickToBottom = useRef(true);

  useEffect(() => {
    if (initialEvents !== undefined) return;
    let cancelled = false;
    if (!workerFeedCache.has(session.id)) {
      setLoading(true);
      setBackfill([]);
    }
    (async () => {
      try {
        // Ask the store for one bounded tail window. Walking from sequence zero
        // would rescan an arbitrarily long worker history on every reopen.
        const page = await bridgeApi.replaySessionEvents(session.id, 0, BACKFILL_LIMIT, true);
        if (cancelled) return;
        const collected = boundFeed(page.map(projectFeedEvent));
        cacheFeed(session.id, collected);
        setBackfill(collected);
        setFailure(undefined);
      } catch (error) {
        if (!cancelled) setFailure(error instanceof Error ? error.message : String(error));
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => { cancelled = true; };
  }, [session.id, initialEvents]);

  const feed = useMemo(() => {
    const seen = new Set(backfill.map(eventKey));
    const merged = [...backfill];
    const replayHighWater = backfill.reduce((highest, event) => Math.max(highest, event.sequence), 0);
    for (const event of liveEvents) {
      if (
        event.sessionId !== session.id
        || seen.has(eventKey(event))
        || (event.sequence > 0 && event.sequence <= replayHighWater)
      ) continue;
      seen.add(eventKey(event));
      merged.push(projectFeedEvent(event));
    }
    return boundFeed(merged.filter(event => feedLabel(event)));
  }, [backfill, liveEvents, session.id]);

  useEffect(() => cacheFeed(session.id, feed), [feed, session.id]);

  useEffect(() => {
    const element = feedRef.current;
    if (element && stickToBottom.current) element.scrollTop = element.scrollHeight;
  }, [feed.length]);

  useEffect(() => {
    closeRef.current?.focus();
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [onClose]);

  const status = workerStatus(session, runtime);
  const result = runtime?.lastResult;
  return (
    <div className="flex min-h-0 flex-1 flex-col bg-background animate-page-mount" role="dialog" aria-modal="true" aria-label={`Worker ${session.title || session.label}`}>
      <div className="flex shrink-0 flex-wrap items-center gap-x-2.5 gap-y-1 border-b border-border px-4 py-3 sm:px-6">
        <button ref={closeRef} type="button" onClick={onClose} className="inline-flex h-7 w-7 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-foreground" aria-label="Back to Mission Control"><X size={14}/></button>
        <h1 className="m-0 min-w-0 truncate font-display text-sm font-semibold tracking-tight text-foreground">{session.title || session.label}</h1>
        <span className={cn("shrink-0 text-[8.5px] font-semibold tracking-[0.07em]", toneText[status.tone])}>{status.label}</span>
        <span className="flex-1" />
        <span className="hidden font-mono text-[9px] text-muted-foreground sm:inline">{runtime?.taskFamily}</span>
        {runtime?.retryCount ? <span className="inline-flex items-center gap-0.5 font-mono text-[9px] text-muted-foreground"><RefreshCw size={8} aria-hidden="true"/>retry {runtime.retryCount}</span> : null}
        <span className="font-mono text-[9px] text-muted-foreground">{formatElapsed(session.startedAt, now)}</span>
        {onToggleFullscreen && <Button type="button" variant="ghost" size="sm" className="text-muted-foreground" onClick={onToggleFullscreen} aria-label={fullscreen ? "Exit fullscreen" : "Fullscreen"}>{fullscreen ? <Minimize2 size={13}/> : <Maximize2 size={13}/>}</Button>}
        <Button type="button" variant="secondary" size="sm" onClick={() => onFocusSession(session.id)}>Open session <ArrowRight size={12}/></Button>
      </div>

      {(runtime?.progressSummary || runtime?.waitingReason) && (
        <div className="flex shrink-0 flex-wrap items-center gap-x-3 gap-y-1 border-b border-border bg-card px-4 py-2 text-[10.5px] sm:px-6">
          {runtime.progressSummary && <span className="min-w-0 truncate font-mono text-foreground/80">{runtime.progressSummary}</span>}
          {runtime.waitingReason && <span className="shrink-0 rounded-full border border-warning/30 bg-warning/10 px-2 py-0.5 text-[9px] font-medium text-warning">waiting: {runtime.waitingReason.replaceAll("_", " ")}{runtime.waitingSince ? ` · ${formatElapsed(runtime.waitingSince, now)}` : ""}</span>}
        </div>
      )}

      <div
        ref={feedRef}
        onScroll={event => {
          const element = event.currentTarget;
          stickToBottom.current = element.scrollHeight - element.scrollTop - element.clientHeight < 40;
        }}
        className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-4 py-3 sm:px-6"
      >
        {failure && <p className="mb-2 rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-[10.5px] text-destructive">{failure}</p>}
        {loading && <p className="flex items-center gap-2 py-4 text-xs text-muted-foreground"><LoaderCircle className="animate-spin" size={13}/>Loading the worker's activity…</p>}
        {!loading && feed.length === 0 && <p className="py-8 text-center font-mono text-[10px] text-muted-foreground/70">No activity recorded yet.</p>}
        <ol className="m-0 list-none space-y-1.5 p-0">
          {feed.map(event => (
            <li key={eventKey(event)} className="flex items-start gap-2">
              <span className="mt-[3px] w-4 shrink-0 text-center"><FeedIcon event={event}/></span>
              <p className={cn("m-0 min-w-0 flex-1 whitespace-pre-wrap break-words font-mono text-[10.5px] leading-[1.55]", event.kind.startsWith("tool.") ? "text-muted-foreground" : "text-foreground/85")}>{feedLabel(event)}</p>
            </li>
          ))}
        </ol>
        {result && (
          <div className="mt-4 rounded-xl border border-border bg-card p-3.5">
            <p className="mb-1.5 text-[9px] font-semibold uppercase tracking-[0.12em] text-muted-foreground/70">Result envelope</p>
            {typeof result.summary === "string" && <p className="text-[11px] leading-relaxed text-foreground/85">{result.summary}</p>}
            <pre className="mt-2 max-h-56 overflow-auto rounded-lg bg-muted p-2.5 font-mono text-[9.5px] leading-[1.5] text-muted-foreground">{JSON.stringify(result, null, 2)}</pre>
          </div>
        )}
      </div>
    </div>
  );
}
