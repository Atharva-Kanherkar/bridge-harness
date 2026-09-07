// Contract: testing/feat-memory-unified-redesign.md — the pure logic behind
// the Memory surface's recall analytics.
import { describe, expect, it } from "vitest";
import {
  deriveRecallStats,
  PACKET_BUDGET_CHARS,
  recordState,
  searchRecords,
  type PacketInjection,
} from "./memoryStats";
import type { MemoryRecord } from "./types";

const rec = (id: string, over: Partial<MemoryRecord> = {}): MemoryRecord => ({
  id, scopeKey: "account:local", kind: "fact", body: `body ${id}`,
  provenance: "user_explicit", status: "active",
  validFrom: "2026-08-24T00:00:00Z", createdAt: "2026-08-24T00:00:00Z", updatedAt: "2026-08-24T00:00:00Z",
  ...over,
});

describe("recordState", () => {
  it("crosses status with provenance", () => {
    expect(recordState(rec("a"))).toBe("pinned");
    expect(recordState(rec("b", { provenance: "model_proposal" }))).toBe("active");
    expect(recordState(rec("c", { status: "proposed", provenance: "model_proposal" }))).toBe("proposed");
    expect(recordState(rec("d", { status: "superseded" }))).toBe("superseded");
    expect(recordState(rec("e", { status: "deleted" }))).toBe("tombstoned");
  });
});

describe("deriveRecallStats", () => {
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
  it("counts only pinned and active bodies against the budget", () => {
    const stats = deriveRecallStats([
      rec("live", { body: "ten chars!" }),
      rec("gone", { status: "superseded", body: "should not count" }),
    ], []);
    expect(stats.budgetCharsUsed).toBe(10);
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
