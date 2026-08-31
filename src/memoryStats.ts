// Pure logic behind the Memory surface's recall analytics. Everything here is
// deterministic and free of DOM/React so it can be unit-tested directly — no
// `Math.random`, no wall-clock reads.
import type { MemoryRecallStats, MemoryRecord } from "./types";

/** The five lifecycle states a memory record can carry — the storage status
 *  crossed with provenance. An active user pin reads as `pinned`; an accepted
 *  model proposal as `active`. */
export type MemoryRecordState = "pinned" | "active" | "proposed" | "superseded" | "tombstoned";

/** Mirrors the packet budget gate in `memory_packet.rs`. */
export const PACKET_BUDGET_CHARS = 4000;
const RECALL_DAYS = 14;

/** Exactly one state per record. */
export function recordState(record: MemoryRecord): MemoryRecordState {
  switch (record.status) {
    case "proposed": return "proposed";
    case "superseded": return "superseded";
    case "deleted": return "tombstoned";
    default:
      return record.provenance === "model_proposal" ? "active" : "pinned";
  }
}

/** One injection of the packet: the day bucket (0 = 13 days ago … 13 = today)
 *  and the record ids that were selected together. The audit the surface reads
 *  is a list of these; `deriveRecallStats` folds over them, the same shape
 *  `memory_retrieval_audits` will expose once wired protocol-first. */
export interface PacketInjection {
  day: number;
  ids: string[];
}

/** Fold the audit into per-record recall counts, a 14-day series, injections
 *  per day, and packet-budget utilization. Empty audit → all zeros, never NaN. */
export function deriveRecallStats(records: MemoryRecord[], audit: PacketInjection[]): MemoryRecallStats {
  const perRecord = new Map<string, { recalls: number; lastDay: number; daily: number[] }>();
  for (const record of records) {
    perRecord.set(record.id, { recalls: 0, lastDay: -1, daily: Array<number>(RECALL_DAYS).fill(0) });
  }
  const injectionsPerDay = Array<number>(RECALL_DAYS).fill(0);
  const injectionCount = audit.length;
  for (const injection of audit) {
    const day = Math.max(0, Math.min(RECALL_DAYS - 1, injection.day));
    injectionsPerDay[day] += 1;
    for (const id of injection.ids) {
      const stat = perRecord.get(id);
      if (!stat) continue;
      stat.recalls += 1;
      stat.daily[day] += 1;
      if (day > stat.lastDay) stat.lastDay = day;
    }
  }
  const budgetCharsUsed = records
    .filter(record => recordState(record) === "pinned" || recordState(record) === "active")
    .reduce((sum, record) => sum + [...record.body.trim()].length, 0);
  return {
    perRecord: [...perRecord.entries()].map(([id, stat]) => ({
      id,
      recalls: stat.recalls,
      lastRecalledDay: stat.lastDay,
      inPacketRatio: injectionCount === 0 ? 0 : stat.recalls / injectionCount,
      daily: stat.daily,
    })),
    injectionsPerDay,
    budgetCharsUsed: Math.min(budgetCharsUsed, PACKET_BUDGET_CHARS),
    budgetCharsMax: PACKET_BUDGET_CHARS,
  };
}

/** The first production caller for FTS-style search: case-insensitive substring
 *  over body+kind. Empty query returns everything; no match returns []. */
export function searchRecords(records: MemoryRecord[], query: string): MemoryRecord[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return records;
  return records.filter(record =>
    record.body.toLowerCase().includes(needle) || record.kind.toLowerCase().includes(needle));
}
