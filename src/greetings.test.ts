import { describe, expect, it } from "vitest";
import { pickGreeting } from "./greetings";

// A fixed evening timestamp keeps the time-bucketed pool stable across runs.
const EVENING = new Date(2026, 0, 1, 19, 0, 0);

describe("pickGreeting", () => {
  it("is stable for a given seed", () => {
    const a = pickGreeting("session-abc", "harness", EVENING);
    const b = pickGreeting("session-abc", "harness", EVENING);
    expect(a.headline).toBe(b.headline);
    expect(a.hint).toBe(b.hint);
  });

  it("names the project in project-titled lines, with a project part for the dotted underline", () => {
    // Sweep seeds until we land on a project-titled line, then assert the
    // parts carry a `project` segment holding the exact name (no `{project}`).
    let sawProjectLine = false;
    for (let index = 0; index < 200 && !sawProjectLine; index += 1) {
      const greeting = pickGreeting(`seed-${index}`, "harness", EVENING);
      const projectPart = greeting.parts.find(part => part.kind === "project");
      if (projectPart) {
        sawProjectLine = true;
        expect(projectPart.text).toBe("harness");
        expect(greeting.headline).toContain("harness");
        expect(greeting.headline).not.toContain("{project}");
      }
      // No headline ever leaks the raw token.
      expect(greeting.headline).not.toContain("{project}");
    }
    expect(sawProjectLine).toBe(true);
  });

  it("never renders a project part when no project name is known", () => {
    for (let index = 0; index < 50; index += 1) {
      const greeting = pickGreeting(`seed-${index}`, undefined, EVENING);
      expect(greeting.parts.every(part => part.kind === "text")).toBe(true);
      expect(greeting.headline).not.toContain("{project}");
    }
  });

  it("tints the pool by time of day", () => {
    // Night adds nocturnal lines the daytime buckets never surface, so the same
    // seed can resolve to a different headline across buckets.
    const night = new Date(2026, 0, 1, 2, 0, 0);
    const morning = new Date(2026, 0, 1, 8, 0, 0);
    const seeds = Array.from({ length: 40 }, (_, index) => `t-${index}`);
    const nightSet = new Set(seeds.map(seed => pickGreeting(seed, undefined, night).headline));
    const morningSet = new Set(seeds.map(seed => pickGreeting(seed, undefined, morning).headline));
    // The two time buckets do not produce identical headline sets.
    expect([...nightSet].some(line => !morningSet.has(line))).toBe(true);
  });
});
