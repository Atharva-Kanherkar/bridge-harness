// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { __snapshotForTests, badgesOf, COALESCE_MS, enabledApps, loadApps, pinsOf, PINS_KEY, resetAppsStore, subscribeApps } from "./store";
import { emitMockAppsChanged, mockListApps, mockSetAppEnabled, resetMockApps } from "../appsMock";
import { appsBadge, BUILTIN_APPS } from "./registry";
import { bridgeApi } from "../api";
import type { ListAppsResult } from "../protocol/generated/protocol";

beforeEach(() => { localStorage.clear(); resetMockApps(); resetAppsStore(); });
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); localStorage.clear(); resetMockApps(); resetAppsStore(); });

const flush = () => new Promise(resolve => setTimeout(resolve, 0));

describe("views over core's list", () => {
  it("reads as a fresh install before the first answer", () => {
    expect(enabledApps(null)).toBe(BUILTIN_APPS);
    expect(pinsOf(null)).toEqual([]);
    expect(badgesOf(null)).toEqual({});
  });

  it("drops apps that are off, keeps pin order, and sums the badges core sent", () => {
    mockSetAppEnabled("workflows", false);
    const apps = mockListApps({ workflows: 4, memory: 2, trails: 1 }).apps;
    expect(enabledApps(apps).map(app => app.id)).not.toContain("workflows");
    const badges = badgesOf(apps);
    expect(badges).toEqual({ memory: { count: 2 }, trails: { count: 1 } });
    expect(appsBadge(badges)?.count).toBe("3");
    const pinned = apps.map(app => app.manifest.id === "usage" ? { ...app, pinned: 1 } : app.manifest.id === "notes" ? { ...app, pinned: 0 } : app);
    expect(pinsOf(pinned)).toEqual(["notes", "usage"]);
  });
});

describe("the store", () => {
  it("rereads on apps/changed, once per burst", async () => {
    const list = vi.spyOn(bridgeApi, "listApps");
    const off = subscribeApps(() => undefined);
    await flush();
    expect(list).toHaveBeenCalledTimes(1);
    vi.useFakeTimers();
    emitMockAppsChanged();
    emitMockAppsChanged();
    emitMockAppsChanged();
    vi.advanceTimersByTime(COALESCE_MS);
    vi.useRealTimers();
    await flush();
    expect(list).toHaveBeenCalledTimes(2);
    off();
  });

  it("keeps the newest answer when an older one arrives late", async () => {
    const answers: Array<(value: ListAppsResult) => void> = [];
    vi.spyOn(bridgeApi, "listApps").mockImplementation(() => new Promise(resolve => answers.push(resolve)));
    const off = subscribeApps(() => undefined);
    const older = loadApps();
    const newer = loadApps();
    const offApp = (id: string) => ({ apps: mockListApps().apps.map(app => app.manifest.id === id ? { ...app, enabled: false } : app) });
    answers[answers.length - 1](offApp("usage"));
    await newer;
    answers[answers.length - 2](offApp("notes"));
    await older;
    const seen = enabledApps(__snapshotForTests()).map(app => app.id);
    expect(seen).toContain("notes");
    expect(seen).not.toContain("usage");
    off();
  });

  it("imports slice 1 pins after the first answer", async () => {
    localStorage.setItem(PINS_KEY, JSON.stringify(["pulls"]));
    const off = subscribeApps(() => undefined);
    for (let index = 0; index < 5; index += 1) await flush();
    expect(pinsOf((await bridgeApi.listApps()).apps)).toEqual(["pulls"]);
    expect(localStorage.getItem(PINS_KEY)).toBeNull();
    off();
  });
});
