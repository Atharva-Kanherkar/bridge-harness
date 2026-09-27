import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { readShowThinking, SHOW_THINKING_STORAGE_KEY, writeShowThinking } from "./transcriptSettings";

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
