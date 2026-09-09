// The TypeScript half of the CodexBar meter port: `paceWeekly`,
// `paceVisible`, and `adaptiveDelay` must agree with
// `src-tauri/bridge-core/src/meter.rs` case for case.
import { describe, expect, it } from "vitest";
import { adaptiveDelay, paceLabel, paceTokenDelta, paceVisible, paceWeekly, type AdaptiveInput } from "./meter";
import type { RateWindow } from "./usage";
import fixture from "../testing/fixtures/meter-pace-cases.json";

const NOW = Date.UTC(2026, 8, 7, 12, 0, 0); // a Monday noon UTC
const WEEK_MS = 10_080 * 60_000;

function window(usedPercent: number, resetsInMs: number): RateWindow {
  return { id: "weekly", label: "Weekly", usedPercent, windowMinutes: 10_080, resetsInSeconds: resetsInMs / 1000, source: "reported" };
}

describe("meter (CodexBar port)", () => {
  it("reports deficit with an ETA at half time over pace", () => {
    const pace = paceWeekly(window(75, WEEK_MS / 2), NOW)!;
    expect(pace.expectedUsedPercent).toBeCloseTo(50, 6);
    expect(pace.deltaPercent).toBeCloseTo(25, 6);
    expect(pace.stage).toBe("far_ahead");
    expect(pace.willLastToReset).toBe(false);
    expect(pace.etaSeconds).toBeGreaterThan(0);
    expect(paceTokenDelta(pace)).toBe("+25%");
    expect(paceLabel(pace)).toContain("in deficit");
  });

  it("reports reserve lasting until reset when under pace", () => {
    const pace = paceWeekly(window(20, WEEK_MS / 2), NOW)!;
    expect(pace.stage).toBe("far_behind");
    expect(pace.willLastToReset).toBe(true);
    expect(paceLabel(pace)).toContain("in reserve");
    expect(paceLabel(pace)).toContain("lasts until reset");
  });

  it("yields nothing without reset timing or with a reset outside the window", () => {
    expect(paceWeekly({ id: "w", label: "W", usedPercent: 10, source: "reported" }, NOW)).toBeUndefined();
    expect(paceWeekly(window(10, WEEK_MS * 2), NOW)).toBeUndefined();
  });

  it("agrees with the Rust core on every shared fixture, including the multiplier", () => {
    const nowMs = Date.parse(fixture.now);
    for (const kase of fixture.cases) {
      const pace = paceWeekly(
        {
          id: "weekly",
          label: "Weekly",
          usedPercent: kase.used_percent,
          windowMinutes: kase.window_minutes,
          resetsInSeconds: kase.resets_in_seconds ?? undefined,
          source: "reported",
        },
        nowMs,
        kase.resets_at,
      )!;
      expect(pace, `${kase.name}: pace`).toBeDefined();
      expect(pace.deltaPercent, `${kase.name}: delta`).toBeCloseTo(kase.expected.delta, 6);
      expect(pace.stage, `${kase.name}: stage`).toBe(kase.expected.stage);
      expect(pace.willLastToReset, `${kase.name}: willLast`).toBe(kase.expected.will_last);
      expect(pace.etaSeconds != null, `${kase.name}: eta`).toBe(kase.expected.eta_some);
      expect(pace.speedMultiplierToReset ?? NaN, `${kase.name}: multiplier`).toBeCloseTo(kase.expected.multiplier, 4);
    }
  });

  it("hides pace before 3% elapsed, except the weekly menu token at 1%", () => {
    const early = window(1, WEEK_MS - 0.02 * WEEK_MS);
    expect(paceVisible(early, NOW)).toBe(false);
    expect(paceVisible(early, NOW, true)).toBe(true);
    expect(paceVisible(window(10, WEEK_MS - 0.05 * WEEK_MS), NOW)).toBe(true);
  });

  it("matches the adaptive table reasons", () => {
    const base: AdaptiveInput = { nowMs: NOW, lowPowerMode: false, thermallyConstrained: false, agentAware: false };
    expect(adaptiveDelay(base).reason).toBe("longIdle");
    expect(adaptiveDelay({ ...base, lowPowerMode: true }).reason).toBe("constrained");
    expect(adaptiveDelay({ ...base, lastMenuOpenMs: NOW - 60_000 }).reason).toBe("recentInteraction");
    expect(adaptiveDelay({ ...base, lastMenuOpenMs: NOW - 30 * 60_000 }).reason).toBe("warm");
    expect(adaptiveDelay({ ...base, lastMenuOpenMs: NOW - 2 * 60 * 60_000 }).reason).toBe("idle");
    expect(adaptiveDelay({ ...base, lastMenuOpenMs: NOW - 2 * 60 * 60_000, lastCodingActivityMs: NOW - 60_000, agentAware: true }).reason).toBe("codingActivity");
    expect(adaptiveDelay({ ...base, lastMenuOpenMs: NOW - 2 * 60 * 60_000, lastCodingActivityMs: NOW - 60_000 }).reason).toBe("idle");
  });
});
