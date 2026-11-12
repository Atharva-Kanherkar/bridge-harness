// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
import type { AppPreferences, AppPreferenceValues } from "./protocol/generated/protocol";
import { applyLocalPreferences, changedKeys, readLocalPreferences, startAppPreferenceSync, type AppPreferencesApi } from "./appPreferences";
import { THEME_EVENT } from "./theme";
import { SHOW_THINKING_EVENT } from "./transcriptSettings";

class MemoryStorage {
  items = new Map<string, string>();
  getItem(key: string) { return this.items.get(key) ?? null; }
  setItem(key: string, value: string) { this.items.set(key, value); }
}

/** A core that keeps one record and tells subscribers when it moves. */
function fakeCore(initial: AppPreferenceValues = {}) {
  let record: AppPreferences = { revision: 0, values: { ...initial } };
  const listeners = new Set<() => void>();
  const saves: AppPreferenceValues[] = [];
  const api: AppPreferencesApi = {
    getAppPreferences: vi.fn(async () => ({ revision: record.revision, values: { ...record.values } })),
    saveAppPreferences: vi.fn(async (values: AppPreferenceValues) => {
      saves.push(values);
      record = { revision: record.revision + 1, values: { ...record.values, ...values } };
      listeners.forEach(listener => listener());
      return record;
    }),
    onAppPreferencesChanged: async handler => { listeners.add(handler); return () => listeners.delete(handler); },
  };
  /** A change made somewhere else: a chat, another window. */
  const setElsewhere = (values: AppPreferenceValues) => {
    record = { revision: record.revision + 1, values: { ...record.values, ...values } };
    listeners.forEach(listener => listener());
  };
  return { api, saves, setElsewhere, values: () => record.values };
}

const settle = () => new Promise(resolve => setTimeout(resolve, 0));

describe("app preferences", () => {
  it("reads only what this window explicitly stored", () => {
    const storage = new MemoryStorage();
    expect(readLocalPreferences(storage)).toEqual({});
    storage.setItem("bridge.theme", "dark");
    storage.setItem("bridge.skin", "nonsense");
    storage.setItem("bridge.transcript.showThinking", "false");
    expect(readLocalPreferences(storage)).toEqual({ theme: "dark", showThinking: false });
  });

  it("reports only keys that differ", () => {
    expect(changedKeys({ theme: "dark", showThinking: true }, { theme: "dark", showThinking: false })).toEqual({ showThinking: true });
  });

  it("writes the cache and fires the events consumers already listen to", () => {
    const storage = new MemoryStorage();
    const target = new EventTarget();
    const heard: string[] = [];
    target.addEventListener(THEME_EVENT, () => heard.push("theme"));
    target.addEventListener(SHOW_THINKING_EVENT, () => heard.push("transcript"));
    applyLocalPreferences({ theme: "light" }, storage, target as unknown as Window);
    expect(storage.getItem("bridge.theme")).toBe("light");
    expect(heard).toEqual(["theme"]);
    applyLocalPreferences({ autoExpandEdits: true }, storage, target as unknown as Window);
    expect(storage.getItem("bridge.transcript.autoExpandEditActivity")).toBe("true");
    expect(heard).toEqual(["theme", "transcript"]);
  });

  it("applies the core's values on start and seeds only keys the core lacks", async () => {
    const storage = new MemoryStorage();
    storage.setItem("bridge.theme", "dark");
    storage.setItem("bridge.skin", "vibrancy");
    const core = fakeCore({ theme: "light" });
    const stop = startAppPreferenceSync(core.api, storage, new EventTarget() as unknown as Window);
    await settle();
    expect(storage.getItem("bridge.theme")).toBe("light");
    expect(core.saves).toEqual([{ skin: "vibrancy" }]);
    // Defaults this window never chose are not promoted into choices.
    expect(core.values()).toEqual({ theme: "light", skin: "vibrancy" });
    stop();
  });

  it("follows a change made elsewhere without echoing it back", async () => {
    const storage = new MemoryStorage();
    const target = new EventTarget();
    const core = fakeCore();
    const heard = vi.fn();
    target.addEventListener(THEME_EVENT, heard);
    const stop = startAppPreferenceSync(core.api, storage, target as unknown as Window);
    await settle();
    core.setElsewhere({ theme: "light", showThinking: false });
    await settle();
    expect(storage.getItem("bridge.theme")).toBe("light");
    expect(storage.getItem("bridge.transcript.showThinking")).toBe("false");
    expect(heard).toHaveBeenCalledTimes(1);
    expect(core.saves).toEqual([]);
    stop();
  });

  it("saves a change made in this window", async () => {
    const storage = new MemoryStorage();
    const target = new EventTarget();
    const core = fakeCore({ theme: "light" });
    const stop = startAppPreferenceSync(core.api, storage, target as unknown as Window);
    await settle();
    // What Settings does: write the cache, then fire the event.
    storage.setItem("bridge.theme", "dark");
    target.dispatchEvent(new Event(THEME_EVENT));
    await settle();
    expect(core.saves).toEqual([{ theme: "dark" }]);
    expect(core.values().theme).toBe("dark");
    stop();
  });

  it("stops listening when stopped", async () => {
    const storage = new MemoryStorage();
    const core = fakeCore();
    const stop = startAppPreferenceSync(core.api, storage, new EventTarget() as unknown as Window);
    await settle();
    stop();
    core.setElsewhere({ theme: "dark" });
    await settle();
    expect(storage.getItem("bridge.theme")).toBeNull();
  });
});
