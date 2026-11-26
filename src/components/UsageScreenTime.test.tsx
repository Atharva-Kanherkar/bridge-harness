// @vitest-environment jsdom
// The Time tab and the window wiring, rules 12, 13, 15 to 21 of
// testing/feat-usage-screen-time.md.
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import type { ScreenTimeParams, UsageScreenTimeResult } from "../types";
import { FLUSH_MS, readScreenTimePreferences, SCREEN_TIME_PREFERENCES_KEY, startPresenceTracking, TICK_MS, writeScreenTimePreferences } from "../screenTime";
import { clearScreenTimeCacheForTests, COUNTING_POLL_MS, UsageScreenTime } from "./UsageScreenTime";

let container: HTMLDivElement;
let root: Root;

const HOUR = 3_600;

function days(params: ScreenTimeParams, fill: (index: number) => [number, number]): UsageScreenTimeResult["days"] {
  const out: UsageScreenTimeResult["days"] = [];
  const start = new Date(`${params.sinceDay}T00:00:00Z`).getTime();
  const end = new Date(`${params.untilDay}T00:00:00Z`).getTime();
  for (let at = start, index = 0; at <= end; at += 86_400_000, index += 1) {
    const [human, agent] = fill(index);
    out.push({ day: new Date(at).toISOString().slice(0, 10), humanSeconds: human, agentSeconds: agent });
  }
  return out;
}

function result(params: ScreenTimeParams, overrides: Partial<UsageScreenTimeResult> = {}, fill: (index: number) => [number, number] = index => [HOUR + index * 600, 3 * HOUR]): UsageScreenTimeResult {
  return {
    sinceDay: params.sinceDay,
    untilDay: params.untilDay,
    timeZone: params.timeZone ?? "UTC",
    days: days(params, fill),
    allTime: { humanSeconds: 120 * HOUR, agentSeconds: 400 * HOUR, humanSinceDay: "2026-09-01", agentSinceDay: "2026-07-04" },
    agentHistory: "complete",
    projects: [],
    pullRequests: { opened: 0, merged: 0, open: 0, closed: 0, repos: [], recent: [] },
    hours: Array.from({ length: 24 }, (_, hour) => ({ hour, humanSeconds: 0, agentSeconds: 0 })),
    harnesses: [],
    ...overrides,
  };
}

