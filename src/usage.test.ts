import { describe, expect, it } from "vitest";
import type { AgentEvent } from "./types";
import type { Session, UsageLedgerRow } from "./types";
import { buildCacheDiagnostics, buildUsageHistory, clampPercent, contextPressure, extractUsageSnapshot, formatReset, latestUsageSnapshot, MAX_CACHE_DIAGNOSTIC_ROWS, projectUsageExhaustion, windowLabel } from "./usage";

function event(kind: string, data: Record<string, unknown>, sequence = 1): AgentEvent {
  return { id: sequence, sessionId: "s1", sequence, protocolVersion: 1, kind, itemId: null, role: null, status: null, title: null, text: null, data, providerMeta: {}, createdAt: new Date().toISOString() };
}

function ledger(overrides: Partial<UsageLedgerRow> = {}): UsageLedgerRow {
  return {
    id: 1, workspaceId: "w", sessionId: "s1", turnId: "turn-1", inputTokens: 0, outputTokens: 0,
    cacheReadTokens: 0, cacheWriteTokens: 0, uncachedInputTokens: 0, contextPercent: null,
    capabilityUnits: 0, runtimeMs: 1, costMicrousd: null, costSource: null,
    stablePrefixId: "prefix-1", stablePrefixHash: "hash-1", promptSchemaVersion: 1,
    prefixTokenEstimate: 100, harness: "codex", model: "gpt-5", role: "worker:implementation",
    taskFamily: "implementation", restorationMode: "fresh", crossHarnessReuse: "same_harness",
    source: "provider.codex", createdAt: "2026-07-16T10:00:00Z", ...overrides,
  };
}

describe("extractUsageSnapshot", () => {
  it("parses Codex-style rate_limits windows (the /status data)", () => {
    const snapshot = extractUsageSnapshot({
      rate_limits: {
        primary: { used_percent: 12.4, window_minutes: 300, resets_in_seconds: 3600 },
        secondary: { used_percent: 3.1, window_minutes: 10080, resets_in_seconds: 200000 },
      },
    });
    expect(snapshot).not.toBeNull();
    expect(snapshot!.windows).toHaveLength(2);
    // Shortest window first.
    expect(snapshot!.windows[0]).toMatchObject({ label: "5h", usedPercent: 12.4, resetsInSeconds: 3600 });
    expect(snapshot!.windows[0].source).toBe("reported");
    expect(snapshot!.source).toBe("reported");
    expect(snapshot!.windows[1]).toMatchObject({ label: "Weekly", usedPercent: 3.1 });
  });

  it("parses the real Codex account/rateLimits/read response shape", () => {
    const resetsAt = Math.floor(Date.now() / 1000) + 7200; // absolute epoch seconds, 2h out
    const snapshot = extractUsageSnapshot({
      rateLimits: {
        planType: "pro",
        primary: { usedPercent: 41, windowDurationMins: 300, resetsAt },
        secondary: { usedPercent: 9, windowDurationMins: 10080, resetsAt: resetsAt + 500000 },
      },
      rateLimitsByLimitId: { codex: { primary: { usedPercent: 41, windowDurationMins: 300, resetsAt } } },
    });
    expect(snapshot).not.toBeNull();
    expect(snapshot!.windows[0]).toMatchObject({ label: "5h", usedPercent: 41 });
    // resetsAt was converted to a forward-looking countdown (~2h).
    expect(snapshot!.windows[0].resetsInSeconds).toBeGreaterThan(7000);
    expect(snapshot!.windows[0].resetsInSeconds).toBeLessThanOrEqual(7200);
    expect(snapshot!.windows[1]).toMatchObject({ label: "Weekly", usedPercent: 9 });
  });

  it("finds rate_limits nested under a usage container", () => {
    const snapshot = extractUsageSnapshot({ usage: { rateLimits: { primary: { usedPercent: 88, windowMinutes: 60 } } } });
    expect(snapshot!.windows[0]).toMatchObject({ label: "1h", usedPercent: 88 });
  });

  it("parses Claude-style tokens and cost without windows", () => {
    const snapshot = extractUsageSnapshot({ usage: { input_tokens: 1200, output_tokens: 300 }, total_cost_usd: 0.0421 });
    expect(snapshot).not.toBeNull();
    expect(snapshot!.windows).toHaveLength(0);
    expect(snapshot!.totalTokens).toBe(1500);
    expect(snapshot!.costUsd).toBeCloseTo(0.0421);
  });

  it("parses Claude /usage windows with explicit labels and reset text", () => {
    const snapshot = extractUsageSnapshot({
      rateLimits: {
        "1_session": { label: "Session", usedPercent: 38, resetsLabel: "resets Jul 13 at 7:09pm" },
        "2_week": { label: "Week", usedPercent: 60, resetsLabel: "resets Jul 14 at 3:29am" },
        "3_fable": { label: "Fable", usedPercent: 81, resetsLabel: "resets Jul 14 at 3:29am" },
      },
    });
    expect(snapshot!.windows.map(w => w.label)).toEqual(["Session", "Week", "Fable"]);
    expect(snapshot!.windows[0]).toMatchObject({ usedPercent: 38, resetsLabel: "resets Jul 13 at 7:09pm" });
    expect(snapshot!.windows[2].usedPercent).toBe(81);
  });

  it("reads context percent when present", () => {
    expect(extractUsageSnapshot({ context_percent: 63 })!.contextPercent).toBe(63);
  });

  it("preserves model metadata and explicit measured provenance", () => {
    const snapshot = extractUsageSnapshot({ contextPercent: 42, model: "gpt-5", metricSource: "measured" });
    expect(snapshot).toMatchObject({ model: "gpt-5", source: "measured" });
  });

  it("returns null when there is no real signal", () => {
    expect(extractUsageSnapshot({})).toBeNull();
    expect(extractUsageSnapshot({ foo: "bar" })).toBeNull();
    expect(extractUsageSnapshot(null)).toBeNull();
  });
});

