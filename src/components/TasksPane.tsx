import { useState } from "react";
import { PaneState } from "@/components/ui/pane";
import { ChevronDown, ChevronRight, Clock3, ListTree, Pin, Square, TerminalSquare } from "lucide-react";
import type { AgentEvent, BridgeEvent, Session, WorkerRuntimeRecord } from "../types";
import type { QueuedWorkerRequest } from "../protocol/generated/protocol";
import { cn } from "@/lib/utils";
import { workerStatus, type WorkerTone } from "./workerStatus";
import type { TerminalActivity } from "./TerminalPane";
import { workerClock } from "./WorkerControls";
import { WorkerDetail } from "./WorkerDetail";
import { formatElapsed } from "../utils";

// The Agents pane: the agents this chat has running right now, and nothing
// else. Each one opens in place to its own transcript, the same feed the
// worker view has always shown, so you can watch it without the chat being
// covered. A pinned agent stays open and stays listed after it finishes, until
// you unpin it. Finished, failed and cancelled workers are not listed: the
// chat already carries their results.

const TONE_DOT: Record<WorkerTone, string> = {
  working: "bg-success",
  waiting: "bg-warning",
  attention: "bg-warning",
  warm: "bg-info",
  done: "bg-muted-foreground/40",
  failed: "bg-destructive",
  stalled: "bg-destructive",
  idle: "bg-muted-foreground/40",
};

/// Running, or waiting on you: the two states in which an agent is doing
/// something now. Starting, resuming and restored resolve to `working`.
export function isActiveTone(tone: WorkerTone): boolean {
  return tone === "working" || tone === "waiting";
}

/// Lifecycles with nothing left to stop.
const TERMINAL_LIFECYCLES = ["completed", "cancelled", "stopped"];

/// Every session under `rootId`, at any depth. The snapshot a chat reads holds
/// the whole workspace's workers, so this is what keeps another chat's workers
/// out of this one.
export function descendantIds(rootId: string, sessions: readonly Session[]): Set<string> {
  const children = new Map<string, string[]>();
  for (const session of sessions) {
    if (!session.parentSessionId) continue;
    children.set(session.parentSessionId, [...(children.get(session.parentSessionId) ?? []), session.id]);
  }
  const found = new Set<string>();
  const stack = [...(children.get(rootId) ?? [])];
  while (stack.length > 0) {
    const id = stack.pop()!;
    if (found.has(id)) continue;
    found.add(id);
    stack.push(...(children.get(id) ?? []));
  }
  return found;
}

export type AgentRow = { runtime: WorkerRuntimeRecord; session: Session; pinned: boolean };

/// The rows the pane lists: this chat's active workers, plus any it pinned.
export function activeAgentRows(chatId: string, sessions: readonly Session[], runtimes: readonly WorkerRuntimeRecord[], pinned: ReadonlySet<string>): AgentRow[] {
  const mine = descendantIds(chatId, sessions);
  const rows = runtimes.flatMap(runtime => {
    if (!mine.has(runtime.sessionId)) return [];
    const session = sessions.find(item => item.id === runtime.sessionId);
    if (!session) return [];
    const isPinned = pinned.has(runtime.sessionId);
    if (!isPinned && !isActiveTone(workerStatus(session, runtime).tone)) return [];
    return [{ runtime, session, pinned: isPinned }];
  });
  return rows.sort((left, right) => Number(right.pinned) - Number(left.pinned) || (left.session.startedAt ?? "").localeCompare(right.session.startedAt ?? ""));
}

function queueObjective(request: unknown): string | undefined {
  if (typeof request !== "object" || request === null) return undefined;
  const value = (request as Record<string, unknown>).objective;
  return typeof value === "string" ? value : undefined;
}

function queueReason(request: unknown): string | undefined {
  if (typeof request !== "object" || request === null) return undefined;
  const value = (request as Record<string, unknown>).reason;
  return typeof value === "string" ? value : undefined;
}