const settle = () => act(async () => { for (let i = 0; i < 5; i += 1) await Promise.resolve(); });
const text = () => container.textContent ?? "";
const click = (element: Element | null | undefined) => act(async () => { (element as HTMLElement).click(); });
const byText = (selector: string, label: string) => [...container.querySelectorAll(selector)].find(element => element.textContent?.trim() === label);

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  localStorage.clear();
  clearScreenTimeCacheForTests();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("UsageScreenTime", () => {
  it("shows today, the range, all time and agent leverage, and a column per day", async () => {
    const spy = vi.spyOn(bridgeApi, "screenTime").mockImplementation(async params => result(params));
    await act(async () => { root.render(<UsageScreenTime onError={() => undefined} />); });
    await settle();
    const params = spy.mock.calls[0][0];
    expect(params.timeZone).toBeTruthy();
    expect((new Date(`${params.untilDay}T00:00:00Z`).getTime() - new Date(`${params.sinceDay}T00:00:00Z`).getTime()) / 86_400_000).toBe(6);
    // Today is the seventh day: 1h + 6 x 10m.
    expect(container.querySelector('[data-testid="screen-time-today"]')?.textContent).toBe("2h 00m");
    expect(text()).toContain("agents worked 3h 00m");
    expect(text()).toContain("Daily average1h 30m");
    expect(text()).toContain("Last 7 days10h 30m");
    expect(text()).toContain("All time120h");
    expect(text()).toContain("since Sep 1");
    expect(text()).toContain("Agent time21h 00m2.0× yours");
    expect(container.querySelectorAll("[data-day]")).toHaveLength(7);
    expect(container.querySelector('[data-testid="screen-time-all-time"]')?.textContent).toBe("120h");
    expect(text()).toContain("Today · 120h");
    expect(text()).toContain("109h before");
  });

  it("reads one day on hover and the range total otherwise", async () => {
    vi.spyOn(bridgeApi, "screenTime").mockImplementation(async params => result(params));
    await act(async () => { root.render(<UsageScreenTime onError={() => undefined} />); });
    await settle();
    const status = () => container.querySelector('section[aria-label="Daily time"] [role="status"]')?.textContent ?? "";
    expect(status()).toBe("Last 7 days: you 10h 30m · agents 21h 00m");
    const first = container.querySelector("[data-day]")!;
    await act(async () => { first.dispatchEvent(new MouseEvent("pointerover", { bubbles: true })); });
    expect(status()).toMatch(/^[A-Z][a-z]{2}, [A-Z][a-z]{2} \d+: you 1h 00m · agents 3h 00m$/);
  });

  it("asks for 30 days on Month", async () => {
    const spy = vi.spyOn(bridgeApi, "screenTime").mockImplementation(async params => result(params));
    await act(async () => { root.render(<UsageScreenTime onError={() => undefined} />); });
    await settle();
    await click(byText('[role="radio"]', "Month"));
    await settle();
    const params = spy.mock.calls.at(-1)![0];
    expect((new Date(`${params.untilDay}T00:00:00Z`).getTime() - new Date(`${params.sinceDay}T00:00:00Z`).getTime()) / 86_400_000).toBe(29);
    expect(container.querySelectorAll("[data-day]")).toHaveLength(30);
    expect(text()).toContain("Last 30 days");
  });

  it("says agent history is still counting and asks again until it is complete", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    let calls = 0;
    const spy = vi.spyOn(bridgeApi, "screenTime").mockImplementation(async params => result(params, { agentHistory: calls++ < 1 ? "counting" : "complete" }));
    await act(async () => { root.render(<UsageScreenTime onError={() => undefined} />); });
    await settle();
    expect(text()).toContain("Still counting agent time from before Screen Time");
    expect(spy).toHaveBeenCalledTimes(1);
    await act(async () => { vi.advanceTimersByTime(COUNTING_POLL_MS); });
    await settle();
    expect(spy).toHaveBeenCalledTimes(2);
    expect(text()).not.toContain("Still counting");
    await act(async () => { vi.advanceTimersByTime(COUNTING_POLL_MS * 3); });
    expect(spy).toHaveBeenCalledTimes(2);
  });

  it("explains an empty range and a fresh start", async () => {
    vi.spyOn(bridgeApi, "screenTime").mockImplementation(async params => result(params, { allTime: { humanSeconds: 0, agentSeconds: 0, humanSinceDay: null, agentSinceDay: null } }, () => [0, 0]));
    await act(async () => { root.render(<UsageScreenTime onError={() => undefined} />); });
    await settle();
    expect(text()).toContain("No time recorded in this range.");
    expect(text()).toContain("Bridge counts your time from now on.");
    expect(text()).toContain("Your total starts with the first minute you spend in Bridge.");
    expect(text()).toContain("starting now");
    expect(text()).not.toContain("× yours");
  });

  it("turns tracking off and on with the switch", async () => {
    vi.spyOn(bridgeApi, "screenTime").mockImplementation(async params => result(params));
    await act(async () => { root.render(<UsageScreenTime onError={() => undefined} />); });
    await settle();
    const toggle = container.querySelector<HTMLButtonElement>('[role="switch"]')!;
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    await click(toggle);
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    expect(readScreenTimePreferences()).toEqual({ track: false });
    expect(text()).toContain("Your time is not being recorded. Agent time still counts.");
    await click(toggle);
    expect(JSON.parse(localStorage.getItem(SCREEN_TIME_PREFERENCES_KEY)!)).toEqual({ track: true });
    expect(text()).not.toContain("not being recorded");
  });

  it("reports a failed read", async () => {
    vi.spyOn(bridgeApi, "screenTime").mockRejectedValue(new Error("daemon away"));
    const onError = vi.fn();
    await act(async () => { root.render(<UsageScreenTime onError={onError} />); });
    await settle();
    expect(onError).toHaveBeenCalledWith("Daemon away.");
  });
});

describe("startPresenceTracking", () => {
  let visibility: DocumentVisibilityState = "visible";
  let focused = true;

  beforeEach(() => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval", "Date"] });
    vi.setSystemTime(new Date("2026-11-21T09:00:10Z"));
    visibility = "visible";
    focused = true;
    vi.spyOn(document, "hasFocus").mockImplementation(() => focused);
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => visibility });
  });

  const minute = () => Math.floor(Date.now() / 60_000);

  it("sends the minutes in use every minute and at once on blur", async () => {
    const send = vi.fn(async () => undefined);
    const stop = startPresenceTracking(send);
    const first = minute();
    window.dispatchEvent(new Event("pointermove"));
    vi.advanceTimersByTime(TICK_MS);
    vi.advanceTimersByTime(FLUSH_MS - TICK_MS);
    await Promise.resolve();
    expect(send).toHaveBeenCalledWith({ minutes: [first, first + 1] });
    window.dispatchEvent(new Event("keydown"));
    vi.advanceTimersByTime(TICK_MS);
    focused = false;
    window.dispatchEvent(new Event("blur"));
    expect(send).toHaveBeenCalledTimes(2);
    expect(send).toHaveBeenLastCalledWith({ minutes: [first + 1] });
    stop();
  });

  it("flushes when the document turns hidden and stops counting while hidden", async () => {
    const send = vi.fn(async () => undefined);
    const stop = startPresenceTracking(send);
    visibility = "hidden";
    document.dispatchEvent(new Event("visibilitychange"));
    expect(send).toHaveBeenCalledWith({ minutes: [minute()] });
    vi.advanceTimersByTime(FLUSH_MS * 3);
    await Promise.resolve();
    expect(send).toHaveBeenCalledTimes(1);
    stop();
  });

  it("records nothing once tracking is switched off", async () => {
    const send = vi.fn(async () => undefined);
    const stop = startPresenceTracking(send);
    writeScreenTimePreferences({ track: false });
    vi.advanceTimersByTime(FLUSH_MS * 2);
    await Promise.resolve();
    expect(send).not.toHaveBeenCalled();
    stop();
    window.dispatchEvent(new Event("blur"));
    expect(send).not.toHaveBeenCalled();
  });
});
