import { useEffect, useState } from "react";
import { clsx } from "clsx";
import { PaneState } from "@/components/ui/pane";
import { Kbd } from "@/components/ui/kbd";
import { formatShortcut } from "@/lib/platform";
import { ArrowUpRight, Check, ChevronLeft, ChevronRight, Clock3, Hand, ListTree, LoaderCircle, Pin, Square, TerminalSquare, X, type LucideIcon } from "lucide-react";
import type { AgentEvent, BridgeEvent, Session, WorkerRuntimeRecord } from "../types";
import type { QueuedWorkerRequest } from "../protocol/generated/protocol";
import { cn } from "@/lib/utils";
import { workerStatus, type WorkerTone } from "./workerStatus";
import type { TerminalActivity } from "./TerminalPane";
import { workerClock } from "./WorkerControls";
import { AgentChat } from "./AgentChat";
import { agentName, agentRole, agentTopic } from "./agentIdentity";
import { formatElapsed, harnessLabel, modelLabel } from "../utils";
import { HarnessMark } from "./harnessMarks";
import { dockPaneChord } from "../dockLayout";
import { WORKER_STATE_TEXT, formatWorkerClock, sentenceCase, stateOfTone, useTickingNow, type WorkerState } from "./workerFanOut";

// The Agents pane: the agents this chat has running right now, and nothing
// else, drawn as the agent tree they form: the orchestrator on top, each
// worker indented under whoever started it. A row opens that agent's chat in
// the pane, drawn the way every Bridge chat is, so you can watch it without
// the orchestrator's chat being covered. The dock opens here on its own when
// the orchestrator starts an agent, with that agent's chat showing. A pinned
// agent stays listed after it finishes, until you unpin it. Finished, failed
// and cancelled workers are not listed: the chat already carries their
// results, and its fan-out card still counts them.
//
// This is the agent tree only. The conversation tree (forks, branches) and the
// workspace tree (which worktree a worker writes in) are facts on a row, never
// what decides where the row sits.

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

export type LineageRow = AgentRow & { depth: number };

/// The listed agents as a tree under the orchestrator, depth 1 for the workers
/// it started. Each row hangs from its nearest listed ancestor, so a worker's
/// own workers sit one step further in, and a row whose parent is not listed
/// (finished and unpinned) moves up to the nearest one that is. Siblings keep
/// the list's order: pinned first, then by start.
export function agentLineage(chatId: string, rows: readonly AgentRow[], sessions: readonly Session[]): LineageRow[] {
  const listed = new Set(rows.map(row => row.session.id));
  const byId = new Map(sessions.map(session => [session.id, session]));
  const children = new Map<string, AgentRow[]>();
  for (const row of rows) {
    const seen = new Set<string>();
    let parent = row.session.parentSessionId ?? undefined;
    while (parent && parent !== chatId && !listed.has(parent) && !seen.has(parent)) {
      seen.add(parent);
      parent = byId.get(parent)?.parentSessionId ?? undefined;
    }
    const key = parent && listed.has(parent) ? parent : chatId;
    children.set(key, [...(children.get(key) ?? []), row]);
  }
  const ordered: LineageRow[] = [];
  const placed = new Set<string>();
  const visit = (id: string, depth: number) => {
    for (const row of children.get(id) ?? []) {
      if (placed.has(row.session.id)) continue;
      placed.add(row.session.id);
      ordered.push({ ...row, depth });
      visit(row.session.id, depth + 1);
    }
  };
  visit(chatId, 1);
  // Parent links that loop never reach the chat; the rows still show.
  for (const row of rows) if (!placed.has(row.session.id)) ordered.push({ ...row, depth: 1 });
  return ordered;
}

/// A worktree as a row can show it: from `.worktrees/` on when it lives in
/// one, else its last segment. The full path is the row's tooltip.
export function worktreeLabel(path: string | null | undefined): string | undefined {
  if (!path) return undefined;
  const at = path.indexOf(".worktrees/");
  if (at >= 0) return path.slice(at);
  return path.split("/").filter(Boolean).at(-1);
}

