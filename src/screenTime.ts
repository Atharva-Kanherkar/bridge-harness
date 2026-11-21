import type { ScreenTimeParams, UsageScreenTimeResult } from "./types";
import { makeUsageWindow, resolveTimeZone } from "./usageReport";

// Screen Time in the webview: the tracker that reports the minutes you spend
// in Bridge, its opt-out, and the arithmetic the Time tab draws. Rules are in
// testing/feat-usage-screen-time.md.

export const SCREEN_TIME_PREFERENCES_KEY = "bridge.screenTime.v1";
/** Fired on `window` when the preference changes, so the tracker and the tab agree at once. */
export const SCREEN_TIME_PREFERENCES_EVENT = "bridge:screen-time-preferences";

export interface ScreenTimePreferences { track: boolean }
export const DEFAULT_SCREEN_TIME_PREFERENCES: ScreenTimePreferences = { track: true };

export function readScreenTimePreferences(storage: Pick<Storage, "getItem"> | undefined = globalThis.localStorage): ScreenTimePreferences {
  try {
    const parsed: unknown = JSON.parse(storage?.getItem(SCREEN_TIME_PREFERENCES_KEY) ?? "null");
    if (parsed && typeof parsed === "object" && typeof (parsed as { track?: unknown }).track === "boolean") return { track: (parsed as ScreenTimePreferences).track };
  } catch {
    // A malformed value means the default.
  }
  return { ...DEFAULT_SCREEN_TIME_PREFERENCES };
}

export function writeScreenTimePreferences(preferences: ScreenTimePreferences, storage: Pick<Storage, "setItem"> | undefined = globalThis.localStorage): void {
  try {
    storage?.setItem(SCREEN_TIME_PREFERENCES_KEY, JSON.stringify({ track: preferences.track }));
  } catch {
    // Storage can be full or blocked; the in-memory choice still applies.
  }
  globalThis.dispatchEvent?.(new CustomEvent(SCREEN_TIME_PREFERENCES_EVENT, { detail: preferences }));
}

/** No input for this long and the minute stops counting, like a screen dimming. */
export const IDLE_MS = 5 * 60_000;
export const TICK_MS = 15_000;
export const FLUSH_MS = 60_000;
/** One day of minutes: what a single `usage/record_presence` call may carry. */
export const MAX_PENDING_MINUTES = 1_440;

export interface PresenceEnvironment {
  now: () => number;
  focused: () => boolean;
  visible: () => boolean;
  tracking: () => boolean;
  send: (minutes: number[]) => Promise<unknown>;
}

/** Collects the minutes you were active and sends them in batches. DOM-free, so tests drive it with a fake clock. */
export class PresenceTracker {
  private lastInputAt: number;
  private readonly pending = new Set<number>();
  private sending = false;

  constructor(private readonly env: PresenceEnvironment) {
    // Opening Bridge into a focused window is using it.
    this.lastInputAt = env.focused() ? env.now() : Number.NEGATIVE_INFINITY;
  }

  /** Any pointer, key, wheel or touch input, or focusing the window. */
  input(): void {
    this.lastInputAt = this.env.now();
  }

  /** Counts the current minute when every rule holds. */
  tick(): void {
    if (!this.env.tracking()) {
      this.pending.clear();
      return;
    }
    const now = this.env.now();
    if (this.env.focused() && this.env.visible() && now - this.lastInputAt < IDLE_MS) {
      this.pending.add(Math.floor(now / 60_000));
      this.trim();
    }
  }

  /** Sends what is pending. A failed send keeps the minutes for the next flush. */
  async flush(): Promise<void> {
    if (!this.env.tracking()) {
      this.pending.clear();
      return;
    }
    if (this.sending || this.pending.size === 0) return;
    const minutes = this.pendingMinutes();
    this.pending.clear();
    this.sending = true;
    try {
      await this.env.send(minutes);
    } catch {
      for (const minute of minutes) this.pending.add(minute);
      this.trim();
    } finally {
      this.sending = false;
    }
  }

  pendingMinutes(): number[] {
    return [...this.pending].sort((a, b) => a - b);
  }

  private trim(): void {
    if (this.pending.size <= MAX_PENDING_MINUTES) return;
    for (const minute of this.pendingMinutes().slice(0, this.pending.size - MAX_PENDING_MINUTES)) this.pending.delete(minute);
  }
}

