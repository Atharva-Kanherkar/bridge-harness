import { describe, expect, it, vi } from "vitest";
import type { UsageOverviewSnapshot } from "./protocol/generated/protocol";
import { overviewUsage } from "./usageOverview";

function snapshot(): UsageOverviewSnapshot {
  const unavailable = { value: null, source: null, status: "unavailable" } as const;
  return { schemaVersion: 1, provider: "codex", generatedAt: 100, observedAt: 100, account: null, plan: "pro",
    windows: [{ id: "session", label: "Session", usedPercent: { value: 0, source: "reported", status: "current" }, resetsAt: 200, windowMinutes: 300 }],
    today: { tokens: unavailable, costMicrousd: unavailable, models: [] },
    month: { tokens: unavailable, costMicrousd: unavailable, models: [] }, coverage: "Recorded on this Mac", error: null };
}

describe("shared usage overview adapter", () => {
  it("keeps reported zero but drops stale, expired, and unsupported observations", () => {
    vi.spyOn(Date, "now").mockReturnValue(110_000);
    try {
      const value = snapshot();
      expect(overviewUsage(value)?.windows[0].usedPercent).toBe(0);
      expect(overviewUsage(value)?.windows[0].resetsInSeconds).toBe(90);
      value.observedAt = null;
      expect(overviewUsage(value)).toBeNull();
      value.observedAt = 111;
      expect(overviewUsage(value)).toBeNull();
      value.observedAt = 100;
      value.error = "Account refresh failed";
      expect(overviewUsage(value)).toBeNull();
      value.error = null;
      value.windows[0].usedPercent.status = "stale";
      expect(overviewUsage(value)).toBeNull();
      value.windows[0].usedPercent.status = "current";
      value.windows[0].resetsAt = 109;
      expect(overviewUsage(value)).toBeNull();
      value.schemaVersion = 2;
      expect(overviewUsage(value)).toBeNull();
    } finally { vi.restoreAllMocks(); }
  });
});
