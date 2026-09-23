import { describe, expect, it } from "vitest";
import type { UsageBucket } from "./types";
import { buildUsageReport } from "./usageReport";
import {
  bucketKeys, calendarAxis, calendarWeeks, evenTicks, flowBandPath, flowLayout, harnessesByMetric, mondayIndex, orderedRange, scopeReport, squarify, stackColumns, straightPaths, treemap,
} from "./usageGeometry";

function bucket(day: string, harness: string, model: string, kinds: Partial<UsageBucket["totals"]>, costMicrousd: number, extra: Partial<UsageBucket> = {}): UsageBucket {
  return {
    day, hourStart: null, harness, model, records: 1, sessions: 1, costSource: "model_priced", costMicrousd, cacheSavingsMicrousd: 0, unpricedRecords: 0,
    totals: { uncachedInputTokens: 0, cacheReadTokens: 0, cacheWriteTokens: 0, outputTokens: 0, reasoningTokens: 0, ...kinds },
    ...extra,
  };
}

const periods = ["2026-09-01", "2026-09-02", "2026-09-03", "2026-09-04"];
const buckets: UsageBucket[] = [
  bucket("2026-09-01", "claude", "opus", { cacheReadTokens: 9_000, uncachedInputTokens: 500, outputTokens: 100, reasoningTokens: 40 }, 3_000_000),
  bucket("2026-09-02", "claude", "sonnet", { cacheReadTokens: 2_000, cacheWriteTokens: 300, outputTokens: 50 }, 900_000),
  bucket("2026-09-02", "codex", "gpt", { cacheReadTokens: 4_000, uncachedInputTokens: 700, outputTokens: 80 }, 1_200_000),
  bucket("2026-09-04", "opencode", "kimi", { cacheReadTokens: 1_000, uncachedInputTokens: 200 }, 0, { costSource: "unpriced", unpricedRecords: 1 }),
];
const summary = { buckets, resolution: "day" as const };
const report = buildUsageReport(summary, periods);

describe("straightPaths", () => {
  const geometry = { width: 300, height: 60, plotTop: 4 };

  it("joins the real points with straight segments only", () => {
    const { line, area } = straightPaths([0, 10, 5], 10, geometry);
    expect(line).toBe("M0,60 L150,4 L300,32");
    expect(line).not.toMatch(/[CQS]/);
    expect(area.endsWith("L300,60 L0,60 Z")).toBe(true);
  });

  it("handles the empty and single-point series", () => {
    expect(straightPaths([], 10, geometry)).toEqual({ line: "", area: "" });
    expect(straightPaths([5], 10, geometry).line).toBe("M0,32 L300,32");
    expect(straightPaths([5, 5], 0, geometry).line).toBe("M0,60 L300,60");
  });
});

describe("evenTicks", () => {
  it("spreads up to the target and always keeps both ends", () => {
    expect(evenTicks(30)).toEqual([0, 7, 15, 22, 29]);
    expect(evenTicks(3)).toEqual([0, 1, 2]);
    expect(evenTicks(1)).toEqual([0]);
    expect(evenTicks(0)).toEqual([]);
    expect(evenTicks(24).at(-1)).toBe(23);
  });
});

describe("stackColumns", () => {
  it("stacks harnesses in order and ends at the period total", () => {
    const order = harnessesByMetric(report, "tokens").map(entry => entry.harness);
    const columns = stackColumns(report.periods, order, "tokens");
    expect(columns).toHaveLength(periods.length);
    columns.forEach((column, index) => {
      expect(column.total).toBe(report.periods[index].tokens);
      if (column.segments.length) expect(column.segments.at(-1)!.end).toBe(column.total);
      column.segments.forEach((segment, position) => {
        expect(segment.end - segment.start).toBe(segment.value);
        if (position > 0) expect(segment.start).toBe(column.segments[position - 1].end);
      });
    });
    expect(columns[1].segments.map(segment => segment.harness)).toEqual(order.filter(harness => harness !== "opencode"));
    expect(columns[2]).toEqual({ period: "2026-09-03", total: 0, segments: [] });
  });
});

