/**
 * The one projection every agent surface reads.
 *
 * Bridge workers and harness-native subagents are the same fact to a human —
 * something this conversation started, which is running, which might need a
 * decision, and which leaves a record — but they arrive from two completely
 * different places. A worker is a `Session` plus a `WorkerRuntimeRecord` in the
 * forest. A Claude `Task` subagent, a Codex collab agent and an OpenCode
 * `task` child are nothing but transcript rows the backend stamped with
 * `data.subagent` (or a `SubagentFacet` tool call) inside *someone's*
 * transcript. Before this module the transcript was the only place either was
 * visible, so a subagent was one tool card and a worker was a chat-sized panel
 * that scrolled away.
 *
 * So: one pure function, no hooks, no components, that merges both sources into
 * a nested `AgentRun[]`. The Agents pane, the pinned tray and the chat's
 * one-line pointer all read this. None of them keeps its own copy, and none of
 * them re-projects runtime fields — worker rows reuse `workerPanelModel` and
 * `workerStatus` so a status label can never drift between the dock and the
 * chat.
 *
 * Three rules the type system cannot enforce, so they are worth stating:
 *
 * 1. **Nothing seen disappears.** A finished, cancelled or failed worker is
 *    still a node. Acknowledging a failure is an *appearance* change (it dims),
 *    not a removal; removing a row is what archiving the chat is for.
 * 2. **No new lifecycle.** Tones are `WorkerTone` and status text is
 *    `WorkerStatus`, the same pair the rest of the app already renders. A
 *    subagent's `SubagentFacet.status` is mapped onto them and nothing else.
 * 3. **No raw provider text as a failure.** A failed row shows the stable
 *    `failureClass` code and the last *legible* step. The provider's own error
 *    string stays in diagnostics, where a stack trace belongs.
 */

import type { AgentEvent, Session, SessionForestSnapshot, WorkerRuntimeRecord } from "../types";
import { readToolCall, type SubagentFacet, type ToolCallDisplay, type ToolStatus } from "../transcript/toolCall";
import type { ToolSurface } from "../transcript/events";
import { subagentSource, type ConversationItem } from "../transcript/item";
import { readWireKind, type WireKind } from "../transcript/wire";
import { legibleWorkerLine, workerPanelModel } from "./workerPanel";
import { workerStatus, type WorkerStatus, type WorkerTone } from "./workerStatus";

/** How many steps an expanded row shows. Matches the mockup and the drill-in. */
export const AGENT_EXPANDED_STEPS = 5;

/** How many steps a collapsed row's live line is drawn from. */
export const AGENT_LIVE_STEP_LIMIT = 1;

export type AgentSource = "worker" | "subagent";

export type AgentsScope = "this-chat" | "all-chats";

/** One legible thing an agent did, already shaped for a row. */
export interface AgentStep {
  id: string;
  verb: string;
  target: string;
  /** Diff counts (`+18 −6`) or a hit count, when the provider reported one. */
  extra?: string;
  /** The same counts as numbers. Kept separately so a footer can total them
   *  without re-parsing a display string it also has to render. */
  additions?: number;
  deletions?: number;
  state: "live" | "done" | "failed" | "pending";
}

/** A question one agent is asking the human, answered in its own row. */
export interface AgentAsk {
  /** The transcript item's `eventId`, so the same resolver the card uses works. */
  eventId: number;
  sessionId: string;
  title: string;
  detail?: string;
  command?: string;
  cwd?: string;
  ownedPaths: string[];
  objective?: string;
}

/** The footer / drill-in counters. Every field is optional: a provider that
 *  never reported a cost must not render `$0.00` as if it had. */
export interface AgentCounters {
  files: number;
  additions: number;
  deletions: number;
  toolCalls: number;
  contextPercent?: number;
  costUsd?: number;
}