const STATE_ICON: Record<WorkerState, LucideIcon> = {
  working: LoaderCircle,
  "needs-you": Hand,
  done: Check,
  failed: X,
  stopped: Square,
};

/** Indent by depth: the orchestrator, then one step per generation. */
const DEPTH_INDENT = ["pl-3", "pl-8", "pl-13", "pl-18"];

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

/// A request to show one agent's chat, or the lineage when `sessionId` is
/// null. The nonce tells "show it again" apart from a re-render, so a second
/// spawn or click reopens it after Back.
export type AgentFocus = { sessionId: string | null; nonce: number };

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
  const [selected, setSelected] = useState<string | undefined>(() => focus?.sessionId ?? undefined);
  useEffect(() => {
    if (focus) setSelected(focus.sessionId ?? undefined);
  }, [focus]);
  const rows = activeAgentRows(chatSessionId, sessions, runtimes, pinned);
  const mine = descendantIds(chatSessionId, sessions);
  const now = useTickingNow(rows.some(row => isActiveTone(workerStatus(row.session, row.runtime).tone)));

  // An open agent stays open after it finishes, listed or not: it is what the
  // person is reading. Only an id that is not this chat's is ignored.
  const open = selected && mine.has(selected) ? sessions.find(item => item.id === selected) : undefined;
  if (open) {
    const runtime = runtimes.find(item => item.sessionId === open.id);
    const status = workerStatus(open, runtime);
    const name = agentName(open.id);
    const topic = agentTopic(open);
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
            <b className="shrink-0 text-ui font-medium text-foreground">{name}</b>
            <span className="min-w-0 truncate text-meta text-muted-foreground">{agentRole(open.label)}</span>
            <span className={cn("shrink-0 text-caption font-semibold", status.tone === "waiting" ? "text-warning" : "text-muted-foreground")}>{status.label}</span>
          </span>
          {topic && <span className="mt-0.5 block truncate text-meta text-foreground/80" title={topic}>{topic}</span>}
          <small className="mt-0.5 block truncate font-mono text-caption text-muted-foreground">{[harnessLabel(open.harness), open.model, runtime?.taskFamily].filter(Boolean).join(" · ")}</small>
        </span>
        <span className="shrink-0 font-mono text-caption tabular-nums text-muted-foreground">{formatElapsed(open.startedAt, workerClock(open, runtime, now))}</span>
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
  const empty = rows.length === 0 && queued.length === 0 && shellCount === 0;
  const lineage = agentLineage(chatSessionId, rows, sessions);
  const tones = new Map(rows.map(row => [row.session.id, workerStatus(row.session, row.runtime)]));
  const running = rows.filter(row => tones.get(row.session.id)?.tone === "working").length;
  // What the orchestrator itself is waiting on: the live workers it started.
  const awaited = lineage.filter(row => row.depth === 1 && isActiveTone(tones.get(row.session.id)?.tone ?? "idle"));
  const awaitedSince = Math.min(...awaited.flatMap(row => {
    const at = Date.parse(row.session.startedAt ?? "");
    return Number.isFinite(at) ? [at] : [];
  }));
  const orchestrator = sessions.find(item => item.id === chatSessionId);
  const headline = [
    ...(rows.length > 0 ? [`${rows.length} ${rows.length === 1 ? "worker" : "workers"}`, `${running} running`] : []),
    ...(queued.length > 0 ? [`${queued.length} queued`] : []),
  ].join(" · ");
  const paneDigit = dockPaneChord("tasks");

  return <div className="flex h-full flex-col">
    <div className="min-h-0 flex-1 overflow-y-auto">
      {empty && <PaneState icon={ListTree} title="No agents running">Workers this chat starts show up here while they run.</PaneState>}

      {(lineage.length > 0 || queued.length > 0) && <div className="px-4 pt-4">
        <div className="text-meta font-medium text-muted-foreground">Lineage</div>
        <h2 className="my-1 font-heading text-heading text-foreground">{headline}</h2>
        <p className="mb-3 text-caption text-muted-foreground">The agent tree for this chat. Open a worker to watch its chat.</p>
        <div data-lineage className="overflow-hidden rounded-lg border border-border bg-card py-1">
          <div data-lineage-orchestrator className="flex min-h-12 items-center gap-2.5 py-1.5 pl-3 pr-3">
            <HarnessMark harness={orchestrator?.harness} size={14}/>
            <span className="min-w-0 flex-1">
              <span className="block truncate text-meta text-foreground">Orchestrator</span>
              <span className="block truncate text-caption text-faint">{[orchestrator?.model ? modelLabel(orchestrator.model) : null, "this chat"].filter(Boolean).join(" · ")}</span>
            </span>
            {awaited.length > 0 && <span className="shrink-0 text-right">
              <span className="block text-caption text-info">Waiting on {awaited.length}</span>
              {Number.isFinite(awaitedSince) && <span className="block text-caption tabular-nums text-faint">{formatWorkerClock(now - awaitedSince)}</span>}
            </span>}
          </div>

          {lineage.map(({ runtime, session, pinned: isPinned, depth }) => {
            const status = tones.get(session.id) ?? workerStatus(session, runtime);
            const state = stateOfTone(status.tone);
            const StateIcon = STATE_ICON[state];
            const name = agentName(session.id);
            const topic = agentTopic(session);
            const role = agentRole(session.label);
            const worktree = worktreeLabel(runtime.worktreePath);
            const meta = [topic ? role : null, worktree, session.model ? modelLabel(session.model) : null].filter(Boolean).join(" · ");
            const detail = runtime.waitingReason ? `waiting: ${runtime.waitingReason.replaceAll("_", " ")}` : runtime.progressSummary;
            const files = status.tone === "done" && runtime.resultStatus === "reported" && Array.isArray(runtime.lastResult?.filesChanged) ? runtime.lastResult.filesChanged.length : 0;
            const label = files > 0 ? `Done · ${files} ${files === 1 ? "file" : "files"}` : sentenceCase(status.label);
            const started = Date.parse(session.startedAt ?? "");
            const stoppable = onStopWorker && !TERMINAL_LIFECYCLES.includes(runtime.lifecycleState);
            return <section key={runtime.sessionId} data-agent-row={runtime.sessionId} data-depth={depth} className="group/agent">
              <div className={clsx("flex min-h-12 items-center gap-1 py-1.5 pr-1.5 transition-colors hover:bg-accent/50", DEPTH_INDENT[Math.min(depth, DEPTH_INDENT.length - 1)])}>
                <button
                  type="button"
                  onClick={() => setSelected(runtime.sessionId)}
                  aria-label={`View ${name}`}
                  className="flex min-w-0 flex-1 items-center gap-2.5 rounded-md text-left"
                >
                  <span className="-ml-4 h-6 w-3 shrink-0 rounded-bl-md border-b border-l border-border" aria-hidden="true"/>
                  <HarnessMark harness={session.harness} size={14}/>
                  <span className="min-w-0 flex-1">
                    <span className="flex min-w-0 items-baseline gap-1.5">
                      <b className="shrink-0 text-meta font-medium text-foreground">{name}</b>
                      <span className="min-w-0 truncate text-meta text-foreground/80">{topic ?? role}</span>
                    </span>
                    {meta && <span className="block truncate text-caption text-faint" title={runtime.worktreePath ?? undefined}>{meta}</span>}
                    {detail && <span className="block truncate font-mono text-caption text-muted-foreground">{detail}</span>}
                  </span>
                  <span className="shrink-0 text-right">
                    <span data-agent-state={state} className={clsx("inline-flex items-center gap-1 text-caption", WORKER_STATE_TEXT[state])}>
                      <StateIcon size={11} className={clsx("shrink-0", state === "working" && "animate-spin")} aria-hidden="true"/>{label}
                    </span>
                    {Number.isFinite(started) && <span className="block text-caption tabular-nums text-faint">{formatWorkerClock(workerClock(session, runtime, now) - started)}</span>}
                  </span>
                </button>
                <PinButton name={name} pinned={isPinned} onToggle={() => onTogglePin(runtime.sessionId)} quiet />
                {stoppable && <StopButton name={name} onStop={() => void onStopWorker(runtime.sessionId)} quiet />}
              </div>
            </section>;
          })}

          {queued.map(item => <div key={item.id} data-task-row={item.id} className="flex min-h-12 items-center gap-2.5 py-1.5 pl-8 pr-3">
            <span className="-ml-4 h-6 w-3 shrink-0 rounded-bl-md border-b border-l border-border" aria-hidden="true"/>
            <Clock3 size={14} className="shrink-0 text-muted-foreground" aria-hidden="true" />
            <span className="min-w-0 flex-1">
              <span className="block truncate text-meta text-foreground">{queueObjective(item.request) ?? "Queued delegation"}</span>
              <span className="block truncate text-caption text-faint">{queueReason(item.request) ?? item.lastError ?? "waiting for capacity"} · {modelLabel(item.actualModel)}</span>
            </span>
            <span className="shrink-0 text-caption text-muted-foreground">Queued</span>
          </div>)}
        </div>
        {lineage.length > 0 && <div className="mt-3 flex items-center gap-2 text-caption text-faint">
          {paneDigit && <><Kbd>{formatShortcut({ meta: true, alt: true }, paneDigit)}</Kbd><span>this pane</span><span aria-hidden="true">·</span></>}
          <Kbd>↵</Kbd><span>open worker</span>
        </div>}
      </div>}

      {shellCount > 0 && <div className="px-4 pt-3">
        <button
          type="button"
          onClick={() => onOpenTerminal?.()}
          aria-label="Reveal shells in the terminal pane"
          className="flex w-full items-center gap-2.5 rounded-lg border border-border bg-card px-3 py-2.5 text-left transition-colors hover:bg-accent"
        >
          <TerminalSquare size={12} className="shrink-0 text-muted-foreground" aria-hidden="true" />
          <span className="min-w-0 flex-1 truncate text-meta text-foreground">{shellCount} shell{shellCount === 1 ? "" : "s"} running</span>
          <ChevronRight size={12} className="shrink-0 text-muted-foreground" aria-hidden="true" />
        </button>
      </div>}
    </div>
  </div>;
}

