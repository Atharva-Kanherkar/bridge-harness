import { readWireKind } from "./transcript/wire";
import type { AgentEvent } from "./types";

/**
 * What one send cost as the person saw it.
 *
 * The daemon times its own requests, but it cannot see the click, the time a
 * response spent reaching the page, or when the first output was drawn. A
 * flight starts at the Send click, collects the time at each boundary
 * (`resolve_chat_settings`, `create_configured_chat`, `prepare_turn`,
 * `start_chat`, `submit_sent`, `submit_input`, `first_frame`,
 * `first_output_received`, `first_output_painted`), and is reported once as a
 * `latency.client` row. Only a send that started a new turn is reported: a
 * steered or queued one has no first output of its own to wait for.
 * Durations and step names only: never message text.
 */
export type LatencyFlow = "new_chat" | "send";
export type LatencyStep = { name: string; atMs: number };
type Reporter = (sessionId: string, flow: LatencyFlow, steps: LatencyStep[]) => void;

export interface SendFlight {
  flow: LatencyFlow;
  started: number;
  steps: LatencyStep[];
  sessionId?: string;
  submitted: boolean;
  outputSeen: boolean;
  frameSeen: boolean;
  done: boolean;
}

/** A send with no output by then is a stuck turn, not a latency sample. */
const ABANDON_AFTER_MS = 10 * 60_000;
const OUTPUT_KINDS = new Set([
  "message.delta", "reasoning.delta", "reasoning.started", "message.started",
  "tool.started", "command.started", "file_change.started", "message.completed",
]);

const bySession = new Map<string, SendFlight>();
let reporter: Reporter | undefined;
let nextFrame: (callback: () => void) => void = callback => {
  if (typeof requestAnimationFrame === "function") requestAnimationFrame(callback);
  else setTimeout(callback, 0);
};

export function setLatencyReporter(next: Reporter | undefined): void { reporter = next; }
/** Tests replace the frame clock; the app uses requestAnimationFrame. */
export function setLatencyFrameScheduler(next: (callback: () => void) => void): void { nextFrame = next; }

function flight(flow: LatencyFlow, now: number): SendFlight {
  return { flow, started: now, steps: [], submitted: false, outputSeen: false, frameSeen: false, done: false };
}

/** The Send click on a new-chat draft, before the chat exists. */
export function beginNewChat(now = performance.now()): SendFlight {
  return flight("new_chat", now);
}

/** The chat a new-chat flight created; its first send continues the flight. */
export function bindNewChat(target: SendFlight, sessionId: string): void {
  target.sessionId = sessionId;
  bySession.set(sessionId, target);
}

/** A Send into an existing chat, or the first send of a just-created one. */
export function beginSend(sessionId: string, now = performance.now()): SendFlight {
  const created = bySession.get(sessionId);
  if (created && created.flow === "new_chat" && !created.submitted && !created.done) return created;
  const next = flight("send", now);
  next.sessionId = sessionId;
  bySession.set(sessionId, next);
  return next;
}

export function markFlight(target: SendFlight, name: string, now = performance.now()): void {
  if (target.done || target.steps.some(step => step.name === name)) return;
  target.steps.push({ name, atMs: Math.max(0, Math.round(now - target.started)) });
}

/**
 * The submit request is on its way. Frames for the chat count from here, not
 * from its response: the daemon publishes the turn's first frames while the
 * request is still being answered.
 */
export function markSubmitted(target: SendFlight, now = performance.now()): void {
  markFlight(target, "submit_sent", now);
  target.submitted = true;
}

export function abandonFlight(target: SendFlight): void {
  target.done = true;
  if (target.sessionId && bySession.get(target.sessionId) === target) bySession.delete(target.sessionId);
}

function finish(target: SendFlight): void {
  if (target.done || !target.sessionId) return;
  target.done = true;
  if (bySession.get(target.sessionId) === target) bySession.delete(target.sessionId);
  reporter?.(target.sessionId, target.flow, target.steps);
}

/** Every agent frame passes here on arrival, before React sees it. */
export function observeSendLatency(event: AgentEvent, now = performance.now()): void {
  const target = bySession.get(event.sessionId);
  if (!target || !target.submitted || target.done) return;
  if (now - target.started > ABANDON_AFTER_MS) { abandonFlight(target); return; }
  if (!target.frameSeen) { target.frameSeen = true; markFlight(target, "first_frame", now); }
  if (target.outputSeen) return;
  const kind = readWireKind(event.kind);
  if (!OUTPUT_KINDS.has(kind) || event.role === "user") return;
  target.outputSeen = true;
  markFlight(target, "first_output_received", now);
  // Two frames: the first lets React commit the batch this frame joined, the
  // second runs after that commit has been painted.
  nextFrame(() => nextFrame(() => {
    markFlight(target, "first_output_painted");
    finish(target);
  }));
}
