import { describe, expect, it } from "vitest";
import { placesEqual, recordPlace, type AppPlace } from "./navigationHistory";

const workspace = (sessionId: string | null = null, paradigm: AppPlace["paradigm"] = "single"): AppPlace => ({ view: "workspace", sessionId, paradigm });

describe("recordPlace", () => {
  it("ignores a repeat of the current place", () => {
    const start = [workspace("a")];
    expect(recordPlace(start, 0, workspace("a"))).toEqual({ stack: start, index: 0 });
  });

  it("appends a new place and drops anything after the cursor", () => {
    const stack = [workspace("a"), workspace("b"), { view: "projects" as const, sessionId: "b", paradigm: "single" as const }];
    expect(recordPlace(stack, 0, { view: "work", sessionId: "a", paradigm: "single" })).toEqual({
      stack: [workspace("a"), { view: "work", sessionId: "a", paradigm: "single" }],
      index: 1,
    });
  });

  it("records Mission Control separately from a focused workspace session", () => {
    const focused = workspace("a", "single");
    expect(recordPlace([focused], 0, workspace("a", "grid"))).toEqual({
      stack: [focused, workspace("a", "grid")],
      index: 1,
    });
  });
});

describe("placesEqual", () => {
  it("treats a different session as a different place", () => {
    expect(placesEqual(workspace("a"), workspace("b"))).toBe(false);
    expect(placesEqual(workspace("a"), workspace("a"))).toBe(true);
  });

  it("treats a different view as a different place", () => {
    expect(placesEqual(workspace("a"), { view: "work", sessionId: "a", paradigm: "single" })).toBe(false);
  });

  it("treats automations as a distinct view from marketplace", () => {
    expect(placesEqual(
      { view: "automations", sessionId: null, paradigm: "single" },
      { view: "marketplace", sessionId: null, paradigm: "single" },
    )).toBe(false);
  });
});