export interface AgentNode {
  /** Stable across renders and unique inside a run. For a worker this is its
   *  session id, so a chat pointer can hand it straight back to the pane. */
  id: string;
  source: AgentSource;
  harness: string;
  name: string;
  parentId?: string;
  depth: number;
  status: WorkerStatus;
  liveLine?: AgentStep;
  /** The last `AGENT_EXPANDED_STEPS` legible steps, oldest first. */
  steps: AgentStep[];
  startedAt?: string;
  endedAt?: string;
  children: AgentNode[];
  /** The session whose transcript this agent's activity arrives in. For a
   *  worker that is its own session; for a subagent it is its parent's. */
  sessionId: string;
  model?: string;
  effort?: string;
  branch?: string;
  scope: string[];
  objective?: string;
  /** The stable classification (`sandbox_denied`, `stalled`, …) — never the
   *  provider's own error text. */
  failureCode?: string;
  ask?: AgentAsk;
  counters: AgentCounters;
  /** Set when the human dismissed the failure. The row dims; it does not go. */
  acknowledged?: boolean;
  /** The session row, for the surfaces that need the whole thing (drill-in). */
  session?: Session;
  runtime?: WorkerRuntimeRecord;
  /** The subagent's own task call, for Codex collab children whose events are
   *  reported through `agentsStates` rather than streamed. */
  facet?: SubagentFacet;
}

export interface AgentsCensus {
  running: number;
  needsYou: number;
  done: number;
  failed: number;
  queued: number;
  workers: number;
  subagents: number;
  costUsd: number;
}

export interface AgentRun {
  rootSessionId: string;
  title: string;
  harness: string;
  startedAt?: string;
  agents: AgentNode[];
  census: AgentsCensus;
}

export interface AgentsModelInput {
  sessions: Session[];
  /** One forest per root session, keyed by session id. */
  forests: ReadonlyMap<string, SessionForestSnapshot> | Readonly<Record<string, SessionForestSnapshot>>;
  /** Reduced conversation items per session id, when the host holds them. */
  transcripts?: ReadonlyMap<string, readonly ConversationItem[]> | Readonly<Record<string, readonly ConversationItem[]>>;
  /** The global live event stream, not a single session's slice: a worker's
   *  own frames arrive under the worker's session id. */
  events?: readonly AgentEvent[];
  /** Failures the human has dismissed. They dim; they never disappear. */
  acknowledged?: ReadonlySet<string>;
  /** The chat in front of the human. Only consulted for `this-chat`. */
  rootSessionId?: string;
  scope?: AgentsScope;
  /** Frozen clock for tests. */
  now?: number;
}

function forestOf(input: AgentsModelInput, sessionId: string): SessionForestSnapshot | undefined {
  const source = input.forests;
  if (!source) return undefined;
  if (source instanceof Map) return source.get(sessionId);
  return (source as Readonly<Record<string, SessionForestSnapshot>>)[sessionId];
}

function transcriptOf(input: AgentsModelInput, sessionId: string): readonly ConversationItem[] {
  const source = input.transcripts;
  if (!source) return [];
  if (source instanceof Map) return source.get(sessionId) ?? [];
  return (source as Readonly<Record<string, readonly ConversationItem[]>>)[sessionId] ?? [];
}

// ── steps ───────────────────────────────────────────────────────────────────

const VERB: Record<string, string> = {
  edit: "Edit",
  read: "Read",
  run: "Run",
  search: "Grep",
  tool: "Tool",
};

function stepState(status: ToolStatus): AgentStep["state"] {
  if (status === "running") return "live";
  if (status === "failed") return "failed";
  if (status === "completed") return "done";
  return "pending";
}

function diffExtra(call: ToolCallDisplay): string | undefined {
  const hasAdditions = typeof call.additions === "number" && call.additions > 0;
  const hasDeletions = typeof call.deletions === "number" && call.deletions > 0;
  if (!hasAdditions && !hasDeletions) return undefined;
  return `+${call.additions ?? 0} −${call.deletions ?? 0}`;
}

/** A `Task`/collab call is a step, not a detail panel: it is the moment the
 *  parent spawned a child, and the child's own row hangs off it. */
