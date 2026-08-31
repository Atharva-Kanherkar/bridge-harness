import type { AgentEvent } from "./types";

const ADDITIVE_DELTAS = new Set([
  "message.delta",
  "reasoning.delta",
  "command.output_delta",
  "diff.delta",
]);
const REPLACEABLE_SNAPSHOTS = new Set(["tool.progress"]);

// The item cap alone bounded nothing: one merged command-output item could
// grow without limit. Merged text keeps its tail — the end of a stream is
// what the user is watching — and the batch as a whole keeps the newest
// events that fit the total text budget. Durable history is unaffected; these
// are the live, transient frames only.
export const MAX_MERGED_EVENT_TEXT = 256_000;
export const MAX_TOTAL_EVENT_TEXT = 2_000_000;
export const MAX_PENDING_AGENT_EVENTS = 256;

function mergeKey(event: AgentEvent): string | undefined {
  if (!event.itemId || (!ADDITIVE_DELTAS.has(event.kind) && !REPLACEABLE_SNAPSHOTS.has(event.kind))) return undefined;
  return `${event.sessionId}\u0000${event.kind}\u0000${event.itemId}`;
}

function durableKey(event: AgentEvent): string | undefined {
  return event.id > 0 ? `${event.sessionId}\u0000${event.id}` : undefined;
}

function clampText(text: string): string {
  return text.length > MAX_MERGED_EVENT_TEXT ? text.slice(-MAX_MERGED_EVENT_TEXT) : text;
}

function clearItemMergeIndexes(indexes: Map<string, number>, event: AgentEvent): void {
  if (!event.itemId) return;
  for (const kind of [...ADDITIVE_DELTAS, ...REPLACEABLE_SNAPSHOTS]) {
    indexes.delete(`${event.sessionId}\u0000${kind}\u0000${event.itemId}`);
  }
}

export function appendAgentEventBatch(current: AgentEvent[], incoming: AgentEvent[], limit = 2_000): AgentEvent[] {
  if (!incoming.length) return current;
  // Only positive ids identify durable events. Every transient provider frame
  // deliberately has id=0, so treating it as globally unique discarded all
  // but the first live item across every session.
  const ids = new Set(current.map(durableKey).filter((key): key is string => key !== undefined));
  const next = [...current];
  const mergeIndexes = new Map<string, number>();
  next.forEach((event, index) => {
    const key = mergeKey(event);
    if (key) mergeIndexes.set(key, index);
    else clearItemMergeIndexes(mergeIndexes, event);
  });
  for (const event of incoming) {
    const id = durableKey(event);
    if (id && ids.has(id)) continue;
    if (id) ids.add(id);
    const key = mergeKey(event);
    const mergeIndex = key === undefined ? undefined : mergeIndexes.get(key);
    if (mergeIndex !== undefined) {
      const previous = next[mergeIndex];
      const text = ADDITIVE_DELTAS.has(event.kind)
        ? `${previous.text ?? ""}${event.text ?? ""}`
        : event.text ?? previous.text ?? "";
      next[mergeIndex] = {
        ...event,
        text: clampText(text),
        data: { ...previous.data, ...event.data },
      };
    } else {
      if (!key) clearItemMergeIndexes(mergeIndexes, event);
      // Terminal/durable items keep their complete text. Only transient text
      // that can accumulate between terminals needs the per-item live cap.
      next.push(key && event.text ? { ...event, text: clampText(event.text) } : event);
      if (key) mergeIndexes.set(key, next.length - 1);
    }
  }
  const bounded = next.length > limit ? next.slice(-limit) : next;
  let total = 0;
  for (let index = bounded.length - 1; index >= 0; index -= 1) {
    total += bounded[index].text?.length ?? 0;
    if (total > MAX_TOTAL_EVENT_TEXT) return bounded.slice(index + 1);
  }
  return bounded;
}

/** Bounded pre-render accumulation. Calling this from the event listener keeps
 * a provider burst compact even when the browser cannot run the flush timer
 * promptly; React still receives a single batch update. */
export function queueAgentEvent(current: AgentEvent[], incoming: AgentEvent): AgentEvent[] {
  return appendAgentEventBatch(current, [incoming], MAX_PENDING_AGENT_EVENTS);
}
