import type { SessionForestSnapshot } from "./types";

// Forest polling remains a safety net for durable history and worker state,
// but identical snapshots should not invalidate the entire conversation tree.
export function forestSnapshotKey(snapshot: SessionForestSnapshot): string {
  return JSON.stringify(snapshot);
}
