import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { ArrowRight, Bot, CircleDot, Hammer, LoaderCircle, Maximize2, Minimize2, Navigation, RefreshCw, User, X } from "lucide-react";
import { bridgeApi } from "../api";
import type { AgentEvent, BridgeEvent, Session, WorkerRuntimeRecord } from "../types";
import { WorkerDiagnostics, WorkerStopControl, workerClock } from "./WorkerControls";
import { readWireKind } from "../transcript/wire";
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
  if (readWireKind(event.kind).startsWith("tool.")) return <Hammer size={11} className="text-muted-foreground" aria-hidden="true" />;
  if (event.role === "user") return <User size={11} className="text-muted-foreground" aria-hidden="true" />;
  if (event.role === "assistant") return <Bot size={11} className="text-muted-foreground" aria-hidden="true" />;
  return <CircleDot size={11} className="text-muted-foreground" aria-hidden="true" />;
}

function feedLabel(event: AgentEvent): string | null {
  const title = (event.title ?? "").trim();
  const text = (event.text ?? "").trim();
  const kind = readWireKind(event.kind);
  if (kind === "tool.started") return title ? `Running ${title}` : text ? `Running ${text}` : "Running a tool";
  if (kind === "tool.completed") return title || text || "Tool finished";
  return text || title || null;
}

/** Full activity view for one worker: durable backfill merged with the live
 * stream, the runtime's lifecycle facts, and the final result envelope once
 * it exists. Rendered as an overlay inside Agent Fleet. */
