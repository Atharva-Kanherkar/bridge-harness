import { describe, expect, it } from "vitest";
import { placesEqual, recordPlace, type AppPlace, type AppView } from "./navigationHistory";

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

  it("keeps Marketplace as the single catalog and automations destination", () => {
    // Catalog and Automations are sections of one screen, so history holds one
    // entry for both. Re-adding an "automations" view breaks this exhaustive
    // record at compile time, which is the point of writing it out.
    const views: Record<AppView, true> = { workspace: true, work: true, projects: true, memory: true, marketplace: true, settings: true };
    expect(Object.keys(views).sort()).toEqual(["marketplace", "memory", "projects", "settings", "work", "workspace"]);

    const marketplace: AppPlace = { view: "marketplace", sessionId: null, paradigm: "single" };
    expect(recordPlace([marketplace], 0, marketplace)).toEqual({ stack: [marketplace], index: 0 });
  });
});
