// @vitest-environment jsdom
import { act, useLayoutEffect } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { applySkin, applyTheme, isThemeSkin, readThemeSkin, SKIN_STORAGE_KEY, useThemePreference } from "./theme";
import { clearOmarchyTheme, parseOmarchyPayload, receiveOmarchyTheme, resetOmarchyTheme, skinFontStack, SKIN_FONT_PROPERTY } from "./omarchy";
import { OMARCHY_TOKEN_NAMES } from "./lib/omarchyTheme";

const ROSE_PINE = {
  available: true,
  themeName: "rose-pine",
  mode: "light",
  colors: { background: "#faf4ed", foreground: "#575279", accent: "#56949f", dark_background: "#ede7e1" },
  font: "JetBrainsMono Nerd Font",
};

const root = () => document.documentElement;
const storage = (initial: Record<string, string> = {}) => ({ getItem: (key: string) => initial[key] ?? null });

beforeEach(() => {
  const store = new Map<string, string>();
  vi.stubGlobal("localStorage", { getItem: (key: string) => store.get(key) ?? null, setItem: (key: string, value: string) => store.set(key, value), removeItem: (key: string) => store.delete(key) });
  vi.stubGlobal("matchMedia", () => ({ matches: true, addEventListener() {}, removeEventListener() {} }));
});

afterEach(() => {
  resetOmarchyTheme();
  clearOmarchyTheme();
  root().className = "";
  delete root().dataset.skin;
  vi.unstubAllGlobals();
});

describe("omarchy skin", () => {
  it("is a valid skin but never the default without Omarchy", () => {
    expect(isThemeSkin("omarchy")).toBe(true);
    expect(readThemeSkin(storage())).toBe("graphite");
  });

  it("becomes the default once Omarchy is detected, unless the user picked a skin", () => {
    receiveOmarchyTheme(ROSE_PINE);
    expect(readThemeSkin(storage())).toBe("omarchy");
    expect(readThemeSkin(storage({ [SKIN_STORAGE_KEY]: "graphite" }))).toBe("graphite");
  });

  it("paints the payload's tokens and font and follows the theme's own mode", () => {
    receiveOmarchyTheme(ROSE_PINE);
    applySkin("omarchy");
    // The OS prefers dark (matchMedia stub); rose-pine is light and wins.
    expect(applyTheme("system")).toBe("light");
    expect(root().classList.contains("dark")).toBe(false);
    expect(root().style.getPropertyValue("--background")).toBe("#faf4ed");
    expect(root().style.getPropertyValue("--sidebar")).toBe("#ede7e1");
    expect(root().style.getPropertyValue("--primary")).toBe("#56949f");
    expect(root().style.getPropertyValue(SKIN_FONT_PROPERTY)).toBe('"JetBrainsMono Nerd Font", ui-monospace, monospace');
  });

  it("renders as graphite while no payload has arrived", () => {
    applySkin("omarchy");
    expect(root().dataset.skin).toBe("omarchy");
    expect(applyTheme("system")).toBe("dark");
    for (const name of OMARCHY_TOKEN_NAMES) expect(root().style.getPropertyValue(name)).toBe("");
  });

  it("removes every property when the user leaves the skin", () => {
    receiveOmarchyTheme(ROSE_PINE);
    applySkin("omarchy");
    applySkin("graphite");
    for (const name of OMARCHY_TOKEN_NAMES) expect(root().style.getPropertyValue(name)).toBe("");
    expect(root().style.getPropertyValue(SKIN_FONT_PROPERTY)).toBe("");
    expect(applyTheme("system")).toBe("dark");
  });
});

describe("parseOmarchyPayload", () => {
  it("rejects payloads that do not say Omarchy is available", () => {
    expect(parseOmarchyPayload(null)).toBeNull();
    expect(parseOmarchyPayload({ ...ROSE_PINE, available: false })).toBeNull();
    expect(parseOmarchyPayload({ available: true })).toBeNull();
  });

  it("drops non-hex colours and normalises the rest", () => {
    const payload = parseOmarchyPayload({ ...ROSE_PINE, mode: "neon", font: "  ", colors: { background: "#000", red: "url(evil)" } })!;
    expect(payload.colors).toEqual({ background: "#000" });
    expect(payload.mode).toBe("dark");
    expect(payload.font).toBeNull();
  });

  it("keeps a hostile font name to one quoted family", () => {
    expect(skinFontStack('Evil"; color: red; "')).toBe('"Evil; color: red; ", ui-monospace, monospace');
    expect(skinFontStack(null)).toBe("ui-monospace, monospace");
  });
});

describe("startup", () => {
  it("does not miss a payload that lands between render and the hook's subscription", async () => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    let seen = "";
    function Consumer() {
      seen = useThemePreference().skin;
      return null;
    }
    // Layout effects run before passive ones, so this delivers the payload
    // after render but before useThemePreference subscribes.
    function EarlyShell() {
      useLayoutEffect(() => receiveOmarchyTheme(ROSE_PINE), []);
      return null;
    }
    const host = document.createElement("div");
    const root = createRoot(host);
    await act(async () => {
      root.render(<><Consumer /><EarlyShell /></>);
    });
    expect(seen).toBe("omarchy");
    expect(document.documentElement.dataset.skin).toBe("omarchy");
    expect(document.documentElement.style.getPropertyValue("--background")).toBe("#faf4ed");
    act(() => root.unmount());
  });
});