const INPUT_EVENTS = ["pointerdown", "pointermove", "keydown", "wheel", "touchstart"] as const;

/** Wires a tracker to this window. Returns the cleanup. */
export function startPresenceTracking(send: (minutes: number[]) => Promise<unknown>, target: Window = window): () => void {
  let tracking = readScreenTimePreferences().track;
  const doc = target.document;
  const tracker = new PresenceTracker({
    now: () => Date.now(),
    focused: () => doc.hasFocus(),
    visible: () => doc.visibilityState === "visible",
    tracking: () => tracking,
    send,
  });
  const onInput = () => tracker.input();
  const onLeave = () => {
    tracker.tick();
    void tracker.flush();
  };
  const onVisibility = () => {
    if (doc.visibilityState === "hidden") onLeave();
    else tracker.input();
  };
  const onPreferences = () => {
    tracking = readScreenTimePreferences().track;
    if (!tracking) void tracker.flush();
  };
  const options: AddEventListenerOptions = { capture: true, passive: true };
  for (const name of INPUT_EVENTS) target.addEventListener(name, onInput, options);
  target.addEventListener("focus", onInput);
  target.addEventListener("blur", onLeave);
  target.addEventListener("pagehide", onLeave);
  target.addEventListener(SCREEN_TIME_PREFERENCES_EVENT, onPreferences);
  doc.addEventListener("visibilitychange", onVisibility);
  const ticker = target.setInterval(() => tracker.tick(), TICK_MS);
  const flusher = target.setInterval(() => void tracker.flush(), FLUSH_MS);
  tracker.tick();
  return () => {
    for (const name of INPUT_EVENTS) target.removeEventListener(name, onInput, options);
    target.removeEventListener("focus", onInput);
    target.removeEventListener("blur", onLeave);
    target.removeEventListener("pagehide", onLeave);
    target.removeEventListener(SCREEN_TIME_PREFERENCES_EVENT, onPreferences);
    doc.removeEventListener("visibilitychange", onVisibility);
    target.clearInterval(ticker);
    target.clearInterval(flusher);
    onLeave();
  };
}

export type ScreenTimeRange = 7 | 30;

export function screenTimeParams(days: ScreenTimeRange, now: Date = new Date(), timeZone: string = resolveTimeZone()): ScreenTimeParams {
  const window = makeUsageWindow(days, now, timeZone);
  return { sinceDay: window.sinceDay, untilDay: window.untilDay, timeZone };
}

/** `0m`, `45m`, `3h 05m`, then whole hours past 100: `1,204h`. */
export function formatDuration(seconds: number): string {
  const minutes = Math.max(0, Math.round(seconds / 60));
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours >= 100) return `${hours.toLocaleString("en-US")}h`;
  return `${hours}h ${String(minutes % 60).padStart(2, "0")}m`;
}

export interface ScreenTimeSummary {
  today: { humanSeconds: number; agentSeconds: number };
  humanSeconds: number;
  agentSeconds: number;
  dailyAverageSeconds: number;
  /** Agent minutes per minute of yours, or null with none of yours in the range. */
  leverage: number | null;
  /** Your all-time total at the end of each day in the range. */
  cumulative: number[];
}

export function summarizeScreenTime(result: UsageScreenTimeResult): ScreenTimeSummary {
  const humanSeconds = result.days.reduce((sum, day) => sum + day.humanSeconds, 0);
  const agentSeconds = result.days.reduce((sum, day) => sum + day.agentSeconds, 0);
  const last = result.days[result.days.length - 1];
  // Everything before the range, then each day added on. Clamped: a minute
  // recorded between the two reads must not draw a negative start.
  let running = Math.max(0, result.allTime.humanSeconds - humanSeconds);
  const cumulative = result.days.map(day => (running += day.humanSeconds));
  return {
    today: { humanSeconds: last?.humanSeconds ?? 0, agentSeconds: last?.agentSeconds ?? 0 },
    humanSeconds,
    agentSeconds,
    dailyAverageSeconds: result.days.length > 0 ? humanSeconds / result.days.length : 0,
    leverage: humanSeconds > 0 ? agentSeconds / humanSeconds : null,
    cumulative,
  };
}

export function formatLeverage(leverage: number): string {
  return `${leverage >= 10 ? Math.round(leverage) : leverage.toFixed(1)}×`;
}
