import { afterEach, describe, expect, it } from "vitest";
import {
  AGENTS_PANE_STORAGE_KEY, addAgentIds, agentsPaneSettingsWith, defaultAgentsPaneSettings,
  readAgentsPaneSettings, toggleAgentId, writeAgentsPaneSettings,
} from "./agentsPaneSettings";

// Contract: testing/feat-agents-pane.md §2.

const memory = (initial: Record<string, string> = {}) => {
  const store = new Map(Object.entries(initial));
  return {
    getItem: (key: string) => store.get(key) ?? null,
    setItem: (key: string, value: string) => { store.set(key, value); },
    store,
  };
};

afterEach(() => {
  try { localStorage.clear(); } catch { /* no storage in this environment */ }
});

describe("agentsPaneSettings", () => {
  it("round-trips pinned, expanded and acknowledged ids and the scope", () => {
    const storage = memory();
    writeAgentsPaneSettings({ pinned: ["w1"], expanded: ["w2"], acknowledged: ["w3"], scope: "all-chats" }, storage);
    expect(readAgentsPaneSettings(storage)).toEqual({ pinned: ["w1"], expanded: ["w2"], acknowledged: ["w3"], scope: "all-chats" });
  });

  it("is stored under one key, not one per chat", () => {
    const storage = memory();
    writeAgentsPaneSettings({ ...defaultAgentsPaneSettings(), pinned: ["w1"] }, storage);
    // Reading it back for a different chat returns the same record, because
    // there is no chat in the key to read it for.
    expect(readAgentsPaneSettings(storage).pinned).toEqual(["w1"]);
    expect([...storage.store.keys()]).toEqual([AGENTS_PANE_STORAGE_KEY]);
  });

  it("degrades to defaults on anything malformed", () => {
    for (const raw of ["not json", "[]", '"a string"', "null", "{}", JSON.stringify({ pinned: "no", scope: "nope", expanded: [1, 2] })]) {
      const storage = memory({ [AGENTS_PANE_STORAGE_KEY]: raw });
      const read = readAgentsPaneSettings(storage);
      expect(read.pinned).toEqual([]);
      expect(read.expanded).toEqual([]);
      expect(read.acknowledged).toEqual([]);
      expect(read.scope).toBe("this-chat");
    }
  });

  it("keeps a pinned id the model no longer knows", () => {
    const storage = memory();
    writeAgentsPaneSettings({ ...defaultAgentsPaneSettings(), pinned: ["gone"] }, storage);
    // Unpinning is the only way to drop one: a chat switch must not empty the
    // tray, and only this module ever prunes.
    expect(readAgentsPaneSettings(storage).pinned).toEqual(["gone"]);
  });

  it("survives a storage that refuses to write", () => {
    expect(() => writeAgentsPaneSettings(defaultAgentsPaneSettings(), {
      setItem: () => { throw new Error("read-only"); },
    })).not.toThrow();
    expect(readAgentsPaneSettings({ getItem: () => { throw new Error("no storage"); } })).toEqual(defaultAgentsPaneSettings());
  });

  it("toggles without duplicating, and adds idempotently", () => {
    expect(toggleAgentId(["a", "b"], "c")).toEqual(["a", "b", "c"]);
    expect(toggleAgentId(["a", "b"], "a")).toEqual(["b"]);
    expect(addAgentIds(["a"], ["a", "b"])).toEqual(["a", "b"]);
    expect(agentsPaneSettingsWith(defaultAgentsPaneSettings(), { scope: "all-chats" }).scope).toBe("all-chats");
  });
});
