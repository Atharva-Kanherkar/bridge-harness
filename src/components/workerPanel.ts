import type { AgentEvent, Session, WorkerRuntimeRecord } from "../types";
import { workerStatus, type WorkerStatus } from "./workerStatus";

/**
 * How many activity lines a chat-embedded worker panel shows.
 *
 * Three, because the panel sits inline in a conversation the user is reading —
 * it answers "is this moving and roughly where" and hands off to the full
 * `WorkerDetail` feed for anything more. It is also the memory bound: the
 * projection slices, so a worker that emits ten thousand events still costs
 * three lines here.
 */
export const WORKER_PANEL_FEED_LINES = 3;

export type WorkerPanelFeedLine = { id: number; text: string };

/**
 * The machine blocks a worker and its orchestrator talk to each other in.
 *
 * A worker's typed envelope arrives inside a ```bridge-worker-result fence, and
 * the adapters happily record the fence's opening line as the worker's newest
 * activity. The card then showed a lone "```bridge-worker-result" where its
 * one-line progress note belongs — protocol plumbing presented to a human as
 * status. These lines are never worth a row; they are the wire, not the work.
 */
const MACHINE_TAGS = new Set(["bridge-delegate", "bridge-worker-result", "bridge-peek", "bridge-steer", "bridge-stop"]);

/** One line of worker activity, or nothing if it is only wire chatter. */
export function legibleWorkerLine(text: string | null | undefined): string | undefined {
  const trimmed = (text ?? "").trim();
  if (!trimmed) return undefined;
  // A fence and nothing else. A fence with content after the tag is prose.
  if (/^(?:```|~~~)\s*[\w-]*$/.test(trimmed)) return undefined;
  if (MACHINE_TAGS.has(trimmed)) return undefined;
  return trimmed;
}

/** The result facts worth showing once a worker has reported. */
export interface WorkerPanelResult {
  status?: string;
  summary?: string;
  filesChanged: string[];
  tests: { command: string; status: string }[];
}

export interface WorkerPanelModel {
  session: Session;
  status: WorkerStatus;
  /** Non-zero only; the panel hides a retry count of zero rather than showing "retry 0". */
  retryCount: number;
  progressSummary?: string;
  waitingReason?: string;
  waitingSince?: string;
  startedAt?: string;
  /** Set once the worker's session closed, so a finished panel reports how long
   *  the work took rather than how long ago it started. */
  endedAt?: string;
  taskFamily?: string;
  feed: WorkerPanelFeedLine[];
  /** True once the typed envelope is in, which is what turns the panel into a result card. */
  reported: boolean;
  result?: WorkerPanelResult;
}

function stringList(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((entry): entry is string => typeof entry === "string") : [];
}

function testList(value: unknown): { command: string; status: string }[] {
  if (!Array.isArray(value)) return [];
  return value.flatMap(entry => {
    if (!entry || typeof entry !== "object") return [];
    const record = entry as Record<string, unknown>;
    const command = typeof record.command === "string" ? record.command : undefined;
    if (!command) return [];
    return [{ command, status: typeof record.status === "string" ? record.status : "unknown" }];
  });
}

function panelResult(runtime: WorkerRuntimeRecord | undefined): WorkerPanelResult | undefined {
  if (!runtime || runtime.resultStatus !== "reported") return undefined;
  const last = runtime.lastResult;
  if (!last) return undefined;
  return {
    status: typeof last.status === "string" ? last.status : undefined,
    summary: typeof last.summary === "string" ? last.summary : undefined,
    filesChanged: stringList(last.filesChanged),
    tests: testList(last.tests),
  };
}

/**
 * The last few legible things one worker said or did, newest last.
 *
 * Deltas and bare lifecycle frames carry no text, so filtering on text is what
 * keeps this to lines a human can read. Consecutive duplicates collapse: a
 * provider that re-emits the same title on every progress frame would otherwise
 * fill the whole panel with one repeated line.
 */
export function workerFeedLines(
  events: AgentEvent[],
  sessionId: string,
  limit = WORKER_PANEL_FEED_LINES,
): WorkerPanelFeedLine[] {
  const lines: WorkerPanelFeedLine[] = [];
  for (const event of events) {
    if (event.sessionId !== sessionId) continue;
    const text = legibleWorkerLine(event.text) ?? legibleWorkerLine(event.title);
    if (!text) continue;
    const last = lines[lines.length - 1];
    if (last && last.text === text) { last.id = event.id; continue; }
    lines.push({ id: event.id, text });
    // Bounded as it accumulates rather than at the end: a worker with a long
    // history must not build a full-length array on every render.
    if (lines.length > limit) lines.shift();
  }
  return lines;
}

/**
 * Everything a live worker panel draws, from the session row, its runtime
 * record, and the global live event stream.
 *
 * Pure and separate from the card that renders it: the projection is the part
 * with the bounds and the fallbacks in it, and that is the part worth testing
 * without a DOM.
 */
export function workerPanelModel(
  childSessionId: string,
  sessions: Session[],
  runtimes: WorkerRuntimeRecord[],
  events: AgentEvent[],
  feedLines = WORKER_PANEL_FEED_LINES,
): WorkerPanelModel | null {
  const session = sessions.find(candidate => candidate.id === childSessionId);
  if (!session) return null;
  const runtime = runtimes.find(candidate => candidate.sessionId === childSessionId);
  return {
    session,
    status: workerStatus(session, runtime),
    retryCount: Number(runtime?.retryCount ?? 0),
    progressSummary: legibleWorkerLine(runtime?.progressSummary),
    waitingReason: runtime?.waitingReason ?? undefined,
    waitingSince: runtime?.waitingSince ?? undefined,
    startedAt: session.startedAt ?? undefined,
    endedAt: session.endedAt ?? undefined,
    taskFamily: runtime?.taskFamily,
    feed: workerFeedLines(events, childSessionId, feedLines),
    reported: runtime?.resultStatus === "reported",
    result: panelResult(runtime),
  };
}
