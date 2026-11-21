// Screen Time contract rules 12 to 14 and 22 (testing/feat-usage-screen-time.md):
// what the tracker counts, when it sends, what survives a failed send, the
// opt-out, and how durations read.
import { describe, expect, it, vi } from "vitest";
import type { UsageScreenTimeResult } from "./types";
import {
  DEFAULT_SCREEN_TIME_PREFERENCES, formatDuration, formatLeverage, IDLE_MS, MAX_PENDING_MINUTES, PresenceTracker, readScreenTimePreferences,
  SCREEN_TIME_PREFERENCES_KEY, screenTimeParams, summarizeScreenTime, writeScreenTimePreferences, type PresenceEnvironment,
} from "./screenTime";

const MINUTE = 60_000;
const START = Date.UTC(2026, 10, 21, 9, 0, 0);

function harness(overrides: Partial<PresenceEnvironment> = {}, initial: Partial<{ focused: boolean }> = {}) {
  const state = { now: START, focused: true, visible: true, tracking: true, ...initial };
  const sent: number[][] = [];
  const send = vi.fn(async (minutes: number[]) => { sent.push(minutes); });
  const tracker = new PresenceTracker({
    now: () => state.now,
    focused: () => state.focused,
    visible: () => state.visible,
    tracking: () => state.tracking,
    send,
    ...overrides,
  });
  return { state, sent, send, tracker, minute: (at: number) => Math.floor(at / MINUTE) };
}

describe("PresenceTracker", () => {
  it("counts a minute once while the window is focused, visible and in use", async () => {
    const { state, tracker, sent, minute } = harness();
    tracker.tick();
    state.now += 15_000;
    tracker.tick();
    state.now += 60_000;
    tracker.input();
    tracker.tick();
    await tracker.flush();
    expect(sent).toEqual([[minute(START), minute(START + 75_000)]]);
  });

  it("stops counting after five idle minutes and starts again on input", async () => {
    const { state, tracker, sent, minute } = harness();
    state.now = START + IDLE_MS - 1;
    tracker.tick();
    state.now = START + IDLE_MS;
    tracker.tick();
    state.now = START + IDLE_MS + 3 * MINUTE;
    tracker.tick();
    expect(tracker.pendingMinutes()).toEqual([minute(START + IDLE_MS - 1)]);
    tracker.input();
    tracker.tick();
    await tracker.flush();
    expect(sent[0]).toEqual([minute(START + IDLE_MS - 1), minute(START + IDLE_MS + 3 * MINUTE)]);
  });

  it("counts nothing while the window is unfocused or hidden", () => {
    const { state, tracker } = harness();
    state.focused = false;
    tracker.input();
    tracker.tick();
    state.focused = true;
    state.visible = false;
    tracker.tick();
    expect(tracker.pendingMinutes()).toEqual([]);
  });

  it("starts idle when the app opens into an unfocused window, until it is used", () => {
    const { state, tracker, minute } = harness({}, { focused: false });
    state.focused = true;
    tracker.tick();
    expect(tracker.pendingMinutes()).toEqual([]);
    tracker.input();
    tracker.tick();
    expect(tracker.pendingMinutes()).toEqual([minute(START)]);
  });

  it("keeps minutes from a failed send for the next flush", async () => {
    const { state, tracker, send, minute } = harness();
    send.mockRejectedValueOnce(new Error("daemon away"));
    tracker.tick();
    await tracker.flush();
    expect(tracker.pendingMinutes()).toEqual([minute(START)]);
    state.now += MINUTE;
    tracker.tick();
    await tracker.flush();
    expect(send).toHaveBeenLastCalledWith([minute(START), minute(START + MINUTE)]);
    expect(tracker.pendingMinutes()).toEqual([]);
  });

  it("caps pending minutes at one day, dropping the oldest", async () => {
    const { state, tracker, send } = harness({ send: async () => { throw new Error("offline"); } });
    for (let index = 0; index < MAX_PENDING_MINUTES + 5; index += 1) {
      tracker.input();
      tracker.tick();
      state.now += MINUTE;
    }
    await tracker.flush();
    const pending = tracker.pendingMinutes();
    expect(pending).toHaveLength(MAX_PENDING_MINUTES);
    expect(pending[0]).toBe(Math.floor(START / MINUTE) + 5);
    expect(send).not.toHaveBeenCalled();
  });

  it("records and sends nothing with tracking off, and drops what was pending", async () => {
    const { state, tracker, send } = harness();
    tracker.tick();
    state.tracking = false;
    tracker.tick();
    await tracker.flush();
    expect(tracker.pendingMinutes()).toEqual([]);
    expect(send).not.toHaveBeenCalled();
  });

  it("does not send an empty batch", async () => {
    const { tracker, send } = harness({ focused: () => false });
    await tracker.flush();
    expect(send).not.toHaveBeenCalled();
  });
});

