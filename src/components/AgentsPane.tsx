import { useMemo, useState, type ReactNode } from "react";
import {
  ArrowLeft, Check, ChevronDown, ChevronRight, CircleAlert, Clock3, CornerDownRight, ExternalLink,
  FileCode2, FilePen, FileText, GitBranch, ListTree, Network, Pin, RotateCcw, Search, Send, Square,
  TerminalSquare, X,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { PaneState } from "@/components/ui/pane";
import { HarnessMark } from "./harnessMarks";
import { SteerComposer } from "./WorkerDetail";
import type { QueuedWorkerRequest } from "../protocol/generated/protocol";
import type { AgentAsk, AgentNode, AgentRun, AgentStep, AgentsCensus, AgentsScope } from "./agentsModel";
import { agentClock, agentTrail, censusLine, findAgent, runClock } from "./agentsModel";
import type { ApprovalDecision } from "../types";
import type { TerminalActivity } from "./TerminalPane";

// The Agents pane: the dock's home for everything this conversation started.
//
// It used to be a flat list of Bridge workers with a status word and no
// activity, remounted on every chat switch, with a failure you dismissed gone
// for good. Three things changed and the markup below is a consequence of them:
//
// 1. **Every subagent is an agent.** A Claude `Task`, a Codex collab agent and an
//    OpenCode `task` child arrive through `agentsModel` next to the workers, so
//    the only thing that differs is the `subagent` tag on the row.
// 2. **A row is one live line, and it can be opened.** Collapsed it carries the
//    current step — verb, mono target, diff counts — which is the fact the old
//    pane never had. Opened it carries the objective, the model, the branch,
//    the write scope, the last five steps and the cost.
// 3. **Nothing seen disappears.** A finished, cancelled or failed row dims and
//    stays. Acknowledging a failure is a visual change, never a removal.
//
// Nesting is an indent and a hairline, never a box, and the pane itself is
// presentational: every decision (status, tone, cost) was made in
// `agentsModel`, and every action leaves through a prop.

/** The dot colours, unchanged from the pane this grew out of. No new tone. */
const TONE_DOT = {
  working: "bg-success",
  waiting: "bg-warning",
  attention: "bg-warning",
  warm: "bg-info",
  done: "bg-muted-foreground/25",
  failed: "bg-destructive",
  stalled: "bg-destructive",
  idle: "bg-muted-foreground/25",
} as const;

const TONE_TEXT = {
  working: "text-muted-foreground",
  waiting: "text-warning",
  attention: "text-warning",
  warm: "text-info",
  done: "text-muted-foreground",
  failed: "text-destructive",
  stalled: "text-destructive",
  idle: "text-muted-foreground",
} as const;

const TONE_LABEL = {
  working: "WORKING",
  waiting: "NEEDS YOU",
  attention: "NEEDS YOU",
  warm: "WARM",
  done: "DONE",
  failed: "FAILED",
  stalled: "STALLED",
  idle: "IDLE",
} as const;

function Dot({ tone, live }: { tone: keyof typeof TONE_DOT; live?: boolean }) {
  return <span className={cn("h-1.5 w-1.5 shrink-0 rounded-full", TONE_DOT[tone], live && tone === "working" && "mission-live-accent")} aria-hidden="true" />;
}

function OriginTag({ source }: { source: AgentNode["source"] }) {
  return <span className="shrink-0 rounded border border-border px-1 font-mono text-[10px] leading-4 text-muted-foreground">{source === "worker" ? "worker" : "subagent"}</span>;
}

function IconButton({ children, label, active, onClick }: { children: ReactNode; label: string; active?: boolean; onClick?: () => void }) {
  return <button type="button" aria-label={label} title={label} onClick={onClick} className={cn("grid h-6 w-6 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground", active && "text-foreground")}>{children}</button>;
}

function stepGlyph(verb: string) {
  if (verb === "Edit") return FilePen;
  if (verb === "Read") return FileText;
  if (verb === "Run") return TerminalSquare;
  if (verb === "Grep") return Search;
  if (verb === "Task") return CornerDownRight;
  if (verb === "Result") return Check;
  return FileCode2;
}

/** One step, verb first, target in mono, diff counts in their own colours. */
export function AgentStepLine({ step }: { step: AgentStep }) {
  const Glyph = stepGlyph(step.verb);
  const live = step.state === "live";
  const additions = step.extra?.split(" ")[0];
  const deletions = step.extra?.split(" ")[1];
  return <div className="flex min-h-6 min-w-0 items-center gap-2 text-[12px]">
    <Glyph size={12} strokeWidth={1.7} className="shrink-0 text-muted-foreground" aria-hidden="true" />
    <span className={cn("shrink-0", live ? "text-foreground" : "text-muted-foreground")}>{step.verb}</span>
    <span className={cn("min-w-0 truncate font-mono text-[11.5px]", live ? "text-foreground" : "text-muted-foreground")}>{step.target}</span>
    {step.extra && <span className={cn("shrink-0 font-mono text-[11px]", additions?.startsWith("+") ? "text-success" : "text-muted-foreground")}>
      {additions?.startsWith("+") ? <>{additions} <span className="text-destructive">{deletions}</span></> : step.extra}
    </span>}
    <span className="ml-auto shrink-0">{step.state === "pending" ? null : step.state === "live" ? <Dot tone="working" live /> : step.state === "failed" ? <X size={11} className="text-destructive" aria-hidden="true" /> : <Check size={11} className="text-muted-foreground" aria-hidden="true" />}</span>
  </div>;
}

function Counters({ node }: { node: AgentNode }) {
  const { files, additions, deletions, toolCalls, contextPercent, costUsd } = node.counters;
  return <div className="mt-2.5 flex flex-wrap items-center gap-x-3 gap-y-1 border-t border-border pt-2 font-mono text-[10.5px] tabular-nums text-muted-foreground">
    <span>{files} {files === 1 ? "file" : "files"} · <span className="text-success">+{additions}</span> <span className="text-destructive">−{deletions}</span></span>
    <span>{toolCalls} tool {toolCalls === 1 ? "call" : "calls"}</span>
    {contextPercent !== undefined && <span>ctx {contextPercent}%</span>}
    {costUsd !== undefined && <span>${costUsd.toFixed(2)}</span>}
  </div>;
}

export interface AgentsPaneProps {
  runs: AgentRun[];
  /** A row opens when its id is here. Owned by the host so it persists. */
  expanded: ReadonlySet<string>;
  pinned: ReadonlySet<string>;
  scope: AgentsScope;
  onScope: (scope: AgentsScope) => void;
  onToggleExpanded: (id: string) => void;
  onTogglePinned: (id: string) => void;
  /** The row the chat asked to focus; it stays highlighted until another is. */
  focusedId?: string;
  /** The agent whose full transcript is open inside the pane, if any. */
  drillId?: string;
  /** `undefined` closes the drill-in. Never the chat overlay any more. */
  onDrillIn?: (id: string | undefined) => void;
  onSteer?: (id: string, text: string) => Promise<void>;
  onStopWorker?: (id: string) => void;
  onRetryWorker?: (id: string) => void;
  onOpenSession?: (sessionId: string) => void;
  /** The same resolver the transcript's approval card uses. */
  onResolveAsk?: (ask: AgentAsk, decision: ApprovalDecision) => Promise<unknown>;
  onAcknowledge?: (id: string) => void;
  queue?: QueuedWorkerRequest[];
  terminalActivity?: TerminalActivity;
  onOpenTerminal?: () => void;
  now: number;
  /** The chat each run belongs to, for the tray and the `in <chat>` caption. */
  chatTitle?: (rootSessionId: string) => string;
}

export function AgentsPane({
  runs, expanded, pinned, scope, onScope, onToggleExpanded, onTogglePinned, focusedId, drillId, onDrillIn,
  onSteer, onStopWorker, onRetryWorker, onOpenSession, onResolveAsk, onAcknowledge,
  queue = [], terminalActivity, onOpenTerminal, now, chatTitle,
}: AgentsPaneProps) {
  const shellCount = terminalActivity?.running ?? 0;
  const drill = drillId ? findAgent(runs, drillId) : undefined;
  const census = useMemo<AgentsCensus>(() => runs.reduce<AgentsCensus>((total, run) => ({
    running: total.running + run.census.running,
    needsYou: total.needsYou + run.census.needsYou,
    done: total.done + run.census.done,
    failed: total.failed + run.census.failed,
    queued: total.queued + run.census.queued,
    workers: total.workers + run.census.workers,
    subagents: total.subagents + run.census.subagents,
    costUsd: total.costUsd + run.census.costUsd,
  }), { running: 0, needsYou: 0, done: 0, failed: 0, queued: 0, workers: 0, subagents: 0, costUsd: 0 }), [runs]);
  const empty = runs.every(run => run.agents.length === 0) && queue.length === 0 && shellCount === 0;

  return <div className="flex h-full flex-col">
    <div className="flex min-h-10 shrink-0 items-center gap-2 border-b border-border px-3">
      <div className="u-segmented flex items-center rounded-md p-0.5 text-[11.5px]">
        <button type="button" onClick={() => onScope("this-chat")} data-active={scope === "this-chat"} className="u-segmented-item rounded px-2 py-0.5" aria-pressed={scope === "this-chat"}>This chat</button>
        <button type="button" onClick={() => onScope("all-chats")} data-active={scope === "all-chats"} className="u-segmented-item rounded px-2 py-0.5" aria-pressed={scope === "all-chats"}>All chats</button>
      </div>
      <span className="ml-auto" />
      <span className="font-mono text-[11px] tabular-nums text-muted-foreground">{censusLine(census)}</span>
    </div>

    {drill
      ? <AgentDrillIn node={drill} trail={agentTrail(runs, drill.id)} now={now} pinned={pinned.has(drill.id)} onBack={() => onDrillIn?.(undefined)} onTogglePinned={() => onTogglePinned(drill.id)} onOpenSession={onOpenSession} onSteer={onSteer} onStopWorker={onStopWorker} />
      : <div className="min-h-0 flex-1 overflow-y-auto pb-3">
        {empty && <PaneState icon={Network} title="No agents yet">Nothing has been delegated and no harness subagent has run in this chat.</PaneState>}

        {runs.map(run => <section key={run.rootSessionId} className="pb-1">
          <div className="flex min-w-0 items-center gap-2 px-3 pb-1 pt-3">
            <HarnessMark harness={run.harness} size={13} />
            <span className="min-w-0 truncate text-[12px] font-medium text-foreground">{run.title}</span>
            <span className="shrink-0 font-mono text-[10.5px] text-muted-foreground">orchestrator</span>
            <span className="ml-auto font-mono text-[11px] tabular-nums text-muted-foreground">{runClock(run, now)}</span>
          </div>
          <div className="flex flex-col gap-0.5 px-1.5">
            {run.agents.map(node => <AgentRow
              key={node.id}
              node={node}
              now={now}
              expanded={expanded}
              pinned={pinned}
              focused={focusedId === node.id}
              chatTitle={scope === "all-chats" ? chatTitle?.(run.rootSessionId) : undefined}
              onToggleExpanded={onToggleExpanded}
              onTogglePinned={onTogglePinned}
              onDrillIn={onDrillIn}
              onSteer={onSteer}
              onStopWorker={onStopWorker}
              onRetryWorker={onRetryWorker}
              onOpenSession={onOpenSession}
              onResolveAsk={onResolveAsk}
              onAcknowledge={onAcknowledge}
            />)}
          </div>
        </section>)}

        {queue.length > 0 && <>
          <div className="flex items-center gap-2 px-3 pb-1 pt-3 text-[11px] font-medium text-muted-foreground">Queued</div>
          {queue.map(item => <div key={item.id} data-task-row={item.id} className="flex items-start gap-2.5 px-4 py-1">
            <Clock3 size={12} className="mt-0.5 shrink-0 text-muted-foreground" aria-hidden="true" />
            <span className="min-w-0 flex-1">
              <span className="block truncate text-[12.5px] text-foreground">{queueObjective(item.request) ?? "Queued delegation"}</span>
              <small className="block truncate text-[11px] text-muted-foreground">
                {queueReason(item.request) ?? item.lastError ?? "waiting for capacity"}
                {` · ${item.actualModel}`}
              </small>
            </span>
          </div>)}
        </>}

        {shellCount > 0 && <button
          type="button"
          onClick={() => onOpenTerminal?.()}
          aria-label="Reveal shells in the terminal pane"
          className="flex w-full items-center gap-2.5 px-4 py-1.5 text-left transition-colors hover:bg-accent"
        >
          <TerminalSquare size={12} className="shrink-0 text-muted-foreground" aria-hidden="true" />
          <span className="min-w-0 flex-1 truncate text-[12px] text-foreground">{shellCount} shell{shellCount === 1 ? "" : "s"} running</span>
          <ChevronRight size={12} className="shrink-0 text-muted-foreground" aria-hidden="true" />
        </button>}
      </div>}

    <div className="flex min-h-8 shrink-0 flex-wrap items-center gap-2 border-t border-border px-3 font-mono text-[11px] text-muted-foreground">
      <span>{census.running + shellCount} running · {census.queued} queued{census.costUsd > 0 ? ` · $${census.costUsd.toFixed(2)} this run` : ""}</span>
      {census.needsYou > 0 && <span className="ml-auto flex items-center gap-1 text-warning"><CircleAlert size={10} aria-hidden="true" />{census.needsYou} needs you</span>}
    </div>
  </div>;
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

// ── the row ─────────────────────────────────────────────────────────────────

interface RowProps {
  node: AgentNode;
  depth?: number;
  now: number;
  expanded: ReadonlySet<string>;
  pinned: ReadonlySet<string>;
  focused?: boolean;
  chatTitle?: string;
  onToggleExpanded: (id: string) => void;
  onTogglePinned: (id: string) => void;
  onDrillIn?: (id: string | undefined) => void;
  onSteer?: (id: string, text: string) => Promise<void>;
  onStopWorker?: (id: string) => void;
  onRetryWorker?: (id: string) => void;
  onOpenSession?: (sessionId: string) => void;
  onResolveAsk?: (ask: AgentAsk, decision: ApprovalDecision) => Promise<unknown>;
  onAcknowledge?: (id: string) => void;
}

/// Guidance into one agent, from the row that shows whether it is going wrong.
function RowSteer({ node, onSteer }: { node: AgentNode; onSteer: (id: string, text: string) => Promise<void> }) {
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string>();
  const send = async () => {
    const text = draft.trim();
    if (!text || busy) return;
    setBusy(true);
    setFailure(undefined);
    try {
      await onSteer(node.id, text);
      setDraft("");
    } catch (cause) {
      setFailure(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  };
  return <form className="mt-1.5" onSubmit={requested => { requested.preventDefault(); void send(); }}>
    <div className="flex items-end gap-1.5">
      <textarea
        value={draft}
        onChange={changed => setDraft(changed.target.value)}
        onKeyDown={pressed => { if (pressed.key === "Enter" && !pressed.shiftKey) { pressed.preventDefault(); void send(); } }}
        rows={1}
        placeholder={`Steer ${node.name}. It reads it at its next step.`}
        aria-label={`Steer ${node.name}`}
        className="min-h-[30px] max-h-28 flex-1 resize-none rounded-lg border border-border bg-card px-2.5 py-1.5 text-[12px] text-foreground outline-none placeholder:text-muted-foreground focus-visible:ring-1 focus-visible:ring-ring"
      />
      <button type="submit" disabled={busy || !draft.trim()} className="inline-flex h-[30px] shrink-0 items-center gap-1 rounded-md bg-primary px-2.5 text-[12px] font-medium text-primary-foreground disabled:opacity-60"><Send size={11} aria-hidden="true" />Send</button>
    </div>
    {failure && <p role="alert" className="mt-1 text-[11px] text-destructive">{failure}</p>}
  </form>;
}

function RowActions({ node, pinned, onTogglePinned, onDrillIn, onSteer, onStopWorker, steerOpen, onToggleSteer }: RowProps & { steerOpen: boolean; onToggleSteer: () => void }) {
  return <div className="mt-2 flex flex-wrap items-center gap-1">
    {onDrillIn && <button type="button" onClick={() => onDrillIn(node.id)} className="inline-flex h-7 items-center gap-1.5 rounded-md border border-border px-2.5 text-[12px] text-foreground transition-colors hover:bg-accent"><ListTree size={12} aria-hidden="true" />Transcript</button>}
    {onSteer && <button type="button" onClick={onToggleSteer} aria-expanded={steerOpen} className="inline-flex h-7 items-center gap-1.5 rounded-md border border-border px-2.5 text-[12px] text-foreground transition-colors hover:bg-accent"><Send size={11} aria-hidden="true" />Steer</button>}
    {onStopWorker && <button type="button" onClick={() => onStopWorker(node.id)} className="inline-flex h-7 items-center gap-1.5 rounded-md px-2 text-[12px] text-muted-foreground transition-colors hover:bg-accent"><Square size={8} className="fill-current" aria-hidden="true" />Stop</button>}
    <span className="ml-auto" />
    <button type="button" onClick={() => onTogglePinned(node.id)} aria-pressed={!!pinned} className={cn("inline-flex h-7 items-center gap-1.5 rounded-md px-2 text-[12px] transition-colors hover:bg-accent", pinned ? "text-foreground" : "text-muted-foreground")}>
      <Pin size={11} className={cn(pinned && "fill-current")} aria-hidden="true" />{pinned ? "Pinned" : "Pin"}
    </button>
  </div>;
}

export function AgentRow(props: RowProps) {
  const { node, depth = 0, now, expanded, pinned, focused, chatTitle, onToggleExpanded, onTogglePinned, onDrillIn, onSteer, onStopWorker, onRetryWorker, onOpenSession, onResolveAsk, onAcknowledge } = props;
  const open = expanded.has(node.id);
  const isPinned = pinned.has(node.id);
  const asking = !!node.ask;
  const failed = !!node.failureCode;
  const quiet = node.status.tone === "done" || node.status.tone === "idle";
  const [askError, setAskError] = useState<string>();
  const [busy, setBusy] = useState<ApprovalDecision>();
  const [steerOpen, setSteerOpen] = useState(false);
  const ask = node.ask;

  const answer = async (decision: ApprovalDecision) => {
    if (!ask || !onResolveAsk) return;
    setBusy(decision);
    setAskError(undefined);
    try {
      await onResolveAsk(ask, decision);
    } catch (cause) {
      // A refused resolve is not a dismissal: the row stays in NEEDS YOU with
      // the reason on it, because the question is still unanswered.
      setAskError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(undefined);
    }
  };

  return <div className={cn("relative", depth > 0 && "ml-[15px] border-l border-border pl-2")} data-agent-row={node.id}>
    <div className={cn(
      "group rounded-lg px-2.5 py-2 transition-colors",
      focused ? "bg-selection" : open ? "bg-accent/60" : "hover:bg-accent",
      asking && "border-l-2 border-l-warning rounded-l-none",
      failed && "border-l-2 border-l-destructive rounded-l-none",
      node.acknowledged && "opacity-60",
    )}>
      <div className="flex min-w-0 items-center gap-2">
        <button
          type="button"
          onClick={() => onToggleExpanded(node.id)}
          aria-expanded={open}
          aria-label={`${open ? "Collapse" : "Expand"} ${node.name}`}
          className="grid h-4 w-3 shrink-0 place-items-center text-muted-foreground"
        >{open ? <ChevronDown size={12} aria-hidden="true" /> : <ChevronRight size={12} aria-hidden="true" />}</button>
        <Dot tone={node.status.tone} live />
        <HarnessMark harness={node.harness} size={13} />
        <b className={cn("min-w-0 truncate text-[13px] font-medium", quiet ? "text-muted-foreground" : "text-foreground")}>{node.name}</b>
        <OriginTag source={node.source} />
        {isPinned && <Pin size={11} className="shrink-0 fill-current text-muted-foreground" aria-label="Pinned" />}
        <span className="ml-auto flex shrink-0 items-center gap-1.5">
          {node.status.tone !== "working" && <span className={cn("text-[10.5px] font-semibold tracking-[0.06em]", TONE_TEXT[node.status.tone])}>{node.status.label || TONE_LABEL[node.status.tone]}</span>}
          <span className="font-mono text-[11px] tabular-nums text-muted-foreground">{agentClock(node, now)}</span>
        </span>
      </div>

      {chatTitle && <div className="mt-0.5 pl-[34px] text-[11px] text-muted-foreground">in {chatTitle}</div>}

      {!open && !asking && !failed && (node.liveLine
        ? <div className="mt-0.5 pl-[34px] pr-1"><AgentStepLine step={node.liveLine} /></div>
        : <div className="mt-0.5 pl-[34px] pr-1 text-[12px] text-muted-foreground">{node.liveLine ? "" : "No recorded activity yet."}</div>)}

      {asking && ask && <div className="mt-2 pl-[34px]">
        <p className="text-[12px] text-foreground">{ask.title}</p>
        {ask.detail && <p className="mt-0.5 text-[12px] leading-relaxed text-muted-foreground">{ask.detail}</p>}
        {ask.command && <div className="mt-1.5 rounded-md border border-border bg-code px-2 py-1.5 font-mono text-[11.5px] text-foreground">{ask.command}</div>}
        <p className="mt-1 font-mono text-[10.5px] text-muted-foreground">{[ask.cwd, ask.ownedPaths.length > 0 ? ask.ownedPaths.join(" ") : "no write scope"].filter(Boolean).join(" · ")}</p>
        <div className="mt-2 flex flex-wrap items-center gap-1.5">
          <button type="button" disabled={!!busy} onClick={() => void answer("accept")} className="inline-flex h-7 items-center gap-1 rounded-md bg-primary px-2.5 text-[12px] font-medium text-primary-foreground disabled:opacity-60">Approve</button>
          <button type="button" disabled={!!busy} onClick={() => void answer("acceptForSession")} className="inline-flex h-7 items-center rounded-md border border-border px-2.5 text-[12px] text-foreground transition-colors hover:bg-accent">Approve for this run</button>
          <button type="button" disabled={!!busy} onClick={() => void answer("decline")} className="inline-flex h-7 items-center rounded-md px-2 text-[12px] text-muted-foreground transition-colors hover:bg-accent">Deny</button>
        </div>
        {askError && <p role="alert" className="mt-1.5 text-[11.5px] text-destructive">{askError}</p>}
      </div>}

      {failed && <div className="mt-1.5 pl-[34px]">
        <p className="font-mono text-[11px] text-destructive">{node.failureCode}</p>
        {node.liveLine && <p className="mt-0.5 text-[12px] text-muted-foreground">Last step: {node.liveLine.verb} {node.liveLine.target}.</p>}
        <div className="mt-2 flex flex-wrap items-center gap-1.5">
          {onRetryWorker && <button type="button" onClick={() => onRetryWorker(node.id)} className="inline-flex h-7 items-center gap-1 rounded-md border border-border px-2.5 text-[12px] text-foreground transition-colors hover:bg-accent"><RotateCcw size={11} aria-hidden="true" />Retry</button>}
          {onDrillIn && <button type="button" onClick={() => onDrillIn(node.id)} className="inline-flex h-7 items-center rounded-md px-2 text-[12px] text-muted-foreground transition-colors hover:bg-accent">Open transcript</button>}
          {onAcknowledge && <button type="button" onClick={() => onAcknowledge(node.id)} className="inline-flex h-7 items-center rounded-md px-2 text-[12px] text-muted-foreground transition-colors hover:bg-accent">Acknowledge</button>}
        </div>
      </div>}

      {open && <div className="mt-2 pl-[34px] pr-1">
        {node.objective && <p className="text-[12px] leading-relaxed text-muted-foreground">{node.objective}</p>}
        <div className="mt-2 flex flex-wrap items-center gap-1.5 font-mono text-[10.5px] text-muted-foreground">
          {node.model && <span className="rounded border border-border px-1.5 py-0.5">{node.effort ? `${node.model} · ${node.effort}` : node.model}</span>}
          {node.branch && <span className="inline-flex items-center gap-1 rounded border border-border px-1.5 py-0.5"><GitBranch size={10} aria-hidden="true" />{node.branch}</span>}
          {node.scope.map(path => <span key={path} className="rounded border border-border px-1.5 py-0.5">{path}</span>)}
        </div>
        <div className="mt-2.5 flex flex-col">
          {node.steps.length === 0
            ? <p className="text-[12px] text-muted-foreground">No recorded steps yet.</p>
            : node.steps.map(step => <AgentStepLine key={step.id} step={step} />)}
        </div>
        <Counters node={node} />
        <RowActions
          node={node}
          depth={depth}
          now={now}
          expanded={expanded}
          pinned={pinned}
          steerOpen={steerOpen}
          onToggleSteer={() => setSteerOpen(value => !value)}
          onToggleExpanded={onToggleExpanded}
          onTogglePinned={onTogglePinned}
          onDrillIn={onDrillIn}
          onSteer={onSteer}
          onStopWorker={onStopWorker}
          onOpenSession={onOpenSession}
        />
        {steerOpen && onSteer && <RowSteer node={node} onSteer={onSteer} />}
        {onOpenSession && <div className="mt-1.5"><button type="button" onClick={() => onOpenSession(node.sessionId)} className="inline-flex h-6 items-center gap-1 rounded-md px-1 text-[11.5px] text-muted-foreground transition-colors hover:bg-accent">Open as a chat<ExternalLink size={11} aria-hidden="true" /></button></div>}
      </div>}
    </div>
    {node.children.length > 0 && <div className="mt-0.5 flex flex-col gap-0.5">{node.children.map(child => <AgentRow key={child.id} {...props} node={child} depth={depth + 1} focused={false} />)}</div>}
  </div>;
}

// ── drill-in ────────────────────────────────────────────────────────────────

const TABS = ["Activity", "Files", "Prompt", "Result"] as const;
type Tab = typeof TABS[number];

/**
 * Full observability of one agent, inside the pane.
 *
 * This used to be an `absolute inset-0 z-30` overlay over the whole chat
 * section: opening a worker's feed to decide something hid the conversation
 * that raised the question. Here the chat stays where it is, on the left, and
 * the dock shows the one agent in full. That is the whole reason the overlay is
 * gone rather than restyled.
 */
function AgentDrillIn({ node, trail, now, pinned, onBack, onTogglePinned, onOpenSession, onSteer, onStopWorker }: {
  node: AgentNode;
  trail: AgentNode[];
  now: number;
  pinned: boolean;
  onBack: () => void;
  onTogglePinned: () => void;
  onOpenSession?: (sessionId: string) => void;
  onSteer?: (id: string, text: string) => Promise<void>;
  onStopWorker?: (id: string) => void;
}) {
  const [tab, setTab] = useState<Tab>("Activity");
  const steerable = node.source === "worker" && !!onSteer && node.status.tone !== "done" && node.status.tone !== "failed";
  return <div className="flex min-h-0 flex-1 flex-col">
    <div className="flex min-h-10 shrink-0 items-center gap-2 border-b border-border px-2">
      <button type="button" onClick={onBack} className="inline-flex h-7 min-w-0 items-center gap-1 rounded-md px-1.5 text-[12px] text-muted-foreground transition-colors hover:bg-accent">
        <ArrowLeft size={13} aria-hidden="true" />Agents
      </button>
      {trail.slice(0, -1).map(parent => <span key={parent.id} className="flex min-w-0 items-center gap-1 text-[12px] text-muted-foreground"><ChevronRight size={11} aria-hidden="true" className="shrink-0" /><span className="max-w-[80px] truncate">{parent.name}</span></span>)}
      <span className="min-w-0 truncate text-[12.5px] font-medium text-foreground">{node.name}</span>
      <span className="ml-auto" />
      <IconButton label={pinned ? "Unpin" : "Pin"} active={pinned} onClick={onTogglePinned}><Pin size={12} className={cn(pinned && "fill-current")} /></IconButton>
      {onOpenSession && <IconButton label="Open as a chat" onClick={() => onOpenSession(node.sessionId)}><ExternalLink size={12} /></IconButton>}
    </div>

    <div className="shrink-0 border-b border-border px-3 py-2.5">
      <div className="flex items-center gap-2">
        <Dot tone={node.status.tone} live />
        <HarnessMark harness={node.harness} size={13} />
        <span className="min-w-0 truncate text-[12.5px] text-foreground">{node.model ?? node.harness}</span>
        <OriginTag source={node.source} />
        <span className="ml-auto font-mono text-[11px] tabular-nums text-muted-foreground">{agentClock(node, now)}</span>
      </div>
      <div className="mt-2 grid grid-cols-4 gap-2 font-mono text-[10.5px] tabular-nums text-muted-foreground">
        <span><b className="block text-[12px] font-medium text-foreground">{node.counters.toolCalls}</b>tool calls</span>
        <span><b className="block text-[12px] font-medium text-foreground"><span className="text-success">+{node.counters.additions}</span> <span className="text-destructive">−{node.counters.deletions}</span></b>{node.counters.files} files</span>
        <span><b className="block text-[12px] font-medium text-foreground">{node.counters.contextPercent === undefined ? "—" : `${node.counters.contextPercent}%`}</b>context</span>
        <span><b className="block text-[12px] font-medium text-foreground">{node.counters.costUsd === undefined ? "—" : `$${node.counters.costUsd.toFixed(2)}`}</b>so far</span>
      </div>
      <div className="mt-2.5 flex gap-1 text-[12px]">
        {TABS.map(name => <button
          key={name}
          type="button"
          onClick={() => setTab(name)}
          aria-pressed={tab === name}
          className={cn("rounded-md px-2 py-1 transition-colors", tab === name ? "bg-selection text-selection-foreground" : "text-muted-foreground hover:bg-accent")}
        >{name}</button>)}
      </div>
    </div>

    <div className="min-h-0 flex-1 overflow-y-auto px-3 py-3">
      {tab === "Activity" && <div className="flex flex-col rounded-lg border border-border px-2.5 py-1">
        {node.steps.length === 0 ? <p className="py-2 text-[12px] text-muted-foreground">No activity recorded yet.</p> : node.steps.map(step => <AgentStepLine key={step.id} step={step} />)}
      </div>}
      {tab === "Files" && <div className="flex flex-col gap-1.5 text-[12px] text-muted-foreground">
        {node.scope.length === 0 ? <p>No write scope was leased for this agent.</p> : node.scope.map(path => <span key={path} className="font-mono text-[11.5px]">{path}</span>)}
        {node.branch && <span className="mt-2 inline-flex items-center gap-1 font-mono text-[11.5px]"><GitBranch size={11} aria-hidden="true" />{node.branch}</span>}
      </div>}
      {tab === "Prompt" && <p className="whitespace-pre-wrap text-[12.5px] leading-relaxed text-foreground">{node.objective ?? "No objective was reported for this agent."}</p>}
      {tab === "Result" && <p className="whitespace-pre-wrap text-[12.5px] leading-relaxed text-foreground">
        {node.liveLine?.verb === "Result" ? node.liveLine.target : node.status.detail ?? "This agent has not reported a result."}
      </p>}
    </div>

    {steerable && onSteer && <SteerComposer
      sessionId={node.id}
      steerable
      onSteer={onSteer}
      label="Steer this agent. It reads it at its next step."
      className="shrink-0 border-t border-border p-2.5"
      trailing={onStopWorker ? <button type="button" onClick={() => onStopWorker(node.id)} className="inline-flex h-7 items-center gap-1 rounded-md px-2 text-[12px] text-muted-foreground transition-colors hover:bg-accent"><Square size={8} className="fill-current" aria-hidden="true" />Stop</button> : undefined}
    />}
  </div>;
}

// ── the pinned tray ─────────────────────────────────────────────────────────

/**
 * Pinned means sticky.
 *
 * A pinned agent follows you: another chat, another dock pane, and it is still
 * at the bottom of the dock with its live step, and an ask can be answered from
 * there without navigating. That is the whole reason pins are global rather than
 * per-chat — a pin scoped to the window you pinned it in would not be a pin.
 */
export function PinnedAgentsTray({ runs, pinned, now, pane, onOpenAgents, onResolveAsk, chatTitle }: {
  runs: AgentRun[];
  pinned: ReadonlySet<string>;
  now: number;
  /** The pane the dock is showing; the tray would be redundant on Agents itself. */
  pane: string;
  onOpenAgents: (id: string) => void;
  onResolveAsk?: (ask: AgentAsk, decision: ApprovalDecision) => Promise<unknown>;
  chatTitle?: (rootSessionId: string) => string;
}) {
  const [open, setOpen] = useState(true);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const entries = useMemo(() => runs.flatMap(run => run.agents.map(node => ({ node, run }))).filter(entry => pinned.has(entry.node.id)), [runs, pinned]);
  if (pane === "tasks" || entries.length === 0) return null;

  const answer = async (ask: AgentAsk, decision: ApprovalDecision) => {
    if (!onResolveAsk) return;
    try {
      await onResolveAsk(ask, decision);
      setErrors(current => { const next = { ...current }; delete next[ask.eventId]; return next; });
    } catch (cause) {
      setErrors(current => ({ ...current, [ask.eventId]: cause instanceof Error ? cause.message : String(cause) }));
    }
  };

  return <div className="shrink-0 border-t border-border bg-sidebar" data-pinned-tray>
    <div className="flex h-8 items-center gap-2 px-3 text-[11px] font-medium text-muted-foreground">
      <Pin size={11} className="fill-current" aria-hidden="true" />Pinned agents
      <span className="font-mono">{entries.length}</span>
      <span className="ml-auto flex items-center gap-1"><button type="button" onClick={() => setOpen(value => !value)} aria-expanded={open} className="inline-flex items-center gap-1 rounded px-1 hover:bg-accent hover:text-foreground">{open ? <ChevronDown size={12} aria-hidden="true" /> : <ChevronRight size={12} aria-hidden="true" />}{open ? "hide" : "show"}</button></span>
    </div>
    {open && <div className="flex flex-col gap-0.5 px-1.5 pb-2">
      {entries.map(({ node, run }) => <div key={node.id} className={cn("rounded-lg px-2.5 py-1.5 transition-colors hover:bg-accent", node.ask && "border-l-2 border-l-warning")}>
        <button type="button" onClick={() => onOpenAgents(node.id)} className="flex w-full min-w-0 items-center gap-2 text-left">
          <Dot tone={node.status.tone} live />
          <HarnessMark harness={node.harness} size={13} />
          <b className="min-w-0 truncate text-[12.5px] font-medium text-foreground">{node.name}</b>
          <span className="ml-auto flex shrink-0 items-center gap-1.5 font-mono text-[11px] tabular-nums text-muted-foreground">
            {node.status.tone !== "working" && <span className={cn("text-[10.5px] font-semibold tracking-[0.06em]", TONE_TEXT[node.status.tone])}>{node.status.label || TONE_LABEL[node.status.tone]}</span>}
            {agentClock(node, now)}
          </span>
        </button>
        <div className="mt-0.5 flex items-center gap-2 pl-[27px] text-[11px] text-muted-foreground">
          <span className="min-w-0 truncate">{chatTitle?.(run.rootSessionId) ?? run.title}</span>
          {node.ask && <><span>·</span><span className="shrink-0 font-mono text-foreground">{node.ask.command ?? node.ask.title}</span>
            <button type="button" onClick={() => void answer(node.ask!, "accept")} className="ml-auto shrink-0 rounded bg-primary px-1.5 text-[11px] font-medium leading-5 text-primary-foreground">Approve</button>
          </>}
          {!node.ask && node.liveLine && <><span>·</span><span className="min-w-0 truncate font-mono">{node.liveLine.verb} {node.liveLine.target}</span></>}
        </div>
        {node.ask && errors[node.ask.eventId] && <p role="alert" className="pl-[27px] text-[11px] text-destructive">{errors[node.ask.eventId]}</p>}
      </div>)}
    </div>}
  </div>;
}
