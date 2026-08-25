import { describe, expect, it } from "vitest";
import { placesEqual, recordPlace, type AppPlace } from "./navigationHistory";

const workspace = (sessionId: string | null = null): AppPlace => ({ view: "workspace", sessionId });

describe("recordPlace", () => {
  it("ignores a repeat of the current place", () => {
    const start = [workspace("a")];
    expect(recordPlace(start, 0, workspace("a"))).toEqual({ stack: start, index: 0 });
  });

  it("appends a new place and drops anything after the cursor", () => {
    const stack = [workspace("a"), workspace("b"), { view: "projects" as const, sessionId: "b" }];
    expect(recordPlace(stack, 0, { view: "work", sessionId: "a" })).toEqual({
      stack: [workspace("a"), { view: "work", sessionId: "a" }],
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
    expect(placesEqual(workspace("a"), { view: "work", sessionId: "a" })).toBe(false);
  });

  it("treats automations as a distinct view from marketplace", () => {
    expect(placesEqual(
      { view: "automations", sessionId: null },
      { view: "marketplace", sessionId: null },
    )).toBe(false);
  });
});