export function WorkerDetail({
  session,
  runtime,
  liveEvents,
  now,
  fullscreen,
  onToggleFullscreen,
  onClose,
  onFocusSession,
  onSteer,
  initialEvents,
  reasons = [],
  onStopWorker,
  embedded = false,
}: {
  session: Session;
  runtime?: WorkerRuntimeRecord;
  liveEvents: AgentEvent[];
  /** Frozen clock for tests and for callers that already own a ticker; the view
   *  keeps its own second hand otherwise, so elapsed time actually moves. */
  now?: number;
  fullscreen?: boolean;
  onToggleFullscreen?: () => void;
  onClose: () => void;
  onFocusSession: (sessionId: string) => void;
  /** Send guidance into this worker. Omitted where steering is not offered. */
  onSteer?: (sessionId: string, text: string) => Promise<void>;
  /** Test seam: pre-loaded durable events, skipping the backfill fetch. */
  initialEvents?: AgentEvent[];
  reasons?: BridgeEvent[];
  onStopWorker?: (id: string) => Promise<void>;
  /** Rendered inside a row of the Agents pane rather than over the chat: no
   *  dialog semantics, no Escape handler, no focus grab, and a feed that
   *  scrolls within a capped height. The row it sits in owns the name,
   *  status, clock and Stop, so the header is not repeated. */
  embedded?: boolean;
}) {
  const [liveNow, setLiveNow] = useState(Date.now);
  useEffect(() => {
    if (now !== undefined) return;
    const timer = window.setInterval(() => setLiveNow(Date.now()), 1_000);
    return () => window.clearInterval(timer);
  }, [now]);
  const clock = workerClock(session, runtime, now ?? liveNow);
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
    if (embedded) return;
    closeRef.current?.focus();
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [onClose, embedded]);

  const status = workerStatus(session, runtime);
  const result = runtime?.lastResult;
  // Mirrors the backend gate (session_input::worker_steer_gate) so the composer
  // is not offered for a steer that would be refused.
  const steerable = runtime?.resultStatus !== "reported"
    && runtime?.lifecycleState !== "checkpointing"
    && (session.status === "working" || session.status === "waiting");
  return (
    <div
      className={embedded ? "flex min-h-0 flex-col" : "flex min-h-0 flex-1 flex-col bg-background animate-page-mount"}
      role={embedded ? "region" : "dialog"}
      aria-modal={embedded ? undefined : true}
      aria-label={`Worker ${session.title || session.label}`}
    >
      {!embedded && <div className="flex shrink-0 flex-wrap items-center gap-x-2.5 gap-y-1 border-b border-border px-4 py-3 sm:px-6">
        <button ref={closeRef} type="button" onClick={onClose} className="inline-flex h-7 w-7 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-foreground" aria-label="Back to Agent Fleet"><X size={14}/></button>
        <h1 className="m-0 min-w-0 truncate font-display text-sm font-semibold tracking-tight text-foreground">{session.title || session.label}</h1>
        <span className={cn("shrink-0 text-[11px] font-semibold tracking-[0.07em]", toneText[status.tone])}>{status.label}</span>
        <span className="flex-1" />
        <span className="hidden font-mono text-[11px] text-muted-foreground sm:inline">{runtime?.taskFamily}</span>
        {runtime?.retryCount ? <span className="inline-flex items-center gap-0.5 font-mono text-[11px] text-muted-foreground"><RefreshCw size={8} aria-hidden="true"/>retry {runtime.retryCount}</span> : null}
        <span className="font-mono text-[11px] text-muted-foreground">{formatElapsed(session.startedAt, clock)}</span>
        <WorkerStopControl session={session} runtime={runtime} onStop={onStopWorker} />
        {onToggleFullscreen && <Button type="button" variant="ghost" size="sm" className="text-muted-foreground" onClick={onToggleFullscreen} aria-label={fullscreen ? "Exit fullscreen" : "Fullscreen"}>{fullscreen ? <Minimize2 size={13}/> : <Maximize2 size={13}/>}</Button>}
        <Button type="button" variant="secondary" size="sm" onClick={() => onFocusSession(session.id)}>Open session <ArrowRight size={12}/></Button>
      </div>}

      <div className={cn("space-y-2 border-b border-border py-2", embedded ? "px-3" : "px-4 sm:px-6")}>
        <p className="flex items-center gap-2 text-xs text-muted-foreground">
          <span className="min-w-0 truncate">{session.harness} · {session.model ?? "Model not reported"}{embedded && runtime?.taskFamily ? ` · ${runtime.taskFamily}` : ""}</span>
          {embedded && <button type="button" onClick={() => onFocusSession(session.id)} className="ml-auto inline-flex shrink-0 items-center gap-1 rounded-md px-1.5 py-0.5 text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">Open session <ArrowRight size={11} aria-hidden="true"/></button>}
        </p>
        <WorkerDiagnostics reasons={reasons} sessionId={session.id} />
      </div>

      {(runtime?.progressSummary || runtime?.waitingReason) && (
        <div className={cn("flex shrink-0 flex-wrap items-center gap-x-3 gap-y-1 border-b border-border bg-card py-2 text-[11px]", embedded ? "px-3" : "px-4 sm:px-6")}>
          {runtime.progressSummary && <span className="min-w-0 truncate font-mono text-foreground/80">{runtime.progressSummary}</span>}
          {runtime.waitingReason && <span className="shrink-0 rounded-full border border-warning/30 bg-warning/10 px-2 py-0.5 text-[11px] font-medium text-warning">waiting: {runtime.waitingReason.replaceAll("_", " ")}{runtime.waitingSince ? ` · ${formatElapsed(runtime.waitingSince, clock)}` : ""}</span>}
        </div>
      )}

      <div
        ref={feedRef}
        onScroll={event => {
          const element = event.currentTarget;
          stickToBottom.current = element.scrollHeight - element.scrollTop - element.clientHeight < 40;
        }}
        className={embedded ? "max-h-[26rem] min-h-0 overflow-y-auto overscroll-contain px-3 py-2.5" : "min-h-0 flex-1 overflow-y-auto overscroll-contain px-4 py-3 sm:px-6"}
      >
        {failure && <p className="mb-2 rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-[11px] text-destructive">{failure}</p>}
        {loading && <p className="flex items-center gap-2 py-4 text-xs text-muted-foreground"><LoaderCircle className="animate-spin" size={13}/>Loading the worker's activity…</p>}
        {!loading && feed.length === 0 && <p className="py-8 text-center font-mono text-[11px] text-muted-foreground">No activity recorded yet.</p>}
        <ol className="m-0 list-none space-y-1.5 p-0">
          {feed.map(event => (
            <li key={eventKey(event)} className="flex items-start gap-2">
              <span className="mt-[3px] w-4 shrink-0 text-center"><FeedIcon event={event}/></span>
              <p className={cn("m-0 min-w-0 flex-1 whitespace-pre-wrap break-words font-mono text-[11px] leading-[1.55]", readWireKind(event.kind).startsWith("tool.") ? "text-muted-foreground" : "text-foreground/85")}>{feedLabel(event)}</p>
            </li>
          ))}
        </ol>
        {result && (
          <div className="mt-4 rounded-xl border border-border bg-card p-3.5">
            <p className="mb-1.5 text-[11px] font-semibold uppercase tracking-[0.12em] text-muted-foreground">Result envelope</p>
            {typeof result.summary === "string" && <p className="text-[11px] leading-relaxed text-foreground/85">{result.summary}</p>}
            <pre className="mt-2 max-h-56 overflow-auto rounded-lg bg-muted p-2.5 font-mono text-[11px] leading-[1.5] text-muted-foreground">{JSON.stringify(result, null, 2)}</pre>
          </div>
        )}
      </div>

      {onSteer && <SteerComposer sessionId={session.id} steerable={steerable} onSteer={onSteer} className={embedded ? "shrink-0 border-t border-border px-3 py-2.5" : undefined}/>}
    </div>
  );
}