/// Lineage row actions surface on hover or keyboard focus, so a tree of rows
/// reads as names and states first. A pinned agent keeps its pin visible.
const QUIET_ACTION = "opacity-0 transition-opacity group-hover/agent:opacity-100 focus-visible:opacity-100";

function PinButton({ name, pinned, onToggle, quiet = false }: { name: string; pinned: boolean; onToggle: () => void; quiet?: boolean }) {
  return <button
    type="button"
    onClick={onToggle}
    aria-pressed={pinned}
    aria-label={`${pinned ? "Unpin" : "Pin"} ${name}`}
    title={pinned ? "Unpin: it leaves this list when it finishes" : "Pin: keep it listed here, even after it finishes"}
    className={cn("grid h-7 w-7 shrink-0 place-items-center rounded-md hover:bg-accent", pinned ? "text-foreground" : "text-muted-foreground hover:text-foreground", quiet && !pinned && QUIET_ACTION)}
  ><Pin size={12} className={cn(pinned && "fill-current")} aria-hidden="true" /></button>;
}

function StopButton({ name, onStop, quiet = false }: { name: string; onStop: () => void; quiet?: boolean }) {
  return <button
    type="button"
    onClick={onStop}
    aria-label={`Stop worker ${name}`}
    title="Stop this worker"
    className={cn("grid h-7 w-7 shrink-0 place-items-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground", quiet && QUIET_ACTION)}
  ><Square size={10} strokeWidth={1.8} aria-hidden="true" /></button>;
}
