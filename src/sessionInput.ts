import type { BridgeEvent } from "./types";

/** How Bridge answered a submitted message. Mirrors the protocol's closed set. */
export type InputDelivery = "started" | "steered" | "queued";

/**
 * The follow-ups a session still has waiting for its next phase boundary.
 *
 * Folded from the durable event feed rather than kept in component state: a
 * reconnect replays the same rows, so the composer tells the user the same thing
 * after a restart as it did before one. Each row's body is the queue id, which
 * is what makes the fold exact — a summary count could not say which follow-up
 * a `delivered` row retired.
 */
export function queuedFollowUps(sessionId: string, events: BridgeEvent[]): string[] {
  const waiting = new Set<string>();
  // The feed arrives newest-first; the fold only makes sense oldest-first.
  for (const event of [...events].sort((a, b) => a.id - b.id)) {
    if (event.entityId !== sessionId) continue;
    if (event.kind === "session.input.queued") waiting.add(event.body);
    else if (
      event.kind === "session.input.delivered"
      || event.kind === "session.input.discarded"
      || event.kind === "session.input.abandoned"
    ) waiting.delete(event.body);
  }
  return [...waiting];
}

/**
 * What the submit affordance should say before the user presses it.
 *
 * Read from the harness's advertised capabilities, not guessed from the
 * provider's name: a provider that cannot take input mid-turn gets its
 * follow-up queued, and the button should say so rather than implying the agent
 * is about to read it.
 */
export function activeTurnAction(capabilities: string[] | undefined): "steer" | "queue" {
  return capabilities?.includes("steering") ? "steer" : "queue";
}
