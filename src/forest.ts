import type { SessionForestSnapshot } from "./types";

// Heartbeats legitimately change worker runtime data every few seconds. Keep
// the durable entry array referentially stable when its contents did not
// change, so a heartbeat does not re-project the entire conversation. The
// forest is append-only, so equal length plus an equal last entry means an
// equal array — no stringify of the history is ever needed.
export function mergeForestSnapshot(
  current: SessionForestSnapshot | undefined,
  next: SessionForestSnapshot,
): SessionForestSnapshot {
  if (!current) return next;
  const last = current.entries[current.entries.length - 1];
  const nextLast = next.entries[next.entries.length - 1];
  const entriesUnchanged =
    current.entries.length === next.entries.length &&
    last?.id === nextLast?.id &&
    last?.sequence === nextLast?.sequence;
  return entriesUnchanged ? { ...next, entries: current.entries } : next;
}