function subagentStep(call: ToolCallDisplay, subagent: SubagentFacet, id: string): AgentStep {
  return {
    id,
    verb: "Task",
    target: subagent.agentType ?? "subagent",
    extra: subagent.status === "running" ? "running" : subagent.status === "failed" ? "failed" : "done",
    state: subagent.status === "running" ? "live" : subagent.status === "failed" ? "failed" : "done",
  };
}

function stepFromCall(id: string, call: ToolCallDisplay): AgentStep | undefined {
  if (call.subagent) return subagentStep(call, call.subagent, id);
  if (call.pendingIdentity) return undefined;
  // Only a call that named its subject. Falling back to `doing` here is what
  // put "Using a tool" in a row — and, worse, turned an event whose *text* was
  // a machine fence into a step, because the fence still produced a call with
  // a present-tense label and no target.
  const target = call.target ?? call.path ?? call.command;
  if (!target) return undefined;
  return {
    id,
    verb: VERB[call.verb] ?? "Tool",
    target,
    extra: diffExtra(call),
    additions: typeof call.additions === "number" ? call.additions : undefined,
    deletions: typeof call.deletions === "number" ? call.deletions : undefined,
    state: stepState(call.status),
  };
}

function stepFromItem(item: ConversationItem): AgentStep | undefined {
  const call = item.tool ?? readToolCall({ title: item.title, text: item.text, status: item.status, surface: item.type === "diff" ? "diff" : "activity", data: item.data });
  return stepFromCall(item.key || `item:${item.sequence}`, call);
}

// ── worker rows ─────────────────────────────────────────────────────────────

const QUIET_TONES: ReadonlySet<WorkerTone> = new Set<WorkerTone>(["done", "idle"]);

export function isQuietTone(tone: WorkerTone): boolean {
  return QUIET_TONES.has(tone);
}

/**
 * Worker steps, newest last, from the global live stream.
 *
 * Tool rows are the ones worth showing, and they are read through the same
 * `readToolCall` the transcript uses so a row here and a row there cannot
 * disagree about what an Edit was editing. When a worker has produced prose but
 * no tool row yet, its last legible line stands in as a `Result`-shaped step:
 * the alternative is a row with nothing on it, which reads as broken.
 */
function workerSteps(sessionId: string, events: readonly AgentEvent[], feed: readonly { id: number; text: string }[] = []): AgentStep[] {
  const steps: AgentStep[] = [];
  for (const event of events) {
    if (event.sessionId !== sessionId) continue;
    const call = readToolCall({
      title: event.title ?? undefined,
      text: event.text ?? "",
      status: event.status ?? undefined,
      surface: wireSurface(event.kind, event.data),
      data: event.data,
    });
    const step = call.subagent || call.pendingIdentity ? undefined : stepFromCall(`event:${event.id}`, call);
    // A worker that has said something but not yet run a tool still has a live
    // line: its own last legible prose, shaped like a result. The alternative is
    // a row with nothing on it, which reads as broken rather than as early.
    if (step) { steps.push(step); continue; }
    const line = legibleWorkerLine(event.text) ?? legibleWorkerLine(event.title);
    if (line) steps.push({ id: `event:${event.id}`, verb: "Result", target: line, state: stepState(call.status) });
    if (steps.length > AGENT_EXPANDED_STEPS) steps.shift();
  }
  // `workerFeedLines` reads the same stream, so anything it found that the tool
  // pass did not is a prose line on an event the pass skipped. Folded in by
  // event id and re-sorted, which is what keeps "the newest step" meaning the
  // newest thing that happened rather than whichever pass ran last.
  for (const line of feed) {
    if (steps.some(step => step.id === `event:${line.id}`)) continue;
    steps.push({ id: `event:${line.id}`, verb: "Result", target: line.text, state: "done" });
  }
  steps.sort((left, right) => eventOrder(left.id) - eventOrder(right.id));
  return steps.slice(-AGENT_EXPANDED_STEPS);
}