export function TasksPane({ chatSessionId, sessions, runtimes = [], queue = [], terminalActivity, pinned, onTogglePin, liveEvents, reasons, onOpenSession, onSteer, onStopWorker, onOpenTerminal }: {
  chatSessionId: string;
  sessions: Session[];
  runtimes?: WorkerRuntimeRecord[];
  queue?: QueuedWorkerRequest[];
  terminalActivity?: TerminalActivity;
  pinned: ReadonlySet<string>;
  onTogglePin: (sessionId: string) => void;
  /** The global live stream; each worker's frames arrive under its own id. */
  liveEvents: AgentEvent[];
  reasons?: BridgeEvent[];
  onOpenSession: (sessionId: string) => void;
  onSteer?: (sessionId: string, text: string) => Promise<void>;
  onStopWorker?: (sessionId: string) => Promise<void>;
  onOpenTerminal?: () => void;
}) {
  // A row's open state, once someone has chosen it. Unchosen, a pinned row is
  // open and every other row is closed.
  const [opened, setOpened] = useState<ReadonlyMap<string, boolean>>(() => new Map());
  const rows = activeAgentRows(chatSessionId, sessions, runtimes, pinned);
  const mine = descendantIds(chatSessionId, sessions);
  const queued = queue.filter(item => item.queueStatus === "queued" && (item.parentSessionId === chatSessionId || mine.has(item.parentSessionId)));
  const shellCount = terminalActivity?.running ?? 0;
  const running = rows.filter(row => isActiveTone(workerStatus(row.session, row.runtime).tone)).length;
  const empty = rows.length === 0 && queued.length === 0 && shellCount === 0;
  const now = Date.now();

  return <div className="flex h-full flex-col">
    <div className="min-h-0 flex-1 overflow-y-auto">
      {empty && <PaneState icon={ListTree} title="No agents running">Workers this chat starts show up here while they run.</PaneState>}

      {rows.map(({ runtime, session, pinned: isPinned }) => {
        const status = workerStatus(session, runtime);
        const open = opened.get(runtime.sessionId) ?? isPinned;
        const elapsed = formatElapsed(session.startedAt, workerClock(session, runtime, now));
        const name = session.title || session.label;
        const toggle = () => setOpened(previous => new Map(previous).set(runtime.sessionId, !open));
        return <section key={runtime.sessionId} data-agent-row={runtime.sessionId} className="border-b border-border">
          <div className="flex items-center gap-2 px-3 py-2.5">
            <button
              type="button"
              onClick={toggle}
              aria-expanded={open}
              aria-label={`${open ? "Collapse" : "Expand"} ${name}`}
              className="flex min-w-0 flex-1 items-center gap-2 text-left"
            >
              {open ? <ChevronDown size={12} className="shrink-0 text-muted-foreground" aria-hidden="true" /> : <ChevronRight size={12} className="shrink-0 text-muted-foreground" aria-hidden="true" />}
              <span className={cn("h-1.5 w-1.5 shrink-0 rounded-full", TONE_DOT[status.tone], status.tone === "working" && "mission-live-accent")} aria-hidden="true" />
              <span className="min-w-0 flex-1">
                <span className="flex items-baseline gap-1.5">
                  <b className="min-w-0 truncate text-[13px] font-medium text-foreground">{name}</b>
                  <span className={cn("shrink-0 text-[11px] font-semibold tracking-[0.06em]", status.tone === "waiting" ? "text-warning" : "text-muted-foreground")}>{status.label}</span>
                </span>
                {!open && <small className="mt-0.5 block truncate font-mono text-[11px] text-muted-foreground">{runtime.waitingReason ? `waiting: ${runtime.waitingReason.replaceAll("_", " ")}` : runtime.progressSummary ?? runtime.taskFamily}</small>}
              </span>
              <span className="shrink-0 font-mono text-[11px] tabular-nums text-muted-foreground">{elapsed}</span>
            </button>
            <button
              type="button"
              onClick={() => onTogglePin(runtime.sessionId)}
              aria-pressed={isPinned}
              aria-label={`${isPinned ? "Unpin" : "Pin"} ${name}`}
              title={isPinned ? "Unpin: it leaves this list when it finishes" : "Pin: keep it open here, even after it finishes"}
              className={cn("grid h-7 w-7 shrink-0 place-items-center rounded-md hover:bg-accent", isPinned ? "text-foreground" : "text-muted-foreground hover:text-foreground")}
            ><Pin size={12} className={cn(isPinned && "fill-current")} aria-hidden="true" /></button>
            {onStopWorker && !TERMINAL_LIFECYCLES.includes(runtime.lifecycleState) && <button
              type="button"
              onClick={() => void onStopWorker(runtime.sessionId)}
              aria-label={`Stop worker ${name}`}
              title="Stop this worker"
              className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground"
            ><Square size={10} strokeWidth={1.8} aria-hidden="true" /></button>}
          </div>
          {open && <div className="border-t border-border bg-card/40">
            <WorkerDetail
              embedded
              session={session}
              runtime={runtime}
              liveEvents={liveEvents}
              reasons={reasons}
              onClose={toggle}
              onFocusSession={onOpenSession}
              onSteer={onSteer}
              onStopWorker={onStopWorker}
            />
          </div>}
        </section>;
      })}

      {queued.map(item => <div key={item.id} data-task-row={item.id} className="flex items-start gap-2.5 border-b border-border px-3 py-2.5">
        <Clock3 size={12} className="mt-1 shrink-0 text-muted-foreground" aria-hidden="true" />
        <span className="min-w-0 flex-1">
          <span className="flex items-baseline gap-1.5">
            <b className="min-w-0 truncate text-[13px] font-medium text-foreground">{queueObjective(item.request) ?? "Queued delegation"}</b>
            <span className="shrink-0 text-[11px] font-semibold tracking-[0.06em] text-muted-foreground">QUEUED</span>
          </span>
          <small className="mt-0.5 block truncate text-[11px] text-muted-foreground">{queueReason(item.request) ?? item.lastError ?? "waiting for capacity"} · {item.actualModel}</small>
        </span>
      </div>)}

      {shellCount > 0 && <button
        type="button"
        onClick={() => onOpenTerminal?.()}
        aria-label="Reveal shells in the terminal pane"
        className="flex w-full items-center gap-2.5 border-b border-border px-3 py-2.5 text-left transition-colors hover:bg-accent"
      >
        <TerminalSquare size={12} className="shrink-0 text-muted-foreground" aria-hidden="true" />
        <span className="min-w-0 flex-1 truncate text-[12px] text-foreground">{shellCount} shell{shellCount === 1 ? "" : "s"} running</span>
        <ChevronRight size={12} className="shrink-0 text-muted-foreground" aria-hidden="true" />
      </button>}
    </div>

    {!empty && <div className="flex min-h-8 shrink-0 items-center gap-2 border-t border-border px-3 font-mono text-[11px] text-muted-foreground">
      <span>{running + shellCount} running{queued.length > 0 ? ` · ${queued.length} queued` : ""}</span>
    </div>}
  </div>;
}
