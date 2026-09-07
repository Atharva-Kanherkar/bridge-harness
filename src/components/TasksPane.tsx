import { PaneState } from "@/components/ui/pane";
import { ChevronRight, CircleAlert, Clock3, ListTree, RotateCcw, TerminalSquare, X } from "lucide-react";
import type { Session, WorkerRuntimeRecord } from "../types";
import type { QueuedWorkerRequest } from "../protocol/generated/protocol";
import { cn } from "@/lib/utils";
import { workerStatus, type WorkerTone } from "./workerStatus";
import type { TerminalActivity } from "./TerminalPane";

// The background-tasks pane: one list answering "what is Bridge doing right
// now, and what did it just finish?". The expensive failure mode it removes
// is specific — something finished or failed while you were looking
// elsewhere, and nothing said so. The pane displays and routes; the policy
// engine stays the only authority, so every action here is wiring that
// already exists somewhere deeper.

const TONE_DOT: Record<WorkerTone, string> = {
  working: "bg-success",
  waiting: "bg-warning",
  attention: "bg-warning",
  warm: "bg-info",
  done: "bg-muted-foreground/25",
  failed: "bg-destructive",
  stalled: "bg-destructive",
  idle: "bg-muted-foreground/25",
};

function elapsedLabel(iso?: string | null): string | undefined {
  if (!iso) return undefined;
  const started = new Date(iso).getTime();
  if (Number.isNaN(started)) return undefined;
  const seconds = Math.max(0, Math.floor((Date.now() - started) / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}

function queueReason(request: unknown): string | undefined {
  if (typeof request !== "object" || request === null) return undefined;
  const value = (request as Record<string, unknown>).reason;
  return typeof value === "string" ? value : undefined;
}

function queueObjective(request: unknown): string | undefined {
  if (typeof request !== "object" || request === null) return undefined;
  const value = (request as Record<string, unknown>).objective;
  return typeof value === "string" ? value : undefined;
}

export function TasksPane({ sessions, runtimes = [], queue = [], terminalActivity, acknowledged, onAcknowledge, onOpenSession, onExpandWorker, onRetryWorker, onOpenTerminal }: {
  sessions: Session[];
  runtimes?: WorkerRuntimeRecord[];
  queue?: QueuedWorkerRequest[];
  terminalActivity?: TerminalActivity;
  /** Failure rows the human has dismissed. The host owns the set — the badge
   *  it computes has to agree with the list. */
  acknowledged: ReadonlySet<string>;
  onAcknowledge: (sessionId: string) => void;
  onOpenSession?: (sessionId: string) => void;
  onExpandWorker?: (sessionId: string) => void;
  onRetryWorker?: (sessionId: string) => void;
  onOpenTerminal?: () => void;
}) {
  const workerRows = runtimes.flatMap(runtime => {
    const session = sessions.find(item => item.id === runtime.sessionId);
    if (!session) return [];
    const status = workerStatus(session, runtime);
    const broken = status.tone === "failed" || status.tone === "stalled";
    if (broken && acknowledged.has(runtime.sessionId)) return [];
    return [{ runtime, session, status, broken }];
  });
  const shellCount = terminalActivity?.running ?? 0;
  const empty = workerRows.length === 0 && queue.length === 0 && shellCount === 0;

  return <div className="flex h-full flex-col">
    <div className="min-h-0 flex-1 overflow-y-auto">
      {empty && <PaneState icon={ListTree} title="No background tasks">Nothing is running in the background for this session.</PaneState>}

      {workerRows.map(({ runtime, session, status, broken }) => {
        const elapsed = elapsedLabel(runtime.lastActivityAt ?? session.startedAt);
        const retries = Number(runtime.retryCount) || 0;
        return <div
          key={runtime.sessionId}
          data-task-row={runtime.sessionId}
          className={cn("group flex items-start gap-3 border-b border-border px-3 py-3 transition-colors hover:bg-accent", broken && "bg-destructive/5")}
        >
          <span className={cn("mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full", TONE_DOT[status.tone], status.tone === "working" && "mission-live-accent")} aria-hidden="true" />
          <button
            type="button"
            onClick={() => onOpenSession?.(runtime.sessionId)}
            aria-label={`Open worker ${session.label}`}
            className="min-w-0 flex-1 text-left"
          >
            <span className="flex items-baseline gap-1.5">
              <b className="min-w-0 truncate text-[13px] font-medium text-foreground">{session.label}</b>
              <span className={cn("shrink-0 text-[11px] font-semibold tracking-[0.06em]", broken ? "text-destructive" : status.tone === "waiting" || status.tone === "attention" ? "text-warning" : "text-muted-foreground")}>{status.label}</span>
            </span>
            <small className={cn("mt-0.5 block truncate text-[11px]", broken ? "text-destructive" : "text-muted-foreground")}>
              {runtime.taskFamily}
              {elapsed ? ` · ${elapsed}` : ""}
              {retries > 0 ? ` · ${retries} ${retries === 1 ? "retry" : "retries"}` : ""}
              {status.detail ? ` · ${status.detail}` : ""}
            </small>
          </button>
          {broken && onRetryWorker && <button
            type="button"
            onClick={() => onRetryWorker(runtime.sessionId)}
            aria-label={`Retry worker ${session.label}`}
            title="Retry through the policy path"
            className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground"
          ><RotateCcw size={11} strokeWidth={1.8} aria-hidden="true" /></button>}
          {broken && <button
            type="button"
            onClick={() => onAcknowledge(runtime.sessionId)}
            aria-label={`Dismiss failure of ${session.label}`}
            title="Acknowledge this failure"
            className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground"
          ><X size={11} strokeWidth={1.8} aria-hidden="true" /></button>}
          {onExpandWorker && <button
            type="button"
            onClick={() => onExpandWorker(runtime.sessionId)}
            aria-label={`Expand worker ${session.label}`}
            title="Expand the worker panel"
            className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground"
          ><ListTree size={12} aria-hidden="true" /></button>}
        </div>;
      })}

      {queue.map(item => <div key={item.id} data-task-row={item.id} className="flex items-start gap-2.5 border-b border-border px-2.5 py-2.5">
        <Clock3 size={12} className="mt-1 shrink-0 text-muted-foreground" aria-hidden="true" />
        <span className="min-w-0 flex-1">
          <span className="flex items-baseline gap-1.5">
            <b className="min-w-0 truncate text-[13px] font-medium text-foreground">{queueObjective(item.request) ?? "Queued delegation"}</b>
            <span className="shrink-0 text-[11px] font-semibold tracking-[0.06em] text-muted-foreground">{item.queueStatus.toUpperCase()}</span>
          </span>
          {/* An unexplained "pending" is the thing this pane exists to kill. */}
          <small className="mt-0.5 block truncate text-[11px] text-muted-foreground">
            {queueReason(item.request) ?? item.lastError ?? "waiting for capacity"}
            {` · ${item.actualModel}`}
          </small>
        </span>
      </div>)}

      {shellCount > 0 && <button
        type="button"
        onClick={() => onOpenTerminal?.()}
        aria-label="Reveal shells in the terminal pane"
        className="flex w-full items-center gap-2.5 border-b border-border px-2.5 py-2.5 text-left transition-colors hover:bg-accent"
      >
        <TerminalSquare size={12} className="shrink-0 text-muted-foreground" aria-hidden="true" />
        <span className="min-w-0 flex-1 truncate text-[12px] text-foreground">{shellCount} shell{shellCount === 1 ? "" : "s"} running</span>
        <ChevronRight size={12} className="shrink-0 text-muted-foreground" aria-hidden="true" />
      </button>}
    </div>

    <div className="flex min-h-8 shrink-0 flex-wrap items-center gap-2 border-t border-border px-2.5 font-mono text-[11px] text-muted-foreground">
      <span>{workerRows.filter(row => row.status.tone === "working").length + shellCount} running · {queue.length} queued</span>
      {workerRows.some(row => row.broken) && <span className="ml-auto flex items-center gap-1 text-destructive"><CircleAlert size={10} aria-hidden="true" /> needs you</span>}
    </div>
  </div>;
}