/**
 * The same surface rule the transcript codec applies, in one place.
 *
 * It decides whether a bare frame counts as an edit or a generic tool, so
 * copying the rule rather than importing the codec's private helper keeps the
 * pane and the transcript reading the same event the same way — the alternative
 * is a second rule that quietly disagrees.
 */
function wireSurface(kind: WireKind, data: Record<string, unknown>): ToolSurface {
  const raw = readWireKind(kind);
  if (raw.startsWith("file_change.") || raw.startsWith("diff.")) return "diff";
  return data.kind === "edit" ? "diff" : "activity";
}

/** Event ids sort numerically inside a step id, so "the newest" is one number. */
function eventOrder(stepId: string): number {
  const id = Number(stepId.replace(/^event:/, ""));
  return Number.isFinite(id) ? id : 0;
}

function costMicrousd(forest: SessionForestSnapshot | undefined, sessionId: string): number | undefined {
  if (!forest) return undefined;
  let total = 0;
  let seen = false;
  for (const row of forest.usage) {
    if (row.sessionId !== sessionId) continue;
    const value = typeof row.costMicrousd === "number" ? row.costMicrousd : null;
    if (value === null) continue;
    total += value;
    seen = true;
  }
  return seen ? total / 1_000_000 : undefined;
}

function contextPercentOf(forest: SessionForestSnapshot | undefined, sessionId: string, session?: Session): number | undefined {
  const ledger = forest?.usage.find(row => row.sessionId === sessionId && typeof row.contextPercent === "number");
  if (ledger && typeof ledger.contextPercent === "number") return ledger.contextPercent;
  return typeof session?.contextPercent === "number" ? session.contextPercent : undefined;
}

/** The worker's write scope, from the lease that granted it. */
function leaseScope(forest: SessionForestSnapshot | undefined, sessionId: string): string[] {
  const lease = forest?.workerLeases.find(entry => entry.sessionId === sessionId);
  return Array.isArray(lease?.ownedPaths) ? lease.ownedPaths.map(String) : [];
}

// ── asks ────────────────────────────────────────────────────────────────────

function stringList(value: unknown): string[] {
  return Array.isArray(value) ? value.map(String).filter(entry => entry !== "") : [];
}

/**
 * The question a child is asking, read off the transcript.
 *
 * A worker blocked on its own in-session approval is mirrored onto its
 * orchestrator's transcript as a `delegation.blocked` row carrying
 * `childSessionId`, because the card would otherwise render on a conversation
 * nobody is looking at. That row is the same fact the `ApprovalCard` renders,
 * so answering it here goes through the same resolver.
 */
export function askFromItems(items: readonly ConversationItem[], childSessionId: string, ownerSessionId: string): AgentAsk | undefined {
  for (const item of items) {
    if (item.data.childBlocked !== true) continue;
    if (typeof item.data.childSessionId !== "string" || item.data.childSessionId !== childSessionId) continue;
    if (item.status && item.status !== "pending" && item.status !== "waiting") continue;
    return {
      eventId: item.eventId,
      sessionId: ownerSessionId,
      title: typeof item.data.title === "string" ? item.data.title : typeof item.title === "string" ? item.title : "Approval needed",
      detail: item.text?.trim() || undefined,
      command: typeof item.data.command === "string" ? item.data.command : undefined,
      cwd: typeof item.data.cwd === "string" ? item.data.cwd : undefined,
      ownedPaths: stringList(item.data.ownedPaths ?? item.data.requestedOwnedPaths),
      objective: typeof item.data.objective === "string" ? item.data.objective : undefined,
    };
  }
  return undefined;
}

// ── subagent rows ───────────────────────────────────────────────────────────

const SUBAGENT_STATUS: Record<NonNullable<SubagentFacet["status"]>, WorkerStatus> = {
  running: { tone: "working", label: "WORKING" },
  completed: { tone: "done", label: "DONE" },
  failed: { tone: "failed", label: "FAILED" },
};

