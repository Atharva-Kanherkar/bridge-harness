import { describe, expect, it } from "vitest";
import { presetCron, presetFor, relativeTime, triggerSummary } from "./format";

describe("workflow format", () => {
  it("says common schedules in words and shows others as cron", () => {
    expect(triggerSummary({ kind: "manual" })).toBe("On demand");
    expect(triggerSummary({ kind: "schedule", cron: "0 9 * * 1-5", timezone: "UTC" })).toBe("Weekdays at 09:00");
    expect(triggerSummary({ kind: "schedule", cron: "30 16 * * 5", timezone: "UTC" })).toBe("Fridays at 16:30");
    expect(triggerSummary({ kind: "schedule", cron: "15 * * * *", timezone: "UTC" })).toBe("Every hour at :15");
    expect(triggerSummary({ kind: "schedule", cron: "0 9 1,15 * *", timezone: "UTC" })).toBe("Cron 0 9 1,15 * *");
  });

  it("round-trips presets through cron", () => {
    for (const cron of ["0 9 * * *", "0 9 * * 1-5", "30 16 * * 5", "15 * * * *"]) {
      const preset = presetFor({ kind: "schedule", cron, timezone: "UTC" });
      expect(presetCron(preset.preset, preset.time, preset.weekday, preset.custom)).toBe(cron);
    }
    expect(presetFor({ kind: "manual" }).preset).toBe("manual");
  });

  it("reads times relative to now", () => {
    const now = Date.parse("2026-10-29T10:00:00Z");
    expect(relativeTime("2026-10-30T00:00:00Z", now)).toBe("in 14h");
    expect(relativeTime("2026-10-29T08:00:00Z", now)).toBe("2h ago");
    expect(relativeTime(null, now)).toBeNull();
  });
});
