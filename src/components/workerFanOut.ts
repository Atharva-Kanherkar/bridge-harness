import { delegationChildSessionId, delegationFacet, type ConversationItem } from "../conversation";
import type { Session, WorkerRuntimeRecord } from "../types";
import { useNow } from "../useNow";
import { agentName, agentTopic } from "./agentIdentity";
import { workerStatus, type WorkerTone } from "./workerStatus";

// The workers one delegation started, read as one thing.
//
// The orchestrator's transcript gives every worker its own quiet row, which is
// right for what each one was asked and what came back, and wrong for the
// question the reader actually has mid-run: how many are still going, does one
// need me, what is the orchestrator waiting on. A fan-out is the answer: the
// worker rows one delegation produced, grouped, with their states counted.
//
// This is the agent tree only. A fan-out is built from delegation rows and
// their child sessions; the conversation tree (forks, branches, rewinds) never
// enters it, and a worktree is a fact about a worker, never a grouping key.

/** The five things a worker can be, as the fan-out counts them. */
export type WorkerState = "working" | "needs-you" | "done" | "failed" | "stopped";

export const WORKER_STATES: readonly WorkerState[] = ["working", "needs-you", "done", "failed", "stopped"];

export const WORKER_STATE_WORDS: Record<WorkerState, string> = {
  working: "working",
  "needs-you": "needs you",
  done: "done",
  failed: "failed",
  stopped: "stopped",
};

/** Status ink: live is info, a question for you is warning, done is success. */
export const WORKER_STATE_TEXT: Record<WorkerState, string> = {
  working: "text-info",
  "needs-you": "text-warning",
  done: "text-success",
  failed: "text-destructive",
  stopped: "text-muted-foreground",
};

/** Doing something now, or waiting on you: what an orchestrator waits on. */
export function isLiveState(state: WorkerState): boolean {
  return state === "working" || state === "needs-you";
}

/// How a worker's tone reads as a fan-out state. A cancellation is a decision,
/// not a fault, so it is `stopped` rather than `failed` (see `workerStatus`).
/// `attention` covers a reported block, an unreadable result and a request for
/// more delegation: each one waits on a person.
export function stateOfTone(tone: WorkerTone): WorkerState {
  switch (tone) {
    case "working": return "working";
    case "waiting":
    case "attention": return "needs-you";
    case "warm":
    case "done": return "done";
    case "failed":
    case "stalled": return "failed";
    case "idle": return "stopped";
  }
}

/** One delegation's worker rows, anchored at the first of them. */
export type FanOut = {
  /** The key of the first worker row: where the card is drawn. */
  key: string;
  turn: number;
  /** The worker rows, in transcript order. One per worker. */
  members: ConversationItem[];
};

export type FanOuts = {
  /** Fan-outs by the key of the row they are drawn at. */
  byHead: Map<string, FanOut>;
  /** Rows the card draws inside itself, so the transcript skips them. */
  absorbed: Set<string>;
  /** The latest approval mirror per child, for the transcript-only state. */
  blocked: Map<string, ConversationItem>;
};

/// A worker row the card can hold: a spawn, or the spawn with its result
/// folded on. A classified failure keeps its own alert row in the transcript,
/// because it carries an action the reader has to see (Retry).
export function isQuietWorkerRow(item: ConversationItem): boolean {
  if (!isBridgeWorkerRow(item)) return false;
  const facet = delegationFacet(item);
  if (facet === "spawn") return true;
  return facet === "result" && typeof item.data.failureCause !== "string";
}

/**
 * A delegation row for a Bridge worker. A provider-owned subagent (folded in
 * as a delegation with `harnessSubagent`) is not one: it keeps its own row and
 * its own Stop, and a fan-out card never counts or stops it.
 */
function isBridgeWorkerRow(item: ConversationItem): boolean {
  return item.type === "delegation" && !!delegationChildSessionId(item) && item.data.harnessSubagent !== true;
}

function isWorkerRow(item: ConversationItem): boolean {
  if (!isBridgeWorkerRow(item)) return false;
  const facet = delegationFacet(item);
  return facet === "spawn" || facet === "result";
}

/**
 * Group the transcript's worker rows into fan-outs.
 *
 * Worker rows of one turn belong to one fan-out while nothing but other
 * delegation rows (an approval mirror, a steer, a rejected launch) falls
 * between them. Prose, a tool run, an approval card or a new turn closes it:
 * pulling a later row up past something the reader has to read would reorder
 * the narrative, which is the one thing a transcript must not do.
 *
 * Expects the folded list (`foldWorkerDelegations`), so a worker is one row.
 */
export function groupFanOuts(items: readonly ConversationItem[]): FanOuts {
  const byHead = new Map<string, FanOut>();
  const absorbed = new Set<string>();
  const blocked = new Map<string, ConversationItem>();
  let open: FanOut | null = null;
  for (const item of items) {
    if (item.type === "delegation" && delegationFacet(item) === "blocked") {
      const child = delegationChildSessionId(item);
      if (child) blocked.set(child, item);
    }
    if (isWorkerRow(item)) {
      if (open && open.turn === item.turn) {
        open.members.push(item);
      } else {
        open = { key: item.key, turn: item.turn, members: [item] };
        byHead.set(item.key, open);
      }
      continue;
    }
    if (item.type !== "delegation") open = null;
  }
  for (const fanOut of byHead.values()) {
    for (const member of fanOut.members) {
      if (member.key !== fanOut.key && isQuietWorkerRow(member)) absorbed.add(member.key);
    }
  }
  return { byHead, absorbed, blocked };
}

