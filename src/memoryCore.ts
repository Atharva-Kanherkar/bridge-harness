// Pure logic behind the Memory Core surface (issue #416). Everything here is
// deterministic and free of DOM/React so it can be unit-tested directly and so
// the constellation renders identically across mounts and screenshots — no
// `Math.random`, no wall-clock reads.
import type { MemoryCoRecallPair, MemoryRecallStats, MemoryRecord } from "./types";

/** The five visual states a memory node can carry. Identity is the state, not
 *  the color: the surface maps each to a shape/ring/dash as well as a hue. */
export type NodeState = "pinned" | "active" | "proposed" | "superseded" | "tombstoned";

/** One winner per (record) laid out on the constellation. */
export interface GraphNode {
  id: string;
  x: number;
  y: number;
  r: number;
  state: NodeState;
  record: MemoryRecord;
}

export type EdgeKind = "supersedes" | "conflict" | "corecall";
export interface GraphEdge {
  from: string;
  to: string;
  kind: EdgeKind;
  weight?: number;
}

export const NODE_MIN_R = 6;
export const NODE_MAX_R = 17;
/** Mirrors the packet budget gate in `memory_packet.rs`. */
export const PACKET_BUDGET_CHARS = 4000;
const RECALL_DAYS = 14;

/** Exactly one state per record — the storage status crossed with provenance.
 *  An active user pin reads as `pinned`; an accepted model proposal as `active`. */
export function nodeState(record: MemoryRecord): NodeState {
  switch (record.status) {
    case "proposed": return "proposed";
    case "superseded": return "superseded";
    case "deleted": return "tombstoned";
    default:
      return record.provenance === "model_proposal" ? "active" : "pinned";
  }
}

/** Radius grows with confidence and is clamped so a lone high-confidence pin
 *  never swamps the field; a null confidence sits at the mid radius. */
export function nodeRadius(confidenceBps?: number | null): number {
  if (confidenceBps == null) return Math.round((NODE_MIN_R + NODE_MAX_R) / 2);
  const ratio = Math.max(0, Math.min(1, confidenceBps / 10_000));
  return Math.round(NODE_MIN_R + ratio * (NODE_MAX_R - NODE_MIN_R));
}

/** A small, stable string hash (FNV-1a). Keeps the layout deterministic. */
function hash(text: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < text.length; i++) {
    h ^= text.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

/** Records that own a node in the graph: everything except tombstoned, which
 *  the surface sinks out rather than plotting. */
export function graphRecords(records: MemoryRecord[]): MemoryRecord[] {
  return records.filter(record => nodeState(record) !== "tombstoned");
}

/** Deterministic polar layout. State picks the ring (pinned innermost →
 *  superseded outermost); the id hash picks the angle and a small radial
 *  jitter so same-ring nodes never collide. All coordinates land inside the
 *  box with a margin for the node radius. */
export function layoutConstellation(records: MemoryRecord[], width: number, height: number): GraphNode[] {
  const nodes = graphRecords(records);
  const cx = width / 2;
  const cy = height / 2;
  const margin = NODE_MAX_R + 10;
  const maxR = Math.max(1, Math.min(width, height) / 2 - margin);
  const ringOf: Record<NodeState, number> = {
    pinned: 0.42, active: 0.64, proposed: 0.86, superseded: 0.98, tombstoned: 1,
  };
  // Rank within each ring so same-state nodes are evenly fanned around the
  // circle by the golden angle, rather than clumping where their id hashes land.
  const ringIndex = new Map<string, number>();
  const ringCounts: Record<string, number> = {};
  for (const record of nodes) {
    const key = nodeState(record);
    ringIndex.set(record.id, ringCounts[key] ?? 0);
    ringCounts[key] = (ringCounts[key] ?? 0) + 1;
  }
  return nodes.map((record, index) => {
    const h = hash(record.id);
    const rank = ringIndex.get(record.id) ?? index;
    // Golden angle by within-ring rank spreads a ring's nodes apart; the hash
    // only offsets the whole ring so rings don't align spoke-to-spoke.
    const angle = rank * 2.399963 + (h % 360) / 360 * Math.PI * 0.5 + ringOf[nodeState(record)] * 3;
    const jitter = ((h >> 12) % 100) / 100 * 0.08 - 0.04;
    const ring = Math.max(0.14, Math.min(1, ringOf[nodeState(record)] + jitter));
    const dist = ring * maxR;
    return {
      id: record.id,
      x: Math.round(cx + Math.cos(angle) * dist),
      y: Math.round(cy + Math.sin(angle) * dist),
      r: nodeRadius(record.confidenceBps),
      state: nodeState(record),
      record,
    };
  });
}

/** Lineage arrows, conflict ties, and co-recall ties — never referencing an id
 *  outside the plotted set. */
export function buildEdges(records: MemoryRecord[], coRecall: MemoryCoRecallPair[]): GraphEdge[] {
  const present = new Set(graphRecords(records).map(record => record.id));
  const edges: GraphEdge[] = [];
  // Supersession lineage.
  for (const record of records) {
    if (record.supersedes && present.has(record.id) && present.has(record.supersedes)) {
      edges.push({ from: record.supersedes, to: record.id, kind: "supersedes" });
    }
  }
  // Conflict groups: tie every pair sharing a group.
  const groups = new Map<string, string[]>();
  for (const record of records) {
    if (!record.conflictGroup || !present.has(record.id)) continue;
    const bucket = groups.get(record.conflictGroup) ?? [];
    bucket.push(record.id);
    groups.set(record.conflictGroup, bucket);
  }
  for (const bucket of groups.values()) {
    for (let i = 0; i < bucket.length; i++) {
      for (let j = i + 1; j < bucket.length; j++) {
        edges.push({ from: bucket[i], to: bucket[j], kind: "conflict" });
      }
    }
  }
  // Co-recall ties from the audit aggregation.
  for (const pair of coRecall) {
    if (present.has(pair.a) && present.has(pair.b)) {
      edges.push({ from: pair.a, to: pair.b, kind: "corecall", weight: pair.weight });
    }
  }
  return edges;
}

/** One injection of the packet: the day bucket (0 = 13 days ago … 13 = today)
 *  and the record ids that were selected together. The audit the surface reads
 *  is a list of these; `deriveRecallStats`/`deriveCoRecall` fold over them, the
 *  same shape `memory_retrieval_audits` will expose once wired protocol-first. */
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
    .filter(record => nodeState(record) === "pinned" || nodeState(record) === "active")
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

/** Pairs of records injected in the same packet, weighted by co-occurrence.
 *  This is the client-side stand-in for `memory.co_recall_pairs`. */
export function deriveCoRecall(audit: PacketInjection[]): MemoryCoRecallPair[] {
  const counts = new Map<string, number>();
  for (const injection of audit) {
    const ids = [...injection.ids].sort();
    for (let i = 0; i < ids.length; i++) {
      for (let j = i + 1; j < ids.length; j++) {
        const key = `${ids[i]} ${ids[j]}`;
        counts.set(key, (counts.get(key) ?? 0) + 1);
      }
    }
  }
  return [...counts.entries()].map(([key, weight]) => {
    const [a, b] = key.split(" ");
    return { a, b, weight };
  });
}

/** The first production caller for FTS-style search: case-insensitive substring
 *  over body+kind. Empty query returns everything; no match returns []. */
export function searchRecords(records: MemoryRecord[], query: string): MemoryRecord[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return records;
  return records.filter(record =>
    record.body.toLowerCase().includes(needle) || record.kind.toLowerCase().includes(needle));
}