describe("squarify", () => {
  const rect = { x: 0, y: 0, w: 400, h: 300 };
  const items = [60, 30, 25, 10, 5, 0].map((value, index) => ({ item: `m${index}`, value }));
  const tiles = squarify(items, rect);

  it("drops empty items and makes every area proportional to its value", () => {
    expect(tiles.map(tile => tile.item)).toEqual(["m0", "m1", "m2", "m3", "m4"]);
    const total = 130;
    for (const tile of tiles) expect(tile.w * tile.h).toBeCloseTo((tile.value / total) * rect.w * rect.h, 6);
  });

  it("tiles the rectangle exactly, without overlap or overflow", () => {
    expect(tiles.reduce((sum, tile) => sum + tile.w * tile.h, 0)).toBeCloseTo(rect.w * rect.h, 6);
    for (const tile of tiles) {
      expect(tile.x).toBeGreaterThanOrEqual(-1e-9);
      expect(tile.y).toBeGreaterThanOrEqual(-1e-9);
      expect(tile.x + tile.w).toBeLessThanOrEqual(rect.w + 1e-9);
      expect(tile.y + tile.h).toBeLessThanOrEqual(rect.h + 1e-9);
    }
    for (const a of tiles) for (const b of tiles) {
      if (a === b) continue;
      const overlapX = Math.min(a.x + a.w, b.x + b.w) - Math.max(a.x, b.x);
      const overlapY = Math.min(a.y + a.h, b.y + b.h) - Math.max(a.y, b.y);
      expect(overlapX <= 1e-6 || overlapY <= 1e-6).toBe(true);
    }
  });

  it("returns nothing for an empty or zero-valued input", () => {
    expect(squarify([], rect)).toEqual([]);
    expect(squarify([{ item: "a", value: 0 }], rect)).toEqual([]);
  });

  it("nests models inside their harness", () => {
    const groups = treemap(report, "cost", rect);
    expect(groups.map(group => group.harness)).toEqual(["claude", "codex"]);
    const claude = groups[0];
    for (const tile of claude.tiles) {
      expect(tile.x).toBeGreaterThanOrEqual(claude.rect.x - 1e-9);
      expect(tile.x + tile.w).toBeLessThanOrEqual(claude.rect.x + claude.rect.w + 1e-9);
    }
    expect(claude.tiles.map(tile => tile.item.model)).toEqual(["opus", "sonnet"]);
  });
});

describe("flowLayout", () => {
  const sum = (values: number[]) => values.reduce((total, value) => total + value, 0);

  it("runs harness → model → token kind in token mode and conserves every node", () => {
    const layout = flowLayout(report, "tokens", { height: 300, padding: 8, minNode: 3 });
    expect(layout.columns).toBe(3);
    expect(layout.nodes.filter(node => node.column === 2).map(node => node.label)).toEqual(["Cache read", "Cache write", "Uncached input", "Output"]);
    for (const node of layout.nodes) {
      const incoming = layout.links.filter(link => link.target === node.id);
      const outgoing = layout.links.filter(link => link.source === node.id);
      if (node.column > 0) {
        expect(sum(incoming.map(link => link.value))).toBe(node.value);
        expect(sum(incoming.map(link => link.targetHeight))).toBeCloseTo(node.height, 6);
      }
      if (node.column < 2) {
        expect(sum(outgoing.map(link => link.value))).toBe(node.value);
        expect(sum(outgoing.map(link => link.sourceHeight))).toBeCloseTo(node.height, 6);
      }
    }
    const cacheRead = layout.nodes.find(node => node.kind === "cacheReadTokens")!;
    expect(cacheRead.value).toBe(report.totals.cacheReadTokens);
    expect(sum(layout.nodes.filter(node => node.column === 2).map(node => node.value))).toBe(report.totals.processedTokens);
  });

  it("stops at the model in cost mode and leaves out zero-cost nodes", () => {
    const layout = flowLayout(report, "cost", { height: 300, padding: 8, minNode: 3 });
    expect(layout.columns).toBe(2);
    expect(layout.nodes.some(node => node.kind)).toBe(false);
    expect(layout.nodes.some(node => node.harness === "opencode")).toBe(false);
    expect(sum(layout.nodes.filter(node => node.column === 1).map(node => node.value))).toBe(report.totals.costMicrousd);
  });

  it("keeps every node inside the drawing and draws closed bands", () => {
    const layout = flowLayout(report, "tokens", { height: 200, padding: 10, minNode: 3 });
    for (const node of layout.nodes) {
      expect(node.y).toBeGreaterThanOrEqual(-1e-9);
      expect(node.y + node.height).toBeLessThanOrEqual(layout.height + 1e-9);
      expect(node.height).toBeGreaterThanOrEqual(3);
    }
    const path = flowBandPath(layout.links[0], 10, 200);
    expect(path.startsWith("M10,")).toBe(true);
    expect(path.endsWith("Z")).toBe(true);
    expect(path).not.toContain("NaN");
  });

  it("is empty for an empty window", () => {
    const empty = buildUsageReport({ buckets: [], resolution: "day" }, periods);
    expect(flowLayout(empty, "tokens", { height: 300, padding: 8, minNode: 3 }).nodes).toEqual([]);
  });
});

