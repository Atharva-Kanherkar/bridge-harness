// Menu-bar meter math, ported from steipete/CodexBar (MIT) via
// `src-tauri/bridge-core/src/meter.rs` — same functions, same thresholds, so
// the Rust daemon and this client never disagree about a window's pace.
//
// - `paceWeekly` ← `UsagePace.weekly` (even-rate path;
//   `Sources/CodexBarCore/UsagePace.swift`)
// - `paceVisible` ← the 3% rule with the weekly-menu-token 1% exception
//   (`docs/ui.md`)
// - `adaptiveDelay` ← `AdaptiveRefreshPolicy` table
//   (`Sources/CodexBar/AdaptiveRefreshPolicy.swift`, `docs/refresh-loop.md`)
//
// Windows arrive from Bridge's own `usage.updated` rate-limit frames
// (`./usage`, `RateWindow`); only the source differs from CodexBar.

import type { RateWindow } from "./usage";

export type PaceStage =
  | "on_track"
  | "slightly_ahead"
  | "ahead"
  | "far_ahead"
  | "slightly_behind"
  | "behind"
  | "far_behind";

export interface MeterPace {
  stage: PaceStage;
  /** Signed delta vs the sustainable rate: +11 is 11% ahead (deficit). */
  deltaPercent: number;
  expectedUsedPercent: number;
  actualUsedPercent: number;
  etaSeconds?: number;
  willLastToReset: boolean;
  speedMultiplierToReset?: number;
}

const clamp = (value: number, lower: number, upper: number): number =>
  Math.min(upper, Math.max(lower, value));

function paceStage(delta: number): PaceStage {
  const absolute = Math.abs(delta);
  if (absolute <= 2) return "on_track";
  if (absolute <= 6) return delta >= 0 ? "slightly_ahead" : "slightly_behind";
  if (absolute <= 12) return delta >= 0 ? "ahead" : "behind";
  return delta >= 0 ? "far_ahead" : "far_behind";
}

function resetsAtMs(window: RateWindow, nowMs: number, resetsAt?: string | null): number | undefined {
  if (resetsAt) {
    const parsed = Date.parse(resetsAt);
    if (Number.isFinite(parsed)) return parsed;
  }
  const direct = window.resetsInSeconds;
  if (direct != null && Number.isFinite(direct) && direct > 0) return nowMs + direct * 1000;
  return undefined;
}

/** Even-rate pace for one quota window. `undefined` when the reset is missing,
 * outside the window, or the sample contradicts the clock. An explicit
 * absolute reset wins over the countdown, mirroring the Rust core. */
export function paceWeekly(window: RateWindow, nowMs: number = Date.now(), resetsAt?: string | null): MeterPace | undefined {
  const reset = resetsAtMs(window, nowMs, resetsAt);
  if (reset == null) return undefined;
  const minutes = window.windowMinutes ?? 10_080;
  if (!Number.isFinite(minutes) || minutes <= 0) return undefined;
  const duration = minutes * 60_000;
  const timeUntilReset = reset - nowMs;
  if (!(timeUntilReset > 0) || timeUntilReset > duration) return undefined;
  const elapsed = clamp(duration - timeUntilReset, 0, duration);
  const expected = clamp((elapsed / duration) * 100, 0, 100);
  const actual = clamp(window.usedPercent, 0, 100);
  if (elapsed === 0 && actual > 0) return undefined;
  const delta = actual - expected;
  // Both operands are in the same time unit (ms), so the quotient is already
  // a percentage — no divisor. (A stray /1000 here once inflated this 1000x
  // vs the Rust core; both suites now assert the multiplier.)
  const projectedRemaining = elapsed > 0 ? (actual * timeUntilReset) / elapsed : 0;
  const remainingCapacity = 100 - actual;
  const speedMultiplierToReset =
    remainingCapacity > 0 && projectedRemaining > 0 && Number.isFinite(remainingCapacity / projectedRemaining)
      ? remainingCapacity / projectedRemaining
      : undefined;
  let etaSeconds: number | undefined;
  let willLastToReset = false;
  if (actual >= 100) {
    etaSeconds = 0;
  } else if (elapsed > 0 && actual > 0) {
    const rate = actual / (elapsed / 1000);
    if (rate > 0) {
      const candidate = (100 - actual) / rate;
      if (candidate >= timeUntilReset / 1000) willLastToReset = true;
      else etaSeconds = candidate;
    }
  } else if (elapsed > 0) {
    willLastToReset = true;
  }
  return { stage: paceStage(delta), deltaPercent: delta, expectedUsedPercent: expected, actualUsedPercent: actual, etaSeconds, willLastToReset, speedMultiplierToReset };
}

/** Whether pace is shown: hidden until 3% of the window has elapsed; the
 * weekly menu-bar token appears after 1%. */
