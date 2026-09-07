import { MAX_PENDING_AGENT_EVENTS, queueAgentEvent } from "./agentEvents";
import { readWireKind } from "./transcript/wire";
import type { AgentEvent } from "./types";

/** 32 ms intentional buffering budget: native IPC gets 16 ms; the webview
 * gets at most the remaining 16 ms, or its next animation frame. Scheduler/OS
 * stalls can exceed this budget and are measured separately. */
export const DISPLAY_BUDGET_MS = 32;
export const NATIVE_BATCH_BUDGET_MS = 16;
export interface DisplayClock {
  frame(callback: () => void): number;
  cancelFrame(id: number): void;
  timeout(callback: () => void, ms: number): number;
  cancelTimeout(id: number): void;
}
const deferredKinds = new Set(["message.delta", "reasoning.delta", "command.output_delta", "diff.delta", "tool.progress"]);

export function createDisplayScheduler(deliver: (events: AgentEvent[]) => void, clock: DisplayClock) {
  let queue: AgentEvent[] = [];
  let frame: number | undefined;
  let timeout: number | undefined;
  let disposed = false;
  const firstContentBySession = new Map<string, boolean>();
  const cancel = () => {
    if (frame !== undefined) clock.cancelFrame(frame);
    if (timeout !== undefined) clock.cancelTimeout(timeout);
    frame = timeout = undefined;
  };
  const flush = () => {
    cancel();
    if (disposed || !queue.length) return;
    const batch = queue;
    queue = [];
    deliver(batch);
  };
  return {
    push(event: AgentEvent) {
      if (disposed) return;
      const kind = readWireKind(event.kind);
      const newSession = !firstContentBySession.has(event.sessionId);
      if (newSession || kind === "turn.started") firstContentBySession.set(event.sessionId, true);
      // Keep concurrent workers in the same frame batch after their first text.
      // Session switches in the UI do not change event session ownership.
      while (firstContentBySession.size > 256) firstContentBySession.delete(firstContentBySession.keys().next().value!);
      // Flush before the cap can evict an uncommitted lifecycle event.
      if (queue.length >= MAX_PENDING_AGENT_EVENTS - 1) flush();
      queue = queueAgentEvent(queue, event);
      const visible = !!event.text && (kind === "message.delta" || kind === "reasoning.delta");
      const immediate = !deferredKinds.has(kind) || newSession || (visible && firstContentBySession.get(event.sessionId));
      if (visible) firstContentBySession.set(event.sessionId, false);
      if (immediate) { flush(); return; }
      if (frame !== undefined) return;
      frame = clock.frame(flush);
      // rAF is suspended in background windows; a timer still drains the queue.
      timeout = clock.timeout(flush, DISPLAY_BUDGET_MS - NATIVE_BATCH_BUDGET_MS);
    },
    flush,
    dispose() { disposed = true; cancel(); queue = []; firstContentBySession.clear(); },
    pendingCount: () => queue.length,
  };
}
