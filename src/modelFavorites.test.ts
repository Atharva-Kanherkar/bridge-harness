import { describe, expect, it } from "vitest";
import { isModelFavorite, MODEL_FAVORITES_STORAGE_KEY, readModelFavorites, toggleModelFavorite, writeModelFavorites } from "./modelFavorites";

const fakeStorage = (initial: Record<string, string> = {}) => {
  const store = { ...initial };
  return {
    store,
    getItem: (key: string) => store[key] ?? null,
    setItem: (key: string, value: string) => { store[key] = value; },
  };
};

const luna = { harness: "codex", model: "gpt-5.6-luna", account: null };
const opusPersonal = { harness: "claude", model: "opus", account: "personal" };

describe("readModelFavorites", () => {
  it("is empty when nothing is stored", () => {
    expect(readModelFavorites(fakeStorage())).toEqual([]);
  });

  it("reads what was written, account included", () => {
    const storage = fakeStorage();
    expect(writeModelFavorites([luna, opusPersonal], storage)).toBe(true);
    expect(readModelFavorites(storage)).toEqual([luna, opusPersonal]);
  });

  it("drops corrupt JSON, a non-list and malformed entries", () => {
    expect(readModelFavorites(fakeStorage({ [MODEL_FAVORITES_STORAGE_KEY]: "{not json" }))).toEqual([]);
    expect(readModelFavorites(fakeStorage({ [MODEL_FAVORITES_STORAGE_KEY]: JSON.stringify({ harness: "codex" }) }))).toEqual([]);
    const mixed = [luna, { harness: "codex" }, { harness: "", model: "x" }, { harness: "claude", model: "opus", account: 7 }, "opus", null];
    expect(readModelFavorites(fakeStorage({ [MODEL_FAVORITES_STORAGE_KEY]: JSON.stringify(mixed) }))).toEqual([luna]);
  });

  it("treats a missing or empty account as the user's own login and dedupes", () => {
    const stored = [{ harness: "codex", model: "gpt-5.6-luna" }, { harness: "codex", model: "gpt-5.6-luna", account: "" }, luna];
    expect(readModelFavorites(fakeStorage({ [MODEL_FAVORITES_STORAGE_KEY]: JSON.stringify(stored) }))).toEqual([luna]);
  });

  it("caps a runaway list", () => {
    const many = Array.from({ length: 80 }, (_, i) => ({ harness: "opencode", model: `m${i}`, account: null }));
    expect(readModelFavorites(fakeStorage({ [MODEL_FAVORITES_STORAGE_KEY]: JSON.stringify(many) }))).toHaveLength(50);
  });

  it("tolerates storage that throws or is missing", () => {
    expect(readModelFavorites({ getItem: () => { throw new Error("denied"); } })).toEqual([]);
    expect(readModelFavorites(null)).toEqual([]);
  });
});

describe("writeModelFavorites", () => {
  it("reports a read-only storage instead of throwing", () => {
    expect(writeModelFavorites([luna], { setItem: () => { throw new Error("quota"); } })).toBe(false);
    expect(writeModelFavorites([luna], null)).toBe(false);
  });
});

describe("toggleModelFavorite", () => {
  it("adds at the end and removes on the second toggle", () => {
    const once = toggleModelFavorite([luna], opusPersonal);
    expect(once).toEqual([luna, opusPersonal]);
    expect(toggleModelFavorite(once, luna)).toEqual([opusPersonal]);
  });

  it("keeps the same model under another account as its own favorite", () => {
    const own = { ...opusPersonal, account: null };
    const both = toggleModelFavorite([opusPersonal], own);
    expect(both).toHaveLength(2);
    expect(isModelFavorite(both, own)).toBe(true);
    expect(isModelFavorite([opusPersonal], own)).toBe(false);
  });
});
