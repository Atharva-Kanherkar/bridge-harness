import { describe, expect, it } from "vitest";
import { isThemePreference, readThemePreference, resolveTheme, THEME_STORAGE_KEY, writeThemePreference } from "./theme";

const fakeStorage = (initial: Record<string, string> = {}) => {
  const store = { ...initial };
  return {
    store,
    getItem: (key: string) => store[key] ?? null,
    setItem: (key: string, value: string) => { store[key] = value; },
  };
};

describe("isThemePreference", () => {
  it("accepts the three supported preferences", () => {
    expect(isThemePreference("system")).toBe(true);
    expect(isThemePreference("light")).toBe(true);
    expect(isThemePreference("dark")).toBe(true);
  });

  it("rejects anything else", () => {
    expect(isThemePreference("graphite")).toBe(false);
    expect(isThemePreference(null)).toBe(false);
    expect(isThemePreference(undefined)).toBe(false);
  });
});

describe("readThemePreference", () => {
  it("defaults to system when nothing is stored", () => {
    expect(readThemePreference(fakeStorage())).toBe("system");
  });

  it("reads a stored preference", () => {
    expect(readThemePreference(fakeStorage({ [THEME_STORAGE_KEY]: "light" }))).toBe("light");
  });

  it("falls back to system for a corrupt value", () => {
    expect(readThemePreference(fakeStorage({ [THEME_STORAGE_KEY]: "neon" }))).toBe("system");
  });

  it("falls back to system when storage throws", () => {
    const hostile = { getItem: () => { throw new Error("denied"); } };
    expect(readThemePreference(hostile)).toBe("system");
  });
});

describe("writeThemePreference", () => {
  it("persists the preference", () => {
    const storage = fakeStorage();
    writeThemePreference("dark", storage);
    expect(storage.store[THEME_STORAGE_KEY]).toBe("dark");
  });

  it("does not throw when storage is read-only", () => {
    const hostile = { setItem: () => { throw new Error("denied"); } };
    expect(() => writeThemePreference("dark", hostile)).not.toThrow();
  });
});

describe("resolveTheme", () => {
  it("follows the system signal when the preference is system", () => {
    expect(resolveTheme("system", true)).toBe("dark");
    expect(resolveTheme("system", false)).toBe("light");
  });

  it("pins the chosen mode regardless of the system signal", () => {
    expect(resolveTheme("light", true)).toBe("light");
    expect(resolveTheme("dark", false)).toBe("dark");
  });
});
