import { useEffect, useState } from "react";
import { PaneState } from "@/components/ui/pane";
import { ArrowUpRight, ChevronLeft, ChevronRight, Clock3, ListTree, Pin, Square, TerminalSquare } from "lucide-react";
import type { AgentEvent, BridgeEvent, Session, WorkerRuntimeRecord } from "../types";
import type { QueuedWorkerRequest } from "../protocol/generated/protocol";
import { cn } from "@/lib/utils";
import { workerStatus, type WorkerTone } from "./workerStatus";
import type { TerminalActivity } from "./TerminalPane";
import { workerClock } from "./WorkerControls";
import { AgentChat } from "./AgentChat";
import { formatElapsed, harnessLabel } from "../utils";

// The Agents pane: the agents this chat has running right now, and nothing
// else. A row opens that agent's chat in the pane, drawn the way every Bridge
// chat is, so you can watch it without the orchestrator's chat being covered.
// The dock opens here on its own when the orchestrator starts an agent, with
// that agent's chat showing. A pinned agent stays listed after it finishes,
// until you unpin it. Finished, failed and cancelled workers are not listed:
// the chat already carries their results.

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

/// A request to show one agent's chat. The nonce tells "show it again" apart
/// from a re-render, so a second spawn or click reopens it after Back.
export type AgentFocus = { sessionId: string; nonce: number };

