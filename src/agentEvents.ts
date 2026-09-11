import type { AgentEvent } from "./types";
import { readWireKind } from "./transcript/wire";

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
export const MAX_TOTAL_EVENT_BYTES = 8 * 1024 * 1024;
const eventSizes = new WeakMap<AgentEvent, number>();
function retainedSize(event: AgentEvent): number {
  let size = eventSizes.get(event);
  if (size === undefined) { size = JSON.stringify(event).length * 2; eventSizes.set(event, size); }
  return size;
}
export const MAX_PENDING_AGENT_EVENTS = 256;

function mergeKey(event: AgentEvent): string | undefined {
  const kind = readWireKind(event.kind);
  if (!ADDITIVE_DELTAS.has(kind) && !REPLACEABLE_SNAPSHOTS.has(kind)) return undefined;
  const itemId = event.itemId || (kind === "reasoning.delta" ? "reasoning:live" : undefined);
  if (!itemId) return undefined;
  return `${event.sessionId}\u0000${kind}\u0000${itemId}`;
}

function durableKey(event: AgentEvent): string | undefined {
  return event.id > 0 ? `${event.sessionId}\u0000${event.id}` : undefined;
}

function clampText(text: string): string {
  return text.length > MAX_MERGED_EVENT_TEXT ? text.slice(-MAX_MERGED_EVENT_TEXT) : text;
}

function clearItemMergeIndexes(indexes: Map<string, number>, event: AgentEvent): void {
  const kind = readWireKind(event.kind);
  const itemId = event.itemId ?? (kind.startsWith("reasoning.") || kind.startsWith("turn.") ? "reasoning:live" : undefined);
  if (!itemId) return;
  for (const kind of [...ADDITIVE_DELTAS, ...REPLACEABLE_SNAPSHOTS]) {
    indexes.delete(`${event.sessionId}\u0000${kind}\u0000${itemId}`);
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
  // Durable events are only pushed, never spliced, so this recovers the most
  // recently arrived durable sequence even when transient merges move to tail.
  let lastDurableSeq: number | undefined;
  next.forEach((event, index) => {
    if (event.sequence > 0) lastDurableSeq = event.sequence;
    const key = mergeKey(event);
    if (key) mergeIndexes.set(key, index);
    else clearItemMergeIndexes(mergeIndexes, event);
  });
  for (const event of incoming) {
    const id = durableKey(event);
    if (id && ids.has(id)) continue;
    if (id) ids.add(id);
    if (event.sequence > 0) lastDurableSeq = event.sequence;
    const key = mergeKey(event);
    const mergeIndex = key === undefined ? undefined : mergeIndexes.get(key);
    if (key !== undefined && mergeIndex !== undefined) {
      const previous = next[mergeIndex];
      const text = ADDITIVE_DELTAS.has(readWireKind(event.kind))
        ? `${previous.text ?? ""}${event.text ?? ""}`
        : event.text ?? previous.text ?? "";
      const merged = {
        ...event,
        text: clampText(text),
        data: { ...previous.data, ...event.data },
        causalAnchor: previous.causalAnchor,
      };
      // A merge is new activity. Move it to the tail so positional eviction
      // removes the least-recently-touched item, not an actively updating one.
      next.splice(mergeIndex, 1);
      for (const [indexedKey, indexedPosition] of mergeIndexes) {
        if (indexedPosition > mergeIndex) mergeIndexes.set(indexedKey, indexedPosition - 1);
      }
      next.push(merged);
      mergeIndexes.set(key, next.length - 1);
    } else {
      if (!key) clearItemMergeIndexes(mergeIndexes, event);
      // Terminal/durable items keep their complete text. Only transient text
      // that can accumulate between terminals needs the per-item live cap.
      const stamped = key && event.sequence <= 0 && lastDurableSeq !== undefined
        ? { ...event, causalAnchor: lastDurableSeq }
        : event;
      next.push(key && stamped.text ? { ...stamped, text: clampText(stamped.text) } : stamped);
      if (key) mergeIndexes.set(key, next.length - 1);
    }
  }
  const bounded = next.length > limit ? next.slice(-limit) : next;
  let total = 0;
  let bytes = 0;
  for (let index = bounded.length - 1; index >= 0; index -= 1) {
    total += bounded[index].text?.length ?? 0;
    bytes += retainedSize(bounded[index]);
    if (total > MAX_TOTAL_EVENT_TEXT || bytes > MAX_TOTAL_EVENT_BYTES) return bounded.slice(index + 1);
  }
  return bounded;
}

/** Bounded pre-render accumulation. Calling this from the event listener keeps
 * a provider burst compact even when the browser cannot run the flush timer
 * promptly; React still receives a single batch update. */
export function queueAgentEvent(current: AgentEvent[], incoming: AgentEvent): AgentEvent[] {
  return appendAgentEventBatch(current, [incoming], MAX_PENDING_AGENT_EVENTS);
}
