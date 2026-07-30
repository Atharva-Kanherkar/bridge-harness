import type { SessionForestSnapshot } from "./types";

// Forest polling remains a safety net for durable history and worker state,
// but identical snapshots should not invalidate the entire conversation tree.
export function forestSnapshotKey(snapshot: SessionForestSnapshot): string {
  return JSON.stringify(snapshot);
}

// Heartbeats legitimately change worker runtime data every few seconds. Keep
// the durable entry array referentially stable when its contents did not
// change, so a heartbeat does not re-project the entire conversation.
export function mergeForestSnapshot(
  current: SessionForestSnapshot | undefined,
  next: SessionForestSnapshot,
): SessionForestSnapshot {
  if (!current || JSON.stringify(current.entries) !== JSON.stringify(next.entries)) return next;
  return { ...next, entries: current.entries };
}