describe("Screen Time preferences", () => {
  const memory = () => {
    const values = new Map<string, string>();
    return { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => void values.set(key, value) };
  };

  it("defaults to tracking and falls back on a malformed value", () => {
    const storage = memory();
    expect(readScreenTimePreferences(storage)).toEqual(DEFAULT_SCREEN_TIME_PREFERENCES);
    storage.setItem(SCREEN_TIME_PREFERENCES_KEY, "{not json");
    expect(readScreenTimePreferences(storage)).toEqual({ track: true });
    storage.setItem(SCREEN_TIME_PREFERENCES_KEY, JSON.stringify({ track: "no" }));
    expect(readScreenTimePreferences(storage)).toEqual({ track: true });
  });

  it("round-trips the opt-out", () => {
    const storage = memory();
    writeScreenTimePreferences({ track: false }, storage);
    expect(readScreenTimePreferences(storage)).toEqual({ track: false });
  });
});

describe("Screen Time arithmetic", () => {
  it("formats durations like the contract", () => {
    expect(formatDuration(0)).toBe("0m");
    expect(formatDuration(45 * 60)).toBe("45m");
    expect(formatDuration(3 * 3_600 + 5 * 60)).toBe("3h 05m");
    expect(formatDuration(99 * 3_600 + 59 * 60)).toBe("99h 59m");
    expect(formatDuration(1_204 * 3_600 + 30 * 60)).toBe("1,204h");
    expect(formatLeverage(3.21)).toBe("3.2×");
    expect(formatLeverage(12.4)).toBe("12×");
  });

  it("asks for the last 7 or 30 local days ending today", () => {
    const now = new Date("2026-11-20T16:00:00Z");
    expect(screenTimeParams(7, now, "Asia/Tokyo")).toEqual({ sinceDay: "2026-11-15", untilDay: "2026-11-21", timeZone: "Asia/Tokyo" });
    expect(screenTimeParams(30, now, "UTC")).toEqual({ sinceDay: "2026-10-22", untilDay: "2026-11-20", timeZone: "UTC" });
  });

  it("builds today, the average, leverage and the running all-time total", () => {
    const result: UsageScreenTimeResult = {
      sinceDay: "2026-11-19",
      untilDay: "2026-11-21",
      timeZone: "UTC",
      days: [
        { day: "2026-11-19", humanSeconds: 3_600, agentSeconds: 10_800 },
        { day: "2026-11-20", humanSeconds: 0, agentSeconds: 0 },
        { day: "2026-11-21", humanSeconds: 1_800, agentSeconds: 3_600 },
      ],
      allTime: { humanSeconds: 10 * 3_600, agentSeconds: 50 * 3_600, humanSinceDay: "2026-10-01", agentSinceDay: "2026-08-01" },
      agentHistory: "complete",
    };
    const summary = summarizeScreenTime(result);
    expect(summary.today).toEqual({ humanSeconds: 1_800, agentSeconds: 3_600 });
    expect(summary.humanSeconds).toBe(5_400);
    expect(summary.dailyAverageSeconds).toBe(1_800);
    expect(summary.leverage).toBeCloseTo(14_400 / 5_400);
    expect(summary.cumulative).toEqual([34_200, 34_200, 36_000]);
    const idle = summarizeScreenTime({ ...result, days: result.days.map(day => ({ ...day, humanSeconds: 0 })), allTime: { ...result.allTime, humanSeconds: 0 } });
    expect(idle.leverage).toBeNull();
    expect(idle.cumulative).toEqual([0, 0, 0]);
  });
});