/** One child, however it announced itself. */
interface SubagentGroup {
  /** The child session the backend stamped, or a synthetic key for a collab
   *  agent that never got one. */
  childId: string;
  ownerSessionId: string;
  agent?: string;
  title?: string;
  facet?: SubagentFacet;
  rows: ConversationItem[];
}

/**
 * Group one transcript's rows into the children it spawned.
 *
 * Two independent sources, deliberately: rows stamped `data.subagent` (what
 * OpenCode's `task` and, after the Claude normalizer change, a Claude `Task`
 * produce) and `SubagentFacet` tool calls (what a Codex collab agent produces).
 * A collab child the app-server never streams tool events for ends up with a
 * status and a result but no live step — stated here, in the shape, rather than
 * papered over with a fabricated one.
 */
function subagentGroups(items: readonly ConversationItem[]): SubagentGroup[] {
  const groups = new Map<string, SubagentGroup>();
  for (const item of items) {
    const source = subagentSource(item);
    const facet = item.tool?.subagent;
    const agent = source?.agent ?? facet?.agentType;
    // A bare stamped row with no agent and no task call is still a child: its
    // `sessionId` alone is enough to nest it under the right parent.
    const key = source?.sessionId ?? (agent ? `collab:${agent}` : undefined);
    if (!key) continue;
    const existing = groups.get(key);
    if (existing) {
      existing.agent ??= agent;
      existing.title ??= source?.title ?? facet?.description;
      existing.facet ??= facet;
      existing.rows.push(item);
      continue;
    }
    groups.set(key, { childId: source?.sessionId ?? key, ownerSessionId: "", agent, title: source?.title ?? facet?.description, facet, rows: [item] });
  }
  return [...groups.values()];
}

function subagentNode(owner: { id: string; harness: string }, group: SubagentGroup): AgentNode {
  const steps: AgentStep[] = [];
  for (const row of group.rows) {
    const step = stepFromItem(row);
    if (step) steps.push(step);
    if (steps.length > AGENT_EXPANDED_STEPS) steps.shift();
  }
  const status = group.facet?.status
    ? SUBAGENT_STATUS[group.facet.status]
    : steps.length > 0 ? { tone: "working" as const, label: "WORKING" } : { tone: "working" as const, label: "STARTING" };
  const first = group.rows[0];
  const last = group.rows[group.rows.length - 1];
  const additions = group.rows.reduce((sum, row) => sum + (row.tool?.additions ?? 0), 0);
  const deletions = group.rows.reduce((sum, row) => sum + (row.tool?.deletions ?? 0), 0);
  return {
    id: `${owner.id}:sub:${group.childId}`,
    source: "subagent",
    harness: owner.harness,
    name: group.agent ?? group.title ?? "subagent",
    depth: 0,
    status,
    liveLine: steps[steps.length - 1],
    steps,
    startedAt: first?.createdAt,
    endedAt: status.tone === "done" || status.tone === "failed" ? last?.createdAt : undefined,
    children: [],
    sessionId: owner.id,
    scope: [],
    objective: group.facet?.description ?? group.title,
    counters: {
      files: new Set(group.rows.map(row => row.tool?.path).filter((path): path is string => !!path)).size,
      additions,
      deletions,
      toolCalls: steps.length,
    },
    facet: group.facet,
  };
}

// ── the model ───────────────────────────────────────────────────────────────

function isRoot(session: Session): boolean {
  return !session.parentSessionId;
}

function censusOf(agents: readonly AgentNode[], queued: number, costUsd: number): AgentsCensus {
  let running = 0;
  let needsYou = 0;
  let done = 0;
  let failed = 0;
  let workers = 0;
  let subagents = 0;
  const walk = (nodes: readonly AgentNode[]) => {
    for (const node of nodes) {
      if (node.source === "worker") workers += 1; else subagents += 1;
      if (node.ask) needsYou += 1;
      if (node.status.tone === "working") running += 1;
      else if (node.status.tone === "failed" || node.status.tone === "stalled") failed += 1;
      else if (node.status.tone === "done" || node.status.tone === "idle") done += 1;
      walk(node.children);
    }
  };
  walk(agents);
  return { running, needsYou, done, failed, queued, workers, subagents, costUsd };
}

