import { describe, expect, it } from "vitest";
import { compactTokens, contextTone, windowComposition } from "./contextWindows";
import type { ContextWindowReading } from "./protocol/generated/protocol";

const reading = (overrides: Partial<ContextWindowReading> = {}): ContextWindowReading => ({
  usedTokens: 142_000, windowTokens: 1_000_000, percent: 14, state: "measured", source: "claude.context_usage",
  observedAt: "now", turnId: null, autoCompactTokens: 955_000, compactionOwner: "harness",
  segments: [
    { name: "Free space", tokens: 858_000, kind: "free" },
    { name: "Messages", tokens: 61_000, kind: "used" },
    { name: "Autocompact buffer", tokens: 45_000, kind: "buffer" },
    { name: "Tool results", tokens: 38_000, kind: "used" },
    { name: "Deferred tools", tokens: 3_000, kind: "deferred" },
  ],
  consumers: [], forecast: null, ...overrides,
});

describe("windowComposition", () => {
  it("draws only occupying segments and hatches the unattributed remainder", () => {
    const composition = windowComposition(reading());
    expect(composition.used.map(segment => segment.name)).toEqual(["Messages", "Tool results"]);
    expect(composition.unattributed).toBe(142_000 - 61_000 - 38_000);
    expect(composition.free).toBe(858_000);
  });

  it("hatches everything when the harness does not split its window", () => {
    const composition = windowComposition(reading({ segments: [], usedTokens: 90_000, windowTokens: 272_000 }));
    expect(composition.used).toEqual([]);
    expect(composition.unattributed).toBe(90_000);
    expect(composition.free).toBe(182_000);
  });

  it("never draws past the window", () => {
    const composition = windowComposition(reading({ segments: [], usedTokens: 300_000, windowTokens: 200_000 }));
    expect(composition.unattributed).toBe(200_000);
    expect(composition.free).toBe(0);
  });
});

describe("helpers", () => {
  it("formats token counts compactly", () => {
    expect(compactTokens(950)).toBe("950");
    expect(compactTokens(52_400)).toBe("52.4k");
    expect(compactTokens(200_000)).toBe("200k");
    expect(compactTokens(1_000_000)).toBe("1M");
    expect(compactTokens(1_500_000)).toBe("1.5M");
  });

  it("maps percent to the pressure levels", () => {
    expect(contextTone(null)).toBe("unknown");
    expect(contextTone(59)).toBe("healthy");
    expect(contextTone(60)).toBe("elevated");
    expect(contextTone(75)).toBe("high");
    expect(contextTone(90)).toBe("critical");
  });
});