/** Where a worker's live state comes from when Bridge has it. */
export type WorkerLookup = {
  sessions?: readonly Session[];
  runtimes?: readonly WorkerRuntimeRecord[];
};

export type FanOutWorker = {
  childSessionId: string;
  item: ConversationItem;
  /** The worker's topic when Bridge cut one, else its personal name. */
  name: string;
  harness?: string;
  state: WorkerState;
  /** Epoch ms, when known. */
  startedAt?: number;
  endedAt?: number;
};

function epoch(value: string | null | undefined): number | undefined {
  if (!value) return undefined;
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : undefined;
}

/// What the transcript alone says about a worker. Used when Bridge has no
/// session or runtime for it (an archived chat, a preview, a test), and always
/// once its result is folded on: a result is final for the delegation it
/// answers, even if the same worker was later reused for another.
function transcriptState(item: ConversationItem, blocked?: ConversationItem): WorkerState {
  if ("delivered" in item.data) {
    const status = typeof item.data.status === "string" ? item.data.status : "";
    if (typeof item.data.failureCause === "string" || status === "failed") return "failed";
    if (status === "protocol_invalid" || status === "blocked" || status === "needs_delegation") return "needs-you";
    if (status === "cancelled") return "stopped";
    return "done";
  }
  if (blocked?.data.childBlocked === true) return "needs-you";
  return "working";
}

/** Each worker of a fan-out with its state, name and clock. */
export function fanOutWorkers(fanOut: FanOut, blocked: ReadonlyMap<string, ConversationItem>, lookup: WorkerLookup = {}): FanOutWorker[] {
  return fanOut.members.flatMap(item => {
    const childSessionId = delegationChildSessionId(item);
    if (!childSessionId) return [];
    const session = lookup.sessions?.find(candidate => candidate.id === childSessionId);
    const runtime = lookup.runtimes?.find(candidate => candidate.sessionId === childSessionId);
    const settledHere = "delivered" in item.data;
    const state = !settledHere && session && runtime
      ? stateOfTone(workerStatus(session, runtime).tone)
      : transcriptState(item, blocked.get(childSessionId));
    const reported = runtime?.resultStatus === "reported" ? runtime.updatedAt : undefined;
    const dataHarness = typeof item.data.harness === "string" ? item.data.harness : undefined;
    return [{
      childSessionId,
      item,
      name: (session && agentTopic(session)) ?? agentName(childSessionId),
      harness: session?.harness ?? dataHarness,
      state,
      startedAt: epoch(session?.startedAt) ?? epoch(item.createdAt),
      endedAt: isLiveState(state) ? undefined : epoch(session?.endedAt) ?? epoch(reported),
    }];
  });
}

export type FanOutSummary = {
  total: number;
  counts: Record<WorkerState, number>;
  /** The workers still being waited on, in transcript order. */
  waiting: FanOutWorker[];
  /** Up to three marks, then how many more. */
  marks: FanOutWorker[];
  overflow: number;
  live: boolean;
  startedAt?: number;
  /** When the last worker settled, once every one has and Bridge knows when. */
  settledAt?: number;
};

export const MAX_FAN_OUT_MARKS = 3;

export function summarizeFanOut(workers: readonly FanOutWorker[]): FanOutSummary {
  const counts = Object.fromEntries(WORKER_STATES.map(state => [state, 0])) as Record<WorkerState, number>;
  for (const worker of workers) counts[worker.state] += 1;
  const waiting = workers.filter(worker => isLiveState(worker.state));
  const starts = workers.flatMap(worker => worker.startedAt === undefined ? [] : [worker.startedAt]);
  const ends = workers.flatMap(worker => worker.endedAt === undefined ? [] : [worker.endedAt]);
  const live = waiting.length > 0;
  return {
    total: workers.length,
    counts,
    waiting,
    marks: workers.slice(0, MAX_FAN_OUT_MARKS),
    overflow: Math.max(0, workers.length - MAX_FAN_OUT_MARKS),
    live,
    startedAt: starts.length ? Math.min(...starts) : undefined,
    settledAt: !live && ends.length ? Math.max(...ends) : undefined,
  };
}

/** "1 working · 1 needs you · 1 done": the states present, in a fixed order. */
export function fanOutCountParts(counts: Record<WorkerState, number>): Array<{ state: WorkerState; text: string }> {
  return WORKER_STATES.filter(state => counts[state] > 0).map(state => ({ state, text: `${counts[state]} ${WORKER_STATE_WORDS[state]}` }));
}

/** A running clock: 0:48, 12:05, 1:02:05. */
export function formatWorkerClock(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = String(total % 60).padStart(2, "0");
  return hours ? `${hours}:${String(minutes).padStart(2, "0")}:${seconds}` : `${minutes}:${seconds}`;
}

/** The fan-out's clock: live until the last worker settles, then held. */
export function fanOutElapsed(summary: Pick<FanOutSummary, "startedAt" | "settledAt" | "live">, now: number): number | undefined {
  if (summary.startedAt === undefined) return undefined;
  if (summary.live) return now - summary.startedAt;
  return summary.settledAt === undefined ? undefined : summary.settledAt - summary.startedAt;
}

/** "NEEDS YOU" as the lineage writes it: "Needs you". */
export function sentenceCase(label: string): string {
  const lower = label.toLowerCase();
  return lower.charAt(0).toUpperCase() + lower.slice(1);
}

/// Now, once a second while `active`. One interval per caller, and none at
/// all once nothing it draws is live, so a settled card costs nothing.
export function useTickingNow(active: boolean): number {
  return useNow(active ? 1000 : null);
}
