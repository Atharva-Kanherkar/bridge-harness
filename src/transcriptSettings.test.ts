import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AUTO_EXPAND_EDIT_ACTIVITY_STORAGE_KEY, readAutoExpandEditActivity, readShowThinking, SHOW_THINKING_STORAGE_KEY, writeAutoExpandEditActivity, writeShowThinking } from "./transcriptSettings";

beforeEach(() => {
  const store = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => store.get(key) ?? null,
    setItem: (key: string, value: string) => store.set(key, value),
    removeItem: (key: string) => store.delete(key),
  });
});

afterEach(() => vi.unstubAllGlobals());

describe("show-thinking preference", () => {
  it("defaults to on when nothing is stored", () => {
    expect(readShowThinking()).toBe(true);
  });

  it("reads back what it wrote", () => {
    writeShowThinking(false);
    expect(localStorage.getItem(SHOW_THINKING_STORAGE_KEY)).toBe("false");
    expect(readShowThinking()).toBe(false);
    writeShowThinking(true);
    expect(readShowThinking()).toBe(true);
  });

  it("treats a storage failure as the default rather than an error", () => {
    expect(readShowThinking({ getItem: () => { throw new Error("denied"); } })).toBe(true);
    expect(() => writeShowThinking(false, { setItem: () => { throw new Error("denied"); } })).not.toThrow();
  });
});

describe("automatic edit-activity expansion", () => {
  it("defaults to off and only recognizes an explicit opt-in", () => {
    expect(readAutoExpandEditActivity()).toBe(false);
    localStorage.setItem(AUTO_EXPAND_EDIT_ACTIVITY_STORAGE_KEY, "invalid");
    expect(readAutoExpandEditActivity()).toBe(false);
  });

  it("reads back both switch positions", () => {
    writeAutoExpandEditActivity(true);
    expect(localStorage.getItem(AUTO_EXPAND_EDIT_ACTIVITY_STORAGE_KEY)).toBe("true");
    expect(readAutoExpandEditActivity()).toBe(true);
    writeAutoExpandEditActivity(false);
    expect(readAutoExpandEditActivity()).toBe(false);
  });

  it("stays off when storage is unavailable", () => {
    expect(readAutoExpandEditActivity({ getItem: () => { throw new Error("denied"); } })).toBe(false);
    expect(() => writeAutoExpandEditActivity(true, { setItem: () => { throw new Error("denied"); } })).not.toThrow();
  });
});
