// Contract: testing/feat-memory-core.md — the pure logic behind the surface.
import { describe, expect, it } from "vitest";
import {
  buildEdges,
  deriveCoRecall,
  deriveRecallStats,
  layoutConstellation,
  nodeRadius,
  nodeState,
  NODE_MAX_R,
  NODE_MIN_R,
  PACKET_BUDGET_CHARS,
  searchRecords,
  type PacketInjection,
} from "./memoryCore";
import type { MemoryRecord } from "./types";

const rec = (id: string, over: Partial<MemoryRecord> = {}): MemoryRecord => ({
  id, scopeKey: "account:local", kind: "fact", body: `body ${id}`,
  provenance: "user_explicit", status: "active",
  validFrom: "2026-08-24T00:00:00Z", createdAt: "2026-08-24T00:00:00Z", updatedAt: "2026-08-24T00:00:00Z",
  ...over,
});

describe("nodeState", () => {
  it("crosses status with provenance", () => {
    expect(nodeState(rec("a"))).toBe("pinned");
    expect(nodeState(rec("b", { provenance: "model_proposal" }))).toBe("active");
    expect(nodeState(rec("c", { status: "proposed", provenance: "model_proposal" }))).toBe("proposed");
    expect(nodeState(rec("d", { status: "superseded" }))).toBe("superseded");
    expect(nodeState(rec("e", { status: "deleted" }))).toBe("tombstoned");
  });
});

describe("nodeRadius", () => {
  it("is monotonic and clamped, mid for null", () => {
    expect(nodeRadius(0)).toBe(NODE_MIN_R);
    expect(nodeRadius(10_000)).toBe(NODE_MAX_R);
    expect(nodeRadius(5000)).toBeGreaterThan(nodeRadius(1000));
    expect(nodeRadius(null)).toBe(Math.round((NODE_MIN_R + NODE_MAX_R) / 2));
    expect(Number.isNaN(nodeRadius(undefined))).toBe(false);
  });
});

describe("layoutConstellation", () => {
  const records = [rec("a"), rec("b", { status: "proposed", provenance: "model_proposal" }), rec("c", { status: "superseded" }), rec("z", { status: "deleted" })];
  it("is deterministic and omits tombstones", () => {
    const one = layoutConstellation(records, 600, 400);
    const two = layoutConstellation(records, 600, 400);
    expect(one).toEqual(two);
    expect(one.map(node => node.id)).not.toContain("z");
  });
  it("keeps every node inside the box and non-overlapping", () => {
    const nodes = layoutConstellation(records, 600, 400);
    for (const node of nodes) {
      expect(node.x).toBeGreaterThanOrEqual(0);
      expect(node.x).toBeLessThanOrEqual(600);
      expect(node.y).toBeGreaterThanOrEqual(0);
      expect(node.y).toBeLessThanOrEqual(400);
    }
    const points = new Set(nodes.map(node => `${node.x},${node.y}`));
    expect(points.size).toBe(nodes.length);
  });
});

describe("buildEdges", () => {
  it("emits lineage, conflict, and co-recall edges within the set only", () => {
    const records = [
      rec("old", { status: "superseded", conflictGroup: "g" }),
      rec("new", { supersedes: "old", conflictGroup: "g" }),
      rec("other", { conflictGroup: "g" }),
    ];
    const edges = buildEdges(records, [{ a: "new", b: "other", weight: 3 }, { a: "new", b: "ghost", weight: 9 }]);
    expect(edges).toContainEqual({ from: "old", to: "new", kind: "supersedes" });
    expect(edges.filter(edge => edge.kind === "conflict")).toHaveLength(3); // 3 choose 2
    expect(edges).toContainEqual({ from: "new", to: "other", kind: "corecall", weight: 3 });
    expect(edges.some(edge => edge.from === "ghost" || edge.to === "ghost")).toBe(false);
  });
});

describe("deriveRecallStats / deriveCoRecall", () => {
  const records = [rec("a"), rec("b")];
  const audit: PacketInjection[] = [
    { day: 13, ids: ["a", "b"] },
    { day: 13, ids: ["a"] },
    { day: 12, ids: ["a", "b"] },
  ];
  it("aggregates counts, ratios, and budget", () => {
    const stats = deriveRecallStats(records, audit);
    const a = stats.perRecord.find(stat => stat.id === "a")!;
    expect(a.recalls).toBe(3);
    expect(a.lastRecalledDay).toBe(13);
    expect(a.inPacketRatio).toBeCloseTo(1);
    expect(stats.injectionsPerDay[13]).toBe(2);
    expect(stats.budgetCharsMax).toBe(PACKET_BUDGET_CHARS);
    expect(stats.budgetCharsUsed).toBeLessThanOrEqual(PACKET_BUDGET_CHARS);
  });
  it("handles an empty audit without NaN", () => {
    const stats = deriveRecallStats(records, []);
    expect(stats.perRecord.every(stat => stat.recalls === 0 && stat.inPacketRatio === 0)).toBe(true);
  });
  it("weights co-recall pairs by co-occurrence", () => {
    const pairs = deriveCoRecall(audit);
    expect(pairs).toContainEqual({ a: "a", b: "b", weight: 2 });
  });
});

describe("searchRecords", () => {
  const records = [rec("a", { body: "Prefers Tailwind", kind: "preference" }), rec("b", { body: "Works in IST", kind: "fact" })];
  it("matches body and kind case-insensitively; empty returns all", () => {
    expect(searchRecords(records, "")).toHaveLength(2);
    expect(searchRecords(records, "tailwind").map(record => record.id)).toEqual(["a"]);
    expect(searchRecords(records, "FACT").map(record => record.id)).toEqual(["b"]);
    expect(searchRecords(records, "zzz")).toEqual([]);
  });
});
