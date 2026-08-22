import { beforeEach, describe, expect, it } from "vitest";
import {
  clearScrollbackForTests,
  rememberScrollback,
  scrollbackFor,
  SCROLLBACK_PER_SESSION_LIMIT,
  SCROLLBACK_TOTAL_BUDGET,
} from "./terminalScrollback";

describe("rememberScrollback", () => {
  beforeEach(clearScrollbackForTests);

  it("caps a single session at its own limit, keeping the tail", () => {
    rememberScrollback("w1", "HEAD".padEnd(SCROLLBACK_PER_SESSION_LIMIT, "x"));
    rememberScrollback("w1", "TAIL");
    const kept = scrollbackFor("w1");
    expect(kept).toHaveLength(SCROLLBACK_PER_SESSION_LIMIT);
    expect(kept?.endsWith("TAIL")).toBe(true);
    expect(kept?.startsWith("HEAD")).toBe(false);
  });

  it("evicts the least recently active session once the global budget is hit", () => {
    const sessions = Math.ceil(SCROLLBACK_TOTAL_BUDGET / SCROLLBACK_PER_SESSION_LIMIT) + 2;
    for (let index = 0; index < sessions; index += 1) {
      rememberScrollback(`w${index}`, "y".repeat(SCROLLBACK_PER_SESSION_LIMIT));
    }
    expect(scrollbackFor("w0")).toBeUndefined();
    expect(scrollbackFor(`w${sessions - 1}`)).toBeDefined();
    let total = 0;
    for (let index = 0; index < sessions; index += 1) {
      total += scrollbackFor(`w${index}`)?.length ?? 0;
    }
    expect(total).toBeLessThanOrEqual(SCROLLBACK_TOTAL_BUDGET);
  });

  it("refreshes recency on append so an active shell survives", () => {
    rememberScrollback("active", "a".repeat(SCROLLBACK_PER_SESSION_LIMIT));
    for (let index = 0; index < 3; index += 1) {
      rememberScrollback(`idle${index}`, "i".repeat(SCROLLBACK_PER_SESSION_LIMIT));
      rememberScrollback("active", "+");
    }
    rememberScrollback("overflow", "o".repeat(SCROLLBACK_PER_SESSION_LIMIT));
    expect(scrollbackFor("active")).toBeDefined();
    expect(scrollbackFor("idle0")).toBeUndefined();
  });
});
