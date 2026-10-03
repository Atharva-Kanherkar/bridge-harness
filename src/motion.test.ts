import { afterEach, describe, expect, it, vi } from "vitest";
import { scrollBehavior } from "./motion";

const withMotionPreference = (reduce: boolean) => vi.stubGlobal("window", {
  matchMedia: (query: string) => ({ matches: reduce && query === "(prefers-reduced-motion: reduce)" }),
});

afterEach(() => { vi.unstubAllGlobals(); });

describe("scrollBehavior", () => {
  it("scrolls instantly when the user prefers reduced motion", () => {
    withMotionPreference(true);
    expect(scrollBehavior()).toBe("auto");
  });

  it("scrolls smoothly otherwise", () => {
    withMotionPreference(false);
    expect(scrollBehavior()).toBe("smooth");
  });

  it("scrolls smoothly where the preference cannot be read", () => {
    vi.stubGlobal("window", {});
    expect(scrollBehavior()).toBe("smooth");
  });
});