/// Guidance into a running worker, from the surface where you can see it going
/// wrong.
///
/// Labelled as steering, not chatting: the worker still answers to the objective
/// its orchestrator gave it, and what you type amends that objective rather than
/// starting a conversation. Hidden once the worker has reported, because at that
/// point the result is final and offering the box would be a lie.
export function SteerComposer({ sessionId, steerable, onSteer, label = "Steer this worker…", className = "shrink-0 border-t border-border px-4 py-3 sm:px-6", trailing }: {
  sessionId: string;
  steerable: boolean;
  onSteer: (sessionId: string, text: string) => Promise<void>;
  label?: string;
  /** Container chrome. The overlay wants a full-width divider; the docked
   *  composer in a session pane is already inside its own gutter. */
  className?: string;
  /** Rendered beside the Steer button (and beside the finished-worker notice) —
   *  a worker view has no chat ComposerPill of its own, so anything that needs
   *  to live "next to send" for every session, usage health included, has to
   *  be threaded in here too. */
  trailing?: ReactNode;
}) {
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string>();
  if (!steerable) {
    return <div className={cn(className, "flex items-center gap-2")} role="status">
      <span className="flex-1 text-[11px] text-muted-foreground">This worker has finished. Its typed result is final — ask the orchestrator to delegate a follow-up.</span>
      {trailing}
    </div>;
  }
  const send = () => {
    const text = draft.trim();
    if (!text || busy) return;
    setBusy(true); setFailure(undefined);
    void onSteer(sessionId, text)
      .then(() => setDraft(""))
      .catch((cause: unknown) => setFailure(cause instanceof Error ? cause.message : String(cause)))
      .finally(() => setBusy(false));
  };
  return <form className={className} onSubmit={requested => { requested.preventDefault(); send(); }}>
    <div className="flex items-end gap-2">
      <textarea
        value={draft}
        onChange={changed => setDraft(changed.target.value)}
        onKeyDown={pressed => {
          if (pressed.key === "Enter" && !pressed.shiftKey) { pressed.preventDefault(); send(); }
        }}
        rows={1}
        placeholder={label}
        aria-label={label}
        className="min-h-[34px] max-h-32 flex-1 resize-none rounded-xl border border-border bg-card px-3 py-2 text-[12px] text-foreground outline-none placeholder:text-muted-foreground focus-visible:ring-1 focus-visible:ring-ring"
      />
      <Button type="submit" size="sm" disabled={busy || !draft.trim()}><Navigation size={12}/>{busy ? "Sending…" : "Steer"}</Button>
      {trailing}
    </div>
    <p className="mt-1.5 text-[11px] text-muted-foreground">Guidance is folded into the worker&rsquo;s objective and its orchestrator is told. It still reports a typed result.</p>
    {failure && <p className="mt-1.5 text-[11px] text-destructive">{failure}</p>}
  </form>;
}