describe("calendar", () => {
  it("lays days out Monday-first with empty leading and trailing slots", () => {
    expect(mondayIndex("2026-09-01")).toBe(1);
    expect(mondayIndex("2026-09-07")).toBe(0);
    const days = Array.from({ length: 10 }, (_, index) => `2026-09-${String(index + 1).padStart(2, "0")}`);
    const weeks = calendarWeeks(days);
    expect(weeks).toEqual([
      [null, 0, 1, 2, 3, 4, 5],
      [6, 7, 8, 9, null, null, null],
    ]);
    expect(calendarWeeks([])).toEqual([]);
  });

  it("gives out-of-window buckets their own cells, filling days densely", () => {
    const days = ["2026-09-01", "2026-09-02"];
    expect(calendarAxis(days, ["2026-09-01"], "day")).toEqual(days);
    expect(calendarAxis(days, ["2026-08-31", "2026-09-01"], "day")).toEqual(["2026-08-31", "2026-09-01", "2026-09-02"]);
    expect(calendarAxis(days, ["2026-08-29"], "day")).toEqual(["2026-08-29", "2026-08-30", "2026-08-31", "2026-09-01", "2026-09-02"]);
    const hours = ["2026-09-01T10:00:00Z", "2026-09-01T11:00:00Z"];
    expect(calendarAxis(hours, ["2026-09-01T09:00:00Z", "2026-09-01T11:00:00Z"], "hour")).toEqual(["2026-09-01T09:00:00Z", ...hours]);
  });

  it("keeps the whole-window total equal to every visible cell when a bucket falls outside the window", () => {
    // A zone-edge bucket: buildUsageReport keeps it, so the calendar must show it.
    const edge = { buckets: [bucket("2026-08-31", "claude", "opus", { uncachedInputTokens: 900 }, 0), bucket("2026-09-01", "claude", "opus", { uncachedInputTokens: 100 }, 0)], resolution: "day" as const };
    const days = ["2026-09-01", "2026-09-02"];
    const axis = calendarAxis(days, bucketKeys(edge), "day");
    const whole = scopeReport(edge, axis, [0, axis.length - 1]).report;
    expect(whole.totals.processedTokens).toBe(1000);
    expect(whole.totals.processedTokens).toBe(buildUsageReport(edge, days).totals.processedTokens);
    expect(whole.periods.map(period => period.period)).toEqual(axis);
    expect(whole.periods.reduce((sum, period) => sum + period.tokens, 0)).toBe(1000);
  });

  it("orders a range whichever end was picked first", () => {
    expect(orderedRange(5, 2)).toEqual([2, 5]);
    expect(orderedRange(2, 5)).toEqual([2, 5]);
  });

  it("scopes a report to a range by rebuilding from the buckets in it", () => {
    const scoped = scopeReport(summary, periods, [1, 2]);
    expect(scoped.periods).toEqual(["2026-09-02", "2026-09-03"]);
    expect(scoped.report).toEqual(buildUsageReport({ buckets: buckets.filter(entry => entry.day === "2026-09-02"), resolution: "day" }, ["2026-09-02", "2026-09-03"]));
    expect(scoped.report.totals.processedTokens).toBe(2_000 + 300 + 50 + 4_000 + 700 + 80);
    expect(scopeReport(summary, periods, null).report).toEqual(report);
  });
});
