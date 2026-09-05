import { describe, expect, it } from "vitest";
import { effortIndex, effortLabel, effortLevelsFrom, effortMeaning, effortWord } from "./effortLevels";

describe("effort vocabulary", () => {
  it("names the known ladder", () => {
    expect(["low", "medium", "high", "xhigh", "max", "ultra"].map(effortLabel)).toEqual(["Low", "Med", "High", "XHigh", "Max", "Ultra"]);
    expect(["low", "xhigh", "ultra"].map(effortWord)).toEqual(["lightly", "deeply", "relentlessly"]);
    expect(effortMeaning("max")).toContain("Claude");
  });

  it("never hides a wire value it has not seen", () => {
    expect(effortLabel("turbo")).toBe("Turbo");
    expect(effortWord("turbo")).toBe("turbo");
    expect(effortMeaning("turbo")).toBe("");
  });

  it("builds de-duplicated levels and locates the current one", () => {
    const levels = effortLevelsFrom(["low", "high", "high", "xhigh"]);
    expect(levels.map(level => level.value)).toEqual(["low", "high", "xhigh"]);
    expect(effortIndex(levels, "high")).toBe(1);
    expect(effortIndex(levels, "medium")).toBe(-1);
    expect(effortLevelsFrom(undefined)).toEqual([]);
  });
});
