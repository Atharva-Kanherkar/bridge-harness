import { statusBucket } from "./components/sidebarChats";
import type { Session } from "./types";

export type AttentionEvent = { kind: "needs-you" | "turn-completed"; session: Session };

/**
 * Diffs two session-list snapshots (successive `BridgeState.sessions` reads)
 * into attention-worthy status transitions. `previous` is `undefined` on the
 * very first snapshot so startup never replays events for sessions that were
 * already waiting or already idle before Bridge opened.
 */
export function diffAttentionEvents(previous: Session[] | undefined, next: Session[]): AttentionEvent[] {
  if (!previous) return [];
  const previousById = new Map(previous.map(session => [session.id, session]));
  const events: AttentionEvent[] = [];
  for (const session of next) {
    const before = previousById.get(session.id);
    if (!before || before.status === session.status) continue;
    const previousBucket = statusBucket(before.status);
    const nextBucket = statusBucket(session.status);
    if (previousBucket === nextBucket) continue;
    if (nextBucket === "waiting") {
      events.push({ kind: "needs-you", session });
    } else if (previousBucket === "active") {
      events.push({ kind: "turn-completed", session });
    }
  }
  return events;
}
