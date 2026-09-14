import { describe, expect, it } from "vitest";
import type { AgentEvent } from "./types";
import { asWireKind } from "./transcript/wire";
import { clampPercent, contextPressure, extractUsageSnapshot, formatReset, latestUsageSnapshot, windowLabel } from "./usage";

function event(kind: string, data: Record<string, unknown>, sequence = 1): AgentEvent {
  return { id: sequence, sessionId: "s1", sequence, protocolVersion: 1, kind: asWireKind(kind), itemId: null, role: null, status: null, title: null, text: null, data, providerMeta: {}, createdAt: new Date().toISOString() };
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

describe("extractUsageSnapshot over normalized Codex usage", () => {
  it("reads the per-request breakdown the adapter now emits", () => {
    // The raw `thread/tokenUsage/updated` frame nests everything under
    // `tokenUsage`, which this function never looked at; the adapter now puts a
    // normalized `usage` object beside it.
    const snapshot = extractUsageSnapshot({
      threadId: "thread-1",
      turnId: "turn-1",
      tokenUsage: { total: { inputTokens: 800, outputTokens: 100 }, last: { inputTokens: 80, outputTokens: 10 }, modelContextWindow: 272_000 },
      usage: { input_tokens: 80, output_tokens: 10, cache_read_tokens: 70, cache_write_tokens: 5, total_tokens: 90 },
    });
    expect(snapshot?.totalTokens).toBe(90);
    expect(snapshot?.source).toBe("reported");
  });

  it("stays null for a frame carrying only the raw nested shape", () => {
    expect(extractUsageSnapshot({
      threadId: "thread-1",
      turnId: "turn-1",
      tokenUsage: { total: { inputTokens: 800 }, last: { inputTokens: 80 } },
    })).toBeNull();
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