describe("contextPressure", () => {
  it("returns explainable states at the locked thresholds", () => {
    expect(contextPressure().level).toBe("unknown");
    expect(contextPressure(59).level).toBe("healthy");
    expect(contextPressure(60)).toMatchObject({ level: "elevated", percent: 60 });
    expect(contextPressure(75).explanation).toContain("75%");
    expect(contextPressure(90).level).toBe("critical");
    expect(contextPressure().explanation).toContain("recorded");
  });
});

describe("clampPercent", () => {
  it("clamps provider percentages to the display domain", () => {
    expect(clampPercent(-5)).toBe(0);
    expect(clampPercent(42)).toBe(42);
    expect(clampPercent(105)).toBe(100);
  });
});

describe("projectUsageExhaustion", () => {
  it("requires enough history over a meaningful time span", () => {
    expect(projectUsageExhaustion([
      { usedPercent: 70, capturedAt: "2026-07-16T10:00:00Z" },
      { usedPercent: 80, capturedAt: "2026-07-16T10:10:00Z" },
    ])).toBeNull();
  });

  it("returns an estimated, explainable alert inside the horizon", () => {
    const projection = projectUsageExhaustion([
      { usedPercent: 70, capturedAt: "2026-07-16T10:00:00Z" },
      { usedPercent: 75, capturedAt: "2026-07-16T10:05:00Z" },
      { usedPercent: 80, capturedAt: "2026-07-16T10:10:00Z" },
    ]);
    expect(projection).toMatchObject({ source: "estimated", hoursRemaining: 0.3 });
    expect(projection!.explanation).toContain("3 samples");
  });

  it("clamps out-of-range samples before calculating the trend", () => {
    const projection = projectUsageExhaustion([
      { usedPercent: -100, capturedAt: "2026-07-16T10:00:00Z" },
      { usedPercent: 25, capturedAt: "2026-07-16T10:05:00Z" },
      { usedPercent: 50, capturedAt: "2026-07-16T10:10:00Z" },
    ]);
    expect(projection?.hoursRemaining).toBe(0.2);
  });

  it("does not invent exhaustion for flat or distant trends", () => {
    expect(projectUsageExhaustion([
      { usedPercent: 10, capturedAt: "2026-07-16T10:00:00Z" },
      { usedPercent: 10, capturedAt: "2026-07-16T10:05:00Z" },
      { usedPercent: 10, capturedAt: "2026-07-16T10:10:00Z" },
    ])).toBeNull();
  });
});

describe("buildUsageHistory", () => {
  it("ties newest-first records to work units, harnesses, models, outcomes, and sources", () => {
    const session: Session = { id: "s1", workspaceId: "w", harness: "codex", label: "Worker", status: "completed", startedAt: null, endedAt: null, contextPercent: 72, usagePercent: null, metricSource: "reported", model: "gpt-5", restorationMode: "fresh" };
    const row = (id: number, source: string, createdAt: string): UsageLedgerRow => ({ id, workspaceId: "w", sessionId: "s1", turnId: `turn-${id}`, inputTokens: 10, outputTokens: 5, cacheReadTokens: null, cacheWriteTokens: null, uncachedInputTokens: 10, contextPercent: 72, capabilityUnits: 0, runtimeMs: 1, costMicrousd: null, costSource: null, stablePrefixId: null, stablePrefixHash: null, promptSchemaVersion: null, prefixTokenEstimate: null, harness: null, model: null, role: null, taskFamily: null, restorationMode: null, crossHarnessReuse: null, source, createdAt });
    const history = buildUsageHistory([
      row(1, "provider.codex", "2026-07-16T10:00:00Z"),
      row(2, "policy.spawn.strong", "2026-07-16T11:00:00Z"),
    ], [session]);
    expect(history[0]).toMatchObject({ workUnit: "turn-2", harness: "codex", model: "gpt-5", outcome: "completed", source: "measured", totalTokens: 15 });
    expect(history[1].source).toBe("reported");
  });
});

