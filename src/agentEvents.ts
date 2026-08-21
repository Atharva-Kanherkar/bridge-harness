import type { AgentEvent } from "./types";

const MERGEABLE_DELTAS = new Set([
  "message.delta",
  "reasoning.delta",
  "command.output_delta",
  "diff.delta",
  "tool.progress",
]);

// The item cap alone bounded nothing: one merged command-output item could
// grow without limit. Merged text keeps its tail — the end of a stream is
// what the user is watching — and the batch as a whole keeps the newest
// events that fit the total text budget. Durable history is unaffected; these
// are the live, transient frames only.
export const MAX_MERGED_EVENT_TEXT = 1_000_000;
export const MAX_TOTAL_EVENT_TEXT = 8_000_000;

export function appendAgentEventBatch(current: AgentEvent[], incoming: AgentEvent[], limit = 2_000): AgentEvent[] {
  if (!incoming.length) return current;
  const ids = new Set(current.map(event => event.id));
  const next = [...current];
  for (const event of incoming) {
    if (ids.has(event.id)) continue;
    ids.add(event.id);
    const previous = next[next.length - 1];
    if (
      previous
      && MERGEABLE_DELTAS.has(event.kind)
      && previous.kind === event.kind
      && previous.sessionId === event.sessionId
      && previous.itemId === event.itemId
    ) {
      const text = `${previous.text ?? ""}${event.text ?? ""}`;
      next[next.length - 1] = {
        ...event,
        text: text.length > MAX_MERGED_EVENT_TEXT ? text.slice(-MAX_MERGED_EVENT_TEXT) : text,
        data: { ...previous.data, ...event.data },
      };
    } else {
      next.push(event);
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
