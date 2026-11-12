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

type Deferred<T> = { promise: Promise<T>; resolve: (value: T) => void; reject: (error: unknown) => void };
function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}

/** A core whose reads, saves and hints the test drives by hand. */
function scriptedCore(initial: AppPreferences) {
  let record = initial;
  let notify: () => void = () => undefined;
  const saves: AppPreferenceValues[] = [];
  const state = {
    failSaves: false,
    failReads: false,
    record: () => record,
    set: (next: AppPreferences) => { record = next; },
    notify: () => notify(),
    saves,
  };
  const api: AppPreferencesApi = {
    getAppPreferences: vi.fn(async () => {
      if (state.failReads) throw new Error("not attached");
      return { revision: record.revision, values: { ...record.values } };
    }),
    saveAppPreferences: vi.fn(async (values: AppPreferenceValues) => {
      saves.push(values);
      if (state.failSaves) throw new Error("offline");
      record = { revision: record.revision + 1, values: { ...record.values, ...values } };
      return record;
    }),
    onAppPreferencesChanged: async handler => { notify = handler; return () => { notify = () => undefined; }; },
  };
  return { api, state };
}

/** What Settings does in one window: write the shared cache, then tell that window. */
function chooseTheme(storage: MemoryStorage, target: EventTarget, theme: string) {
  storage.setItem("bridge.theme", theme);
  target.dispatchEvent(new Event(THEME_EVENT));
}

describe("app preferences across windows and a flaky backend", () => {
  it("repaints a second window even when the first already wrote the shared cache", async () => {
    const storage = new MemoryStorage();
    storage.setItem("bridge.theme", "light");
    const core = fakeCore({ theme: "light" });
    const first = new EventTarget();
    const second = new EventTarget();
    const stopFirst = startAppPreferenceSync(core.api, storage, first as unknown as Window);
    const stopSecond = startAppPreferenceSync(core.api, storage, second as unknown as Window);
    await settle();
    const repainted = vi.fn();
    second.addEventListener(THEME_EVENT, repainted);
    chooseTheme(storage, first, "dark");
    await settle();
    expect(core.values().theme).toBe("dark");
    expect(repainted).toHaveBeenCalled();
    stopFirst();
    stopSecond();
  });

  it("ignores a snapshot older than one it already accepted", async () => {
    const storage = new MemoryStorage();
    const reads: Deferred<AppPreferences>[] = [];
    let notify: () => void = () => undefined;
    const api: AppPreferencesApi = {
      getAppPreferences: () => { const read = deferred<AppPreferences>(); reads.push(read); return read.promise; },
      saveAppPreferences: vi.fn(async values => ({ revision: 99, values })),
      onAppPreferencesChanged: async handler => { notify = handler; return () => undefined; },
    };
    const stop = startAppPreferenceSync(api, storage, new EventTarget() as unknown as Window);
    reads[0].resolve({ revision: 1, values: { theme: "light" } });
    await settle();
    notify();
    notify();
    // Replies on separate lanes: the newer read answers first.
    reads[2].resolve({ revision: 3, values: { theme: "dark" } });
    await settle();
    reads[1].resolve({ revision: 2, values: { theme: "light" } });
    await settle();
    expect(storage.getItem("bridge.theme")).toBe("dark");
    stop();
  });

  it("lands a newer change made elsewhere while this window's save was in flight", async () => {
    const storage = new MemoryStorage();
    const target = new EventTarget();
    let record: AppPreferences = { revision: 1, values: { theme: "light" } };
    let notify: () => void = () => undefined;
    const save = deferred<AppPreferences>();
    const api: AppPreferencesApi = {
      getAppPreferences: async () => record,
      saveAppPreferences: vi.fn(() => save.promise),
      onAppPreferencesChanged: async handler => { notify = handler; return () => undefined; },
    };
    const stop = startAppPreferenceSync(api, storage, target as unknown as Window);
    await settle();
    chooseTheme(storage, target, "dark");
    await settle();
    // Our save lands as revision 2, then another window stores system as 3.
    record = { revision: 3, values: { theme: "system" } };
    notify();
    await settle();
    save.resolve({ revision: 2, values: { theme: "dark" } });
    await settle();
    expect(storage.getItem("bridge.theme")).toBe("system");
    stop();
  });

  it("keeps a choice whose save failed and retries it instead of taking the core's older value", async () => {
    const storage = new MemoryStorage();
    const target = new EventTarget();
    const core = scriptedCore({ revision: 1, values: { theme: "light" } });
    const stop = startAppPreferenceSync(core.api, storage, target as unknown as Window);
    await settle();
    core.state.failSaves = true;
    chooseTheme(storage, target, "dark");
    await settle();
    expect(storage.getItem("bridge.theme")).toBe("dark");
    // The connection comes back and the host sends its reconciliation hint.
    // The core's older light must never be painted over the person's choice,
    // not even for the moment before the retried save lands.
    const written: string[] = [];
    const setItem = storage.setItem.bind(storage);
    storage.setItem = (key: string, value: string) => { if (key === "bridge.theme") written.push(value); setItem(key, value); };
    core.state.failSaves = false;
    core.state.notify();
    await settle();
    expect(written).not.toContain("light");
    expect(storage.getItem("bridge.theme")).toBe("dark");
    expect(core.state.record().values.theme).toBe("dark");
    expect(core.state.saves).toEqual([{ theme: "dark" }, { theme: "dark" }]);
    stop();
  });
});