describe("buildCacheDiagnostics", () => {
  it("groups cache tokens by prompt and routing dimensions with truthful ratios", () => {
    const diagnostics = buildCacheDiagnostics([
      ledger({ id: 1, cacheReadTokens: 80, cacheWriteTokens: 20, uncachedInputTokens: 100, costMicrousd: 1_000, costSource: "provider_reported" }),
      ledger({ id: 2, cacheReadTokens: 40, cacheWriteTokens: 0, uncachedInputTokens: 60, costMicrousd: null, costSource: null }),
      ledger({ id: 3, restorationMode: "native", cacheReadTokens: 10, cacheWriteTokens: 0, uncachedInputTokens: 10 }),
      ledger({ id: 4, source: "policy.spawn.standard", cacheReadTokens: 999 }),
    ]);
    expect(diagnostics).toHaveLength(2);
    expect(diagnostics[0]).toMatchObject({
      harness: "codex", model: "gpt-5", role: "worker:implementation", taskFamily: "implementation",
      restorationMode: "fresh", stablePrefixId: "prefix-1", promptSchemaVersion: 1,
      cacheReadTokens: 120, cacheWriteTokens: 20, uncachedInputTokens: 160,
      observations: 2, writeAmortization: 6, costCoverage: "partial", reportedCostMicrousd: 1_000,
      costSources: ["provider_reported"], crossHarnessReuse: ["same_harness"],
    });
    expect(diagnostics[0].cacheHitRatio).toBeCloseTo(0.4);
    expect(diagnostics[1].restorationMode).toBe("native");
  });

  it("keeps cost unknown and never invents savings without provider pricing", () => {
    const [diagnostic] = buildCacheDiagnostics([ledger({ costMicrousd: null, costSource: null })]);
    expect(diagnostic.costCoverage).toBe("unknown");
    expect(diagnostic.reportedCostMicrousd).toBeUndefined();
    expect("estimatedSavingsMicrousd" in diagnostic).toBe(false);
  });

  it("leaves ratios undefined for missing metrics and zero denominators", () => {
    const [missing] = buildCacheDiagnostics([ledger({ cacheReadTokens: null, cacheWriteTokens: null, uncachedInputTokens: null })]);
    const [zero] = buildCacheDiagnostics([ledger({ cacheReadTokens: 0, cacheWriteTokens: 0, uncachedInputTokens: 0 })]);
    expect(missing.cacheHitRatio).toBeUndefined();
    expect(missing.writeAmortization).toBeUndefined();
    expect(zero.cacheHitRatio).toBeUndefined();
    expect(zero.writeAmortization).toBeUndefined();
  });

  it("bounds aggregation to the most recent provider rows", () => {
    const rows = Array.from({ length: MAX_CACHE_DIAGNOSTIC_ROWS + 1 }, (_, index) => ledger({
      id: index + 1,
      cacheReadTokens: index === 0 ? 10_000 : 1,
    }));
    const [diagnostic] = buildCacheDiagnostics(rows);
    expect(diagnostic.observations).toBe(MAX_CACHE_DIAGNOSTIC_ROWS);
    expect(diagnostic.cacheReadTokens).toBe(MAX_CACHE_DIAGNOSTIC_ROWS);
  });
});

describe("windowLabel", () => {
  it("maps known window sizes", () => {
    expect(windowLabel("primary", 300)).toBe("5h");
    expect(windowLabel("secondary", 10080)).toBe("Weekly");
    expect(windowLabel("x", 1440)).toBe("Daily");
    expect(windowLabel("x", 30)).toBe("30m");
  });
  it("falls back to the id when the window size is unknown", () => {
    expect(windowLabel("primary")).toBe("Primary");
  });
});

describe("formatReset", () => {
  it("formats countdowns", () => {
    expect(formatReset(3660)).toBe("resets in 1h 1m");
    expect(formatReset(120)).toBe("resets in 2m");
    expect(formatReset(90000)).toBe("resets in 1d 1h");
    expect(formatReset(0)).toBeUndefined();
    expect(formatReset(undefined)).toBeUndefined();
  });
});

describe("latestUsageSnapshot", () => {
  it("uses the most recent usage.updated event that carries a signal", () => {
    const events = [
      event("usage.updated", { rate_limits: { primary: { used_percent: 5, window_minutes: 300 } } }, 1),
      event("message.completed", { text: "hi" }, 2),
      event("usage.updated", { rate_limits: { primary: { used_percent: 42, window_minutes: 300 } } }, 3),
    ];
    expect(latestUsageSnapshot(events)!.windows[0].usedPercent).toBe(42);
  });
  it("returns null with no usage events", () => {
    expect(latestUsageSnapshot([event("message.completed", { text: "hi" })])).toBeNull();
  });
});
