import { describe, expect, it } from "vitest";
import { REVEAL_MS, revealedLength, revealStart } from "./smoothText";

describe("revealedLength", () => {
  it("never reveals past the target", () => {
    for (const elapsed of [0, 1, REVEAL_MS / 2, REVEAL_MS, REVEAL_MS * 10]) {
      expect(revealedLength(3, 40, elapsed)).toBeLessThanOrEqual(40);
    }
  });

  it("drains the whole backlog within the window", () => {
    expect(revealedLength(0, 5_000, REVEAL_MS)).toBe(5_000);
    expect(revealedLength(10, 11, REVEAL_MS)).toBe(11);
  });

  it("moves on every frame, and faster for a bigger backlog", () => {
    expect(revealedLength(0, 1, 16)).toBe(1);
    const small = revealedLength(0, 20, 16);
    const large = revealedLength(0, 2_000, 16);
    expect(small).toBeGreaterThan(0);
    expect(large).toBeGreaterThan(small);
  });

  it("starts where it stands and treats a negative clock as no progress", () => {
    expect(revealedLength(12, 30, -50)).toBe(12);
    expect(revealedLength(30, 12, 0)).toBe(12);
  });
});

describe("revealStart", () => {
  it("continues from the revealed prefix while text extends", () => {
    expect(revealStart("Hello", 3, "Hello, world", true)).toBe(3);
  });

  it("snaps to the full text when the text was replaced", () => {
    expect(revealStart("Hello", 5, "Goodbye", true)).toBe(7);
  });

  it("snaps to the full text once the message is no longer streaming", () => {
    expect(revealStart("Hello", 2, "Hello there", false)).toBe(11);
  });
});