export function TasksPane({ chatSessionId, sessions, runtimes = [], queue = [], terminalActivity, pinned, onTogglePin, focus, liveEvents, reasons, onOpenSession, onSteer, onStopWorker, onOpenTerminal }: {
  chatSessionId: string;
  sessions: Session[];
  runtimes?: WorkerRuntimeRecord[];
  queue?: QueuedWorkerRequest[];
  terminalActivity?: TerminalActivity;
  pinned: ReadonlySet<string>;
  onTogglePin: (sessionId: string) => void;
  focus?: AgentFocus;
  /** The global live stream; each worker's frames arrive under its own id. */
  liveEvents: AgentEvent[];
  reasons?: BridgeEvent[];
  onOpenSession: (sessionId: string) => void;
  onSteer?: (sessionId: string, text: string) => Promise<void>;
  onStopWorker?: (sessionId: string) => Promise<void>;
  onOpenTerminal?: () => void;
}) {
  const [selected, setSelected] = useState<string | undefined>(() => focus?.sessionId);
  useEffect(() => {
    if (focus) setSelected(focus.sessionId);
  }, [focus]);
  const rows = activeAgentRows(chatSessionId, sessions, runtimes, pinned);
  const mine = descendantIds(chatSessionId, sessions);
  const now = Date.now();

  // An open agent stays open after it finishes, listed or not: it is what the
  // person is reading. Only an id that is not this chat's is ignored.
  const open = selected && mine.has(selected) ? sessions.find(item => item.id === selected) : undefined;
  if (open) {
    const runtime = runtimes.find(item => item.sessionId === open.id);
    const status = workerStatus(open, runtime);
    const name = open.title || open.label;
    const isPinned = pinned.has(open.id);
    return <div className="flex h-full flex-col">
      <div className="flex min-h-11 shrink-0 items-center gap-1.5 border-b border-border px-1.5 py-1.5">
        <button
          type="button"
          onClick={() => setSelected(undefined)}
          aria-label="All agents"
          title="All agents"
          className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground"
        ><ChevronLeft size={14} aria-hidden="true" /></button>
        <span className={cn("h-1.5 w-1.5 shrink-0 rounded-full", TONE_DOT[status.tone], status.tone === "working" && "mission-live-accent")} aria-hidden="true" />
        <span className="min-w-0 flex-1">
          <span className="flex items-baseline gap-1.5">
            <b className="min-w-0 truncate text-[13px] font-medium text-foreground">{name}</b>
            <span className={cn("shrink-0 text-[11px] font-semibold tracking-[0.06em]", status.tone === "waiting" ? "text-warning" : "text-muted-foreground")}>{status.label}</span>
          </span>
          <small className="mt-0.5 block truncate font-mono text-[11px] text-muted-foreground">{[harnessLabel(open.harness), open.model, runtime?.taskFamily].filter(Boolean).join(" · ")}</small>
        </span>
        <span className="shrink-0 font-mono text-[11px] tabular-nums text-muted-foreground">{formatElapsed(open.startedAt, workerClock(open, runtime, now))}</span>
        <PinButton name={name} pinned={isPinned} onToggle={() => onTogglePin(open.id)} />
        {onStopWorker && runtime && !TERMINAL_LIFECYCLES.includes(runtime.lifecycleState) && <StopButton name={name} onStop={() => void onStopWorker(open.id)} />}
        <button
          type="button"
          onClick={() => onOpenSession(open.id)}
          aria-label="Open session"
          title="Open this agent as the conversation"
          className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground"
        ><ArrowUpRight size={13} aria-hidden="true" /></button>
      </div>
      <div className="min-h-0 flex-1">
        <AgentChat key={open.id} session={open} runtime={runtime} liveEvents={liveEvents} reasons={reasons} onOpenSession={onOpenSession} onSteer={onSteer} />
      </div>
    </div>;
  }

  const queued = queue.filter(item => item.queueStatus === "queued" && (item.parentSessionId === chatSessionId || mine.has(item.parentSessionId)));
  const shellCount = terminalActivity?.running ?? 0;
  const running = rows.filter(row => isActiveTone(workerStatus(row.session, row.runtime).tone)).length;
  const empty = rows.length === 0 && queued.length === 0 && shellCount === 0;

  return <div className="flex h-full flex-col">
    <div className="min-h-0 flex-1 overflow-y-auto">
      {empty && <PaneState icon={ListTree} title="No agents running">Workers this chat starts show up here while they run.</PaneState>}

      {rows.map(({ runtime, session, pinned: isPinned }) => {
        const status = workerStatus(session, runtime);
        const elapsed = formatElapsed(session.startedAt, workerClock(session, runtime, now));
        const name = session.title || session.label;
        return <section key={runtime.sessionId} data-agent-row={runtime.sessionId} className="border-b border-border">
          <div className="flex items-center gap-2 px-3 py-2.5">
            <button
              type="button"
              onClick={() => setSelected(runtime.sessionId)}
              aria-label={`View ${name}`}
              className="flex min-w-0 flex-1 items-center gap-2 text-left"
            >
              <span className={cn("h-1.5 w-1.5 shrink-0 rounded-full", TONE_DOT[status.tone], status.tone === "working" && "mission-live-accent")} aria-hidden="true" />
              <span className="min-w-0 flex-1">
                <span className="flex items-baseline gap-1.5">
                  <b className="min-w-0 truncate text-[13px] font-medium text-foreground">{name}</b>
                  <span className={cn("shrink-0 text-[11px] font-semibold tracking-[0.06em]", status.tone === "waiting" ? "text-warning" : "text-muted-foreground")}>{status.label}</span>
                </span>
                <small className="mt-0.5 block truncate font-mono text-[11px] text-muted-foreground">{runtime.waitingReason ? `waiting: ${runtime.waitingReason.replaceAll("_", " ")}` : runtime.progressSummary ?? runtime.taskFamily}</small>
              </span>
              <span className="shrink-0 font-mono text-[11px] tabular-nums text-muted-foreground">{elapsed}</span>
              <ChevronRight size={12} className="shrink-0 text-muted-foreground" aria-hidden="true" />
            </button>
            <PinButton name={name} pinned={isPinned} onToggle={() => onTogglePin(runtime.sessionId)} />
            {onStopWorker && !TERMINAL_LIFECYCLES.includes(runtime.lifecycleState) && <StopButton name={name} onStop={() => void onStopWorker(runtime.sessionId)} />}
          </div>
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

function PinButton({ name, pinned, onToggle }: { name: string; pinned: boolean; onToggle: () => void }) {
  return <button
    type="button"
    onClick={onToggle}
    aria-pressed={pinned}
    aria-label={`${pinned ? "Unpin" : "Pin"} ${name}`}
    title={pinned ? "Unpin: it leaves this list when it finishes" : "Pin: keep it listed here, even after it finishes"}
    className={cn("grid h-7 w-7 shrink-0 place-items-center rounded-md hover:bg-accent", pinned ? "text-foreground" : "text-muted-foreground hover:text-foreground")}
  ><Pin size={12} className={cn(pinned && "fill-current")} aria-hidden="true" /></button>;
}

function StopButton({ name, onStop }: { name: string; onStop: () => void }) {
  return <button
    type="button"
    onClick={onStop}
    aria-label={`Stop worker ${name}`}
    title="Stop this worker"
    className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground"
  ><Square size={10} strokeWidth={1.8} aria-hidden="true" /></button>;
}