/** `3 running · 1 needs you · 2 done`, mono, the way every other count reads. */
export function censusLine(census: AgentsCensus): string {
  const parts: string[] = [];
  if (census.running > 0) parts.push(`${census.running} running`);
  if (census.needsYou > 0) parts.push(`${census.needsYou} needs you`);
  if (census.done > 0) parts.push(`${census.done} done`);
  if (census.failed > 0) parts.push(`${census.failed} failed`);
  return parts.join(" · ");
}

function runCost(forest: SessionForestSnapshot | undefined): number {
  let total = 0;
  for (const row of forest?.usage ?? []) {
    if (typeof row.costMicrousd === "number") total += row.costMicrousd;
  }
  return total / 1_000_000;
}

/**
 * The projection. One `AgentRun` per root session that has agents, ordered by
 * the root's own start time so the newest chat leads.
 *
 * `scope` is a *filter on output*, not a different code path: `all-chats` asks
 * the host for more forests, and this function then treats them the same as the
 * chat in front of the human.
 */
export function agentsModel(input: AgentsModelInput): AgentRun[] {
  const events = input.events ?? [];
  const now = input.now ?? Date.now();
  const acknowledged = input.acknowledged ?? new Set<string>();
  const scope = input.scope ?? "this-chat";
  const runs: AgentRun[] = [];

  const roots = input.sessions
    .filter(session => isRoot(session) && (scope === "all-chats" || session.id === input.rootSessionId))
    .filter(session => forestOf(input, session.id) !== undefined || transcriptOf(input, session.id).length > 0);

  for (const root of roots) {
    const forest = forestOf(input, root.id);
    const runtimes = forest?.workerRuntimes ?? [];
    const childIds = new Set(runtimes.map(runtime => runtime.sessionId));
    const nodes: AgentNode[] = [];

    for (const runtime of runtimes) {
      const worker = input.sessions.find(session => session.id === runtime.sessionId);
      if (!worker) continue;
      // `workerPanelModel` is the only status resolver, and reusing it here is
      // what keeps a worker's label identical in the pane, the tray and the
      // chat. It returns null for a session it cannot find, which the guard
      // above has already excluded.
      // Five lines for the expanded body, so the feed's own bound matches the
      // row's. The live line is the last of them, which is why one limit serves
      // both: `workerFeedLines` already keeps the newest.
      const panel = workerPanelModel(worker.id, input.sessions, runtimes, events as AgentEvent[], AGENT_EXPANDED_STEPS);
      const status = panel?.status ?? workerStatus(worker, runtime);
      const allSteps = workerSteps(worker.id, events, panel?.feed ?? []);
      const endedAt = worker.endedAt ?? (runtime.resultStatus === "reported" ? runtime.updatedAt : undefined);
      nodes.push({
        id: worker.id,
        source: "worker",
        harness: worker.harness,
        name: worker.label,
        depth: 0,
        status,
        liveLine: allSteps[allSteps.length - 1],
        steps: allSteps,
        startedAt: worker.startedAt ?? undefined,
        endedAt: endedAt ?? undefined,
        children: [],
        sessionId: worker.id,
        model: worker.model ?? undefined,
        effort: worker.effort ?? undefined,
        branch: runtime.worktreeBranch ?? undefined,
        scope: leaseScope(forest, worker.id),
        objective: runtime.progressSummary ?? panel?.progressSummary,
        failureCode: status.tone === "failed" || status.tone === "stalled" ? (runtime.failureClass ?? status.tone) : undefined,
        ask: askFromItems(transcriptOf(input, root.id), worker.id, root.id),
        counters: {
          files: panel?.result?.filesChanged.length ?? new Set(allSteps.map(step => step.target)).size,
          additions: allSteps.reduce((sum, step) => sum + (step.additions ?? 0), 0),
          deletions: allSteps.reduce((sum, step) => sum + (step.deletions ?? 0), 0),
          toolCalls: allSteps.length,
          contextPercent: contextPercentOf(forest, worker.id, worker),
          costUsd: costMicrousd(forest, worker.id),
        },
        acknowledged: acknowledged.has(worker.id),
        session: worker,
        runtime,
      });
    }

    // Harness subagents, read out of whichever transcript they arrived in: a
    // Claude `Task` (or an OpenCode `task`) started by a worker nests under that
    // worker, one started by the orchestrator sits at the top of the run, and a
    // Codex collab agent — which carries no `data.subagent` at all, only a
    // `SubagentFacet` on its parent's tool call — still gets its row.
    const owners = [root, ...input.sessions.filter(session => childIds.has(session.id))];
    for (const owner of owners) {
      for (const group of subagentGroups(transcriptOf(input, owner.id))) {
        const node = subagentNode(owner, group);
        const parent = nodes.find(candidate => candidate.id === owner.id);
        if (parent && !parent.children.some(existing => existing.id === node.id)) {
          node.parentId = parent.id;
          node.depth = parent.depth + 1;
          parent.children.push(node);
        } else if (!nodes.some(existing => existing.id === node.id)) {
          nodes.push(node);
        }
      }
    }

    if (nodes.length === 0 && !(forest?.workerQueue.length)) continue;
    runs.push({
      rootSessionId: root.id,
      title: root.title || root.label,
      harness: root.harness,
      startedAt: root.startedAt ?? undefined,
      agents: nodes,
      census: censusOf(nodes, forest?.workerQueue.length ?? 0, runCost(forest)),
    });
  }

  runs.sort((left, right) => (left.startedAt ?? "").localeCompare(right.startedAt ?? ""));
  return runs;
}

