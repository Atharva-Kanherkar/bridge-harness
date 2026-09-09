import { describe, expect, it } from "vitest";
import type { UsageBucket } from "./types";
import {
  axisLabelIndices, buildChartSeries, buildUsageReport, DEFAULT_USAGE_PREFERENCES, enumeratePeriods, formatDayShort, formatHourShort, formatTokens, formatUsd, formatWindowLabel,
  hoverIndex, makeUsageWindow, microToUsdPerMtok, niceScale, readUsagePreferences, seriesPaths, summaryParams, USAGE_PREFERENCES_KEY, usdPerMtokToMicro, weakestCostSource, writeUsagePreferences,
} from "./usageReport";

function bucket(overrides: Partial<UsageBucket> & Partial<UsageBucket["totals"]> = {}): UsageBucket {
  const { uncachedInputTokens = 0, cacheReadTokens = 0, cacheWriteTokens = 0, outputTokens = 0, reasoningTokens = 0, ...rest } = overrides;
  return {
    day: "2026-09-01", hourStart: null, harness: "claude", model: "fable", records: 1, sessions: 1, costSource: "model_priced",
    costMicrousd: 0, cacheSavingsMicrousd: 0, unpricedRecords: 0,
    totals: { uncachedInputTokens, cacheReadTokens, cacheWriteTokens, outputTokens, reasoningTokens },
    ...rest,
  };
}

class MemoryStorage {
  private map = new Map<string, string>();
  getItem(key: string) { return this.map.get(key) ?? null; }
  setItem(key: string, value: string) { this.map.set(key, value); }
}

describe("makeUsageWindow", () => {
  it("uses calendar days in the requested zone, inclusive on both ends", () => {
    // 2026-03-08 03:30 UTC is still 2026-03-07 in Los Angeles, and DST starts that day.
    const now = new Date("2026-03-08T03:30:00Z");
    const window = makeUsageWindow(7, now, "America/Los_Angeles");
    expect(window).toEqual({ sinceDay: "2026-03-01", untilDay: "2026-03-07", timeZone: "America/Los_Angeles", resolution: "day" });
    expect(enumeratePeriods(window)).toHaveLength(7);
  });

  it("asks for exactly 24 minute-floored hours at hour resolution", () => {
    const window = makeUsageWindow(1, new Date("2026-09-09T14:07:42.123Z"), "UTC");
    expect(window.resolution).toBe("hour");
    expect(window.untilTime).toBe("2026-09-09T14:07:00Z");
    expect(window.sinceTime).toBe("2026-09-08T14:07:00Z");
    expect(window.sinceDay).toBe("2026-09-08");
    expect(window.untilDay).toBe("2026-09-09");
    const periods = enumeratePeriods(window);
    expect(periods).toHaveLength(24);
    expect(periods[0]).toBe("2026-09-08T14:07:00Z");
  });

  it("serialises to the wire params with nulls where the contract wants them", () => {
    const params = summaryParams(makeUsageWindow(30, new Date("2026-09-09T12:00:00Z"), "UTC"), false);
    expect(params).toEqual({ sinceDay: "2026-08-11", untilDay: "2026-09-09", timeZone: "UTC", resolution: "day", sinceTime: null, untilTime: null, includeImported: false, workspaceId: null });
  });

  it("labels the window in words", () => {
    expect(formatWindowLabel(makeUsageWindow(30, new Date("2026-09-09T12:00:00Z"), "UTC"))).toBe("Aug 11 to Sep 9");
    expect(formatWindowLabel(makeUsageWindow(1, new Date("2026-09-09T14:07:00Z"), "UTC"))).toBe("Sep 8, 2 PM to Sep 9, 2 PM");
  });
});

describe("formatting", () => {
  it("compacts tokens to three significant figures", () => {
    expect(formatTokens(804)).toBe("804");
    expect(formatTokens(80_400)).toBe("80.4K");
    expect(formatTokens(19_940_000_000)).toBe("19.9B");
    expect(formatTokens(1_000_000)).toBe("1M");
    expect(formatTokens(2_500_000)).toBe("2.5M");
  });

  it("formats micro-USD at two decimals and floors sub-cent positives", () => {
    expect(formatUsd(12_500)).toBe("$0.01");
    expect(formatUsd(1_234_567)).toBe("$1.23");
    expect(formatUsd(400)).toBe("<$0.01");
    expect(formatUsd(0)).toBe("$0.00");
  });

  it("formats period labels without parsing days through the local zone", () => {
    expect(formatDayShort("2026-08-07")).toBe("Aug 7");
    expect(formatHourShort("2026-08-11T14:00:00Z", "UTC")).toBe("2 PM");
  });

  it("round-trips USD per million tokens through micro-USD", () => {
    expect(usdPerMtokToMicro("3")).toBe(3_000_000);
    expect(usdPerMtokToMicro("0.30")).toBe(300_000);
    expect(usdPerMtokToMicro("")).toBeNull();
    expect(usdPerMtokToMicro("-1")).toBeNull();
    expect(usdPerMtokToMicro("abc")).toBeNull();
    expect(microToUsdPerMtok(3_750_000)).toBe("3.75");
    expect(microToUsdPerMtok(0)).toBe("0");
    expect(microToUsdPerMtok(null)).toBe("");
  });
});

