import { describe, expect, it } from "vitest";
import { EFFORT_SELECTOR_STORAGE_KEY, isEffortSelectorStyle, isThemePreference, isThemeSkin, readEffortSelectorStyle, readThemePreference, readThemeSkin, resolveTheme, SKIN_STORAGE_KEY, THEME_STORAGE_KEY, writeEffortSelectorStyle, writeThemePreference, writeThemeSkin } from "./theme";

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

describe("isThemeSkin", () => {
  it("accepts the supported skins", () => {
    expect(isThemeSkin("graphite")).toBe(true);
    expect(isThemeSkin("vibrancy")).toBe(true);
  });

  it("rejects anything else", () => {
    expect(isThemeSkin("cursor")).toBe(false);
    expect(isThemeSkin(null)).toBe(false);
    expect(isThemeSkin(undefined)).toBe(false);
  });
});

describe("readThemeSkin", () => {
  it("defaults to graphite when nothing is stored", () => {
    expect(readThemeSkin(fakeStorage())).toBe("graphite");
  });

  it("reads a stored skin", () => {
    expect(readThemeSkin(fakeStorage({ [SKIN_STORAGE_KEY]: "vibrancy" }))).toBe("vibrancy");
  });

  it("falls back to graphite for a corrupt value", () => {
    expect(readThemeSkin(fakeStorage({ [SKIN_STORAGE_KEY]: "glass" }))).toBe("graphite");
  });

  it("falls back to graphite when storage throws", () => {
    const hostile = { getItem: () => { throw new Error("denied"); } };
    expect(readThemeSkin(hostile)).toBe("graphite");
  });
});

describe("writeThemeSkin", () => {
  it("persists the skin", () => {
    const storage = fakeStorage();
    writeThemeSkin("vibrancy", storage);
    expect(storage.store[SKIN_STORAGE_KEY]).toBe("vibrancy");
  });

  it("does not throw when storage is read-only", () => {
    const hostile = { setItem: () => { throw new Error("denied"); } };
    expect(() => writeThemeSkin("vibrancy", hostile)).not.toThrow();
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

describe("isEffortSelectorStyle", () => {
  it("accepts the three shipped styles", () => {
    expect(isEffortSelectorStyle("slider")).toBe(true);
    expect(isEffortSelectorStyle("sentence")).toBe(true);
    expect(isEffortSelectorStyle("list")).toBe(true);
  });

  it("rejects anything else", () => {
    expect(isEffortSelectorStyle("segmented")).toBe(false);
    expect(isEffortSelectorStyle(null)).toBe(false);
    expect(isEffortSelectorStyle(undefined)).toBe(false);
  });
});

describe("readEffortSelectorStyle", () => {
  it("defaults to the slider when nothing is stored", () => {
    expect(readEffortSelectorStyle(fakeStorage())).toBe("slider");
  });

  it("reads a stored style", () => {
    expect(readEffortSelectorStyle(fakeStorage({ [EFFORT_SELECTOR_STORAGE_KEY]: "list" }))).toBe("list");
  });

  it("falls back to the slider for a corrupt value", () => {
    expect(readEffortSelectorStyle(fakeStorage({ [EFFORT_SELECTOR_STORAGE_KEY]: "dial" }))).toBe("slider");
  });

  it("falls back to the slider when storage throws", () => {
    const hostile = { getItem: () => { throw new Error("denied"); } };
    expect(readEffortSelectorStyle(hostile)).toBe("slider");
  });
});

describe("writeEffortSelectorStyle", () => {
  it("persists the style", () => {
    const storage = fakeStorage();
    writeEffortSelectorStyle("sentence", storage);
    expect(storage.store[EFFORT_SELECTOR_STORAGE_KEY]).toBe("sentence");
  });

  it("does not throw when storage is read-only", () => {
    const hostile = { setItem: () => { throw new Error("denied"); } };
    expect(() => writeEffortSelectorStyle("sentence", hostile)).not.toThrow();
  });
});
