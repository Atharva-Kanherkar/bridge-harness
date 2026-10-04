import { describe, expect, it } from "vitest";
import { enterAction, parseFindCommand } from "./chatSearch";
import type { SearchChatsResult } from "./protocol/generated/protocol";

const result = (overrides: Partial<SearchChatsResult> = {}): SearchChatsResult => ({
  query: "stall",
  hits: [{ sessionId: "a", title: "A", harness: "codex", lastActiveAt: "2026-09-29T00:00:00Z", matchCount: 2, snippet: "", score: 1, why: "topic matches", archived: false, ended: false }],
  stage: "index",
  confident: false,
  deepAvailable: true,
  terms: ["stall"],
  elapsedMs: 3,
  modelTokens: 0,
  toolCalls: 0,
  ...overrides,
});

describe("parseFindCommand", () => {
  it("reads the query of a /find turn and ignores everything else", () => {
    expect(parseFindCommand("/find the plugins stall")).toBe("the plugins stall");
    expect(parseFindCommand("  /FIND  last week  ")).toBe("last week");
    expect(parseFindCommand("/find")).toBe("");
    expect(parseFindCommand("/finder")).toBeNull();
    expect(parseFindCommand("please /find x")).toBeNull();
  });
});

describe("enterAction", () => {
  it("opens a confident index answer without asking a model", () => {
    expect(enterAction(result({ confident: true, deepAvailable: false }), false)).toBe("open");
  });

  it("searches deeper when the index is unsure and deep search can run", () => {
    expect(enterAction(result(), false)).toBe("deep");
    expect(enterAction(result({ hits: [] }), false)).toBe("deep");
  });

  it("opens the first hit once the deep stage has answered or cannot run", () => {
    expect(enterAction(result({ stage: "model" }), false)).toBe("open");
    expect(enterAction(result({ stage: "index_fallback" }), false)).toBe("open");
    expect(enterAction(result({ deepAvailable: false }), false)).toBe("open");
    expect(enterAction(result({ deepAvailable: false, hits: [] }), false)).toBe("none");
  });

  it("does nothing while a deep search is already running", () => {
    expect(enterAction(result(), true)).toBe("none");
    expect(enterAction(undefined, false)).toBe("none");
  });
});