describe("buildUsageReport", () => {
  const periods = ["2026-09-01", "2026-09-02", "2026-09-03"];
  const buckets: UsageBucket[] = [
    bucket({ day: "2026-09-01", harness: "claude", model: "fable", uncachedInputTokens: 100, cacheReadTokens: 900, outputTokens: 50, reasoningTokens: 20, costMicrousd: 600_000, cacheSavingsMicrousd: 50_000, records: 3 }),
    bucket({ day: "2026-09-03", harness: "claude", model: "fable", uncachedInputTokens: 50, outputTokens: 50, costMicrousd: 400_000, records: 2 }),
    bucket({ day: "2026-09-03", harness: "codex", model: "gpt-5.6", uncachedInputTokens: 1000, outputTokens: 500, costMicrousd: 0, costSource: "unpriced", unpricedRecords: 4, records: 4 }),
    bucket({ day: "2026-09-02", harness: "opencode", model: "kimi", costMicrousd: 0, records: 1 }),
  ];
  const report = buildUsageReport({ buckets, resolution: "day" }, periods);

  it("totals tokens without adding reasoning, and keeps unpriced tokens at zero cost", () => {
    expect(report.totals.processedTokens).toBe(100 + 900 + 50 + 50 + 50 + 1000 + 500);
    expect(report.totals.reasoningTokens).toBe(20);
    expect(report.totals.costMicrousd).toBe(1_000_000);
    expect(report.totals.unpricedRecords).toBe(4);
    expect(report.totals.records).toBe(10);
    expect(report.costSource).toBe("unpriced");
  });

  it("lists only harnesses with activity, cost first, with cross-metric shares", () => {
    expect(report.harnesses.map(entry => entry.harness)).toEqual(["claude", "codex"]);
    expect(report.harnesses[0].costShare).toBe(1);
    expect(report.harnesses[1].costShare).toBe(0);
    expect(report.harnesses[1].tokenShare).toBeCloseTo(1500 / 2650);
  });

  it("keys models by harness and model and carries the weakest provenance", () => {
    expect(report.models.map(model => `${model.harness}:${model.model}`)).toEqual(["claude:fable", "codex:gpt-5.6"]);
    expect(report.models[0].tokens).toBe(1150);
    expect(report.models[1].costSource).toBe("unpriced");
  });

  it("keeps the dense period axis so gap days chart as zero", () => {
    expect(report.periods.map(period => period.period)).toEqual(periods);
    expect(report.periods[1].tokens).toBe(0);
    expect(report.periods[2].costByHarness).toEqual({ claude: 400_000, codex: 0 });
  });

  it("keys hour buckets by their normalised hourStart", () => {
    const hourly = buildUsageReport({ buckets: [bucket({ day: "2026-09-01", hourStart: "2026-09-01T01:00:00+00:00", outputTokens: 10 })], resolution: "hour" }, ["2026-09-01T00:00:00Z", "2026-09-01T01:00:00Z"]);
    expect(hourly.periods[1].tokens).toBe(10);
  });

  it("reports the weakest source across the window", () => {
    expect(weakestCostSource([])).toBe("none");
    expect(weakestCostSource([bucket({ costSource: "provider_reported" })])).toBe("provider_reported");
    expect(weakestCostSource([bucket({ costSource: "provider_reported" }), bucket({ costSource: "model_priced" })])).toBe("model_priced");
  });

  it("orders chart series by total and never adds or removes one when the metric flips", () => {
    const cost = buildChartSeries(report, "cost");
    const tokens = buildChartSeries(report, "tokens");
    expect(cost.map(series => series.harness)).toEqual(["claude", "codex"]);
    expect(tokens.map(series => series.harness)).toEqual(["codex", "claude"]);
    expect(cost[0].values).toEqual([600_000, 0, 400_000]);
  });
});

describe("chart geometry", () => {
  it("picks 1/2/5 steps that cover the peak", () => {
    expect(niceScale(0, 4)).toEqual({ max: 0, ticks: [0] });
    expect(niceScale(7.3, 4)).toEqual({ max: 8, ticks: [0, 2, 4, 6, 8] });
    expect(niceScale(1_234, 4)).toEqual({ max: 1_500, ticks: [0, 500, 1_000, 1_500] });
  });

  it("emits finite, closed paths across the plot", () => {
    const { line, area } = seriesPaths([0, 4, 2, 8], 8);
    expect(line.startsWith("M0,260")).toBe(true);
    expect(line).not.toMatch(/NaN|Infinity/);
    expect(area.endsWith("L0,260 Z")).toBe(true);
    expect(seriesPaths([5], 10).line).toBe("M0,134 L960,134");
    expect(seriesPaths([], 10)).toEqual({ line: "", area: "" });
  });

  it("snaps hover to the nearest period and labels three axis points", () => {
    expect(hoverIndex(0.49, 30)).toBe(14);
    expect(hoverIndex(2, 30)).toBe(29);
    expect(hoverIndex(0.5, 1)).toBe(0);
    expect(axisLabelIndices(30)).toEqual([0, 14, 29]);
    expect(axisLabelIndices(1)).toEqual([0]);
  });
});

describe("preferences", () => {
  it("round-trips through storage and falls back on garbage", () => {
    const storage = new MemoryStorage();
    expect(readUsagePreferences(storage)).toEqual(DEFAULT_USAGE_PREFERENCES);
    writeUsagePreferences({ metric: "tokens", windowDays: 7, includeImported: false }, storage);
    expect(readUsagePreferences(storage)).toEqual({ metric: "tokens", windowDays: 7, includeImported: false });
    storage.setItem(USAGE_PREFERENCES_KEY, "{\"metric\":\"limits\",\"windowDays\":3}");
    expect(readUsagePreferences(storage)).toEqual(DEFAULT_USAGE_PREFERENCES);
    storage.setItem(USAGE_PREFERENCES_KEY, "not json");
    expect(readUsagePreferences(storage)).toEqual(DEFAULT_USAGE_PREFERENCES);
    expect(readUsagePreferences(undefined)).toEqual(DEFAULT_USAGE_PREFERENCES);
  });
});