export function paceVisible(window: RateWindow, nowMs: number = Date.now(), weeklyMenuToken = false): boolean {
  const resetsAt = resetsAtMs(window, nowMs);
  if (resetsAt == null) return false;
  const minutes = window.windowMinutes ?? 10_080;
  if (!Number.isFinite(minutes) || minutes <= 0) return false;
  const duration = minutes * 60_000;
  const elapsed = duration - (resetsAt - nowMs);
  if (elapsed <= 0) return false;
  return elapsed / duration >= (weeklyMenuToken && minutes === 10_080 ? 0.01 : 0.03);
}

/** "3% in deficit · runs out in 2h" / "5% in reserve · lasts until reset". */
export function paceLabel(pace: MeterPace): string {
  const delta = Math.round(pace.deltaPercent);
  const head =
    pace.stage === "on_track"
      ? "on pace"
      : pace.deltaPercent >= 0
        ? `${delta}% in deficit`
        : `${Math.abs(delta)}% in reserve`;
  const tail =
    pace.actualUsedPercent >= 100
      ? "limit reached"
      : pace.willLastToReset
        ? "lasts until reset"
        : pace.etaSeconds != null
          ? `runs out in ${compactDuration(pace.etaSeconds)}`
          : "reset timing only";
  return `${head} · ${tail}`;
}

/** The pace in plain words for the meter card — no signed deltas, no jargon:
 * "on pace · lasts to reset", "ahead of pace · runs out in 2d 3h",
 * "under pace · lasts to reset". */
export function pacePhrase(pace: MeterPace): string {
  const head =
    pace.stage === "on_track"
      ? "on pace"
      : pace.deltaPercent >= 0
        ? "ahead of pace"
        : "under pace";
  const tail =
    pace.actualUsedPercent >= 100
      ? "limit reached"
      : pace.willLastToReset
        ? "lasts to reset"
        : pace.etaSeconds != null
          ? `runs out in ${compactDuration(pace.etaSeconds)}`
          : undefined;
  return tail ? `${head} · ${tail}` : head;
}

/** Signed compact delta for menu-bar tokens: `+11%` ahead, `-8%` behind. */
export function paceTokenDelta(pace: MeterPace): string {
  const rounded = Math.round(pace.deltaPercent);
  return `${rounded >= 0 ? "+" : ""}${rounded}%`;
}

function compactDuration(seconds: number): string {
  const total = Math.round(seconds);
  const days = Math.floor(total / 86_400);
  const hours = Math.floor((total % 86_400) / 3_600);
  const minutes = Math.floor((total % 3_600) / 60);
  if (days > 0) return `${days}d ${hours}h`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  if (minutes > 0) return `${minutes}m`;
  return "soon";
}

export type AdaptiveReason =
  | "constrained"
  | "recentInteraction"
  | "warm"
  | "codingActivity"
  | "idle"
  | "longIdle";

export interface AdaptiveInput {
  nowMs: number;
  lastMenuOpenMs?: number | null;
  lastCodingActivityMs?: number | null;
  lowPowerMode: boolean;
  thermallyConstrained: boolean;
  agentAware: boolean;
}

/** Next automatic refresh delay. First match wins; every decision lands in
 * 2–30 minutes, mirroring `meter::adaptive_delay`. */
export function adaptiveDelay(input: AdaptiveInput): { delayMs: number; reason: AdaptiveReason } {
  const MINUTE = 60_000;
  if (input.lowPowerMode || input.thermallyConstrained) return { delayMs: 30 * MINUTE, reason: "constrained" };
  if (input.lastMenuOpenMs != null) {
    const age = input.nowMs - input.lastMenuOpenMs;
    if (age <= 5 * MINUTE) return { delayMs: 2 * MINUTE, reason: "recentInteraction" };
    if (age <= 60 * MINUTE) return { delayMs: 5 * MINUTE, reason: "warm" };
    if (age <= 4 * 60 * MINUTE) {
      if (input.agentAware && input.lastCodingActivityMs != null && input.nowMs - input.lastCodingActivityMs < 5 * MINUTE) {
        return { delayMs: 5 * MINUTE, reason: "codingActivity" };
      }
      return { delayMs: 15 * MINUTE, reason: "idle" };
    }
    return { delayMs: 30 * MINUTE, reason: "longIdle" };
  }
  if (input.agentAware && input.lastCodingActivityMs != null && input.nowMs - input.lastCodingActivityMs < 5 * MINUTE) {
    return { delayMs: 5 * MINUTE, reason: "codingActivity" };
  }
  return { delayMs: 30 * MINUTE, reason: "longIdle" };
}

/** Steady-state active cadence for interval-derived heuristics. */
export const METER_NOMINAL_INTERVAL_MS = 5 * 60_000;