/** One node anywhere in the run, by id. Powers "focus this row" from the chat. */
export function findAgent(runs: readonly AgentRun[], id: string): AgentNode | undefined {
  const walk = (nodes: readonly AgentNode[]): AgentNode | undefined => {
    for (const node of nodes) {
      if (node.id === id) return node;
      const hit = walk(node.children);
      if (hit) return hit;
    }
    return undefined;
  };
  for (const run of runs) {
    const hit = walk(run.agents);
    if (hit) return hit;
  }
  return undefined;
}

/** The parent chain of a node, root first — the drill-in breadcrumb. */
export function agentTrail(runs: readonly AgentRun[], id: string): AgentNode[] {
  const walk = (nodes: readonly AgentNode[], trail: AgentNode[]): AgentNode[] | undefined => {
    for (const node of nodes) {
      const next = [...trail, node];
      if (node.id === id) return next;
      const hit = walk(node.children, next);
      if (hit) return hit;
    }
    return undefined;
  };
  for (const run of runs) {
    const hit = walk(run.agents, []);
    if (hit) return hit;
  }
  return [];
}

/** The same clock for a run's own header, which has a start and no end yet. */
export function runClock(run: AgentRun, now: number): string {
  return agentClock({ startedAt: run.startedAt } as AgentNode, now);
}

/**
 * The tabular clock a row wears: `4m 12s`, `38s`, `1h 04m`.
 *
 * Second-level on purpose. A row is the only place the human can watch a run
 * move, and a clock that only ticks once a minute reads as stuck for 59 of
 * every 60 seconds — the exact failure the live step line is there to fix.
 */
export function agentClock(node: AgentNode, now: number): string {
  const start = node.startedAt ? new Date(node.startedAt).getTime() : Number.NaN;
  if (!Number.isFinite(start)) return node.endedAt ? "0s" : "0s";
  const end = node.endedAt ? new Date(node.endedAt).getTime() : now;
  const total = Math.max(0, Math.floor((end - start) / 1000));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = total % 60;
  if (hours) return `${hours}h ${String(minutes).padStart(2, "0")}m`;
  if (minutes) return `${minutes}m ${String(seconds).padStart(2, "0")}s`;
  return `${seconds}s`;
}
