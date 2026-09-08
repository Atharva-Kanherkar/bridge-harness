import { describe, expect, it } from "vitest";
import { carryEffort, effortIndex, effortLabel, effortLevelsFrom, effortMeaning, effortWord, supportedEffortLevelsOf } from "./effortLevels";

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

describe("effort across a model switch", () => {
  const adapters = [
    { id: "claude", defaultModel: "sonnet", models: [
      { id: "sonnet", label: "Sonnet", tier: "standard" as const, defaultForTier: true, supportedEffortLevels: ["low", "high", "xhigh"] },
      { id: "haiku", label: "Haiku", tier: "fast" as const, defaultForTier: true },
    ] },
  ];

  it("resolves the adapter's default model when none is named", () => {
    expect(supportedEffortLevelsOf(adapters, "claude", null)).toEqual(["low", "high", "xhigh"]);
    expect(supportedEffortLevelsOf(adapters, "claude", "haiku")).toEqual([]);
    expect(supportedEffortLevelsOf(adapters, "claude", "retired")).toEqual([]);
    expect(supportedEffortLevelsOf(adapters, "codex", null)).toEqual([]);
  });

  it("keeps a level the new model supports and drops one it does not", () => {
    expect(carryEffort(["low", "high", "xhigh"], "high")).toBe("high");
    expect(carryEffort(["low", "high", "xhigh"], "max")).toBeUndefined();
    expect(carryEffort([], "high")).toBeUndefined();
    expect(carryEffort(["low"], null)).toBeUndefined();
  });
});
