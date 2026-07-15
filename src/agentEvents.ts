import type { AgentEvent } from "./types";

const MERGEABLE_DELTAS = new Set([
  "message.delta",
  "reasoning.delta",
  "command.output_delta",
  "diff.delta",
  "tool.progress",
]);

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
      next[next.length - 1] = {
        ...event,
        text: `${previous.text ?? ""}${event.text ?? ""}`,
        data: { ...previous.data, ...event.data },
      };
    } else {
      next.push(event);
    }
  }
  return next.length > limit ? next.slice(-limit) : next;
}
