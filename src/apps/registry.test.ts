import { describe, expect, it } from "vitest";
import { APP_VIEWS } from "../navigationHistory";
import { appForView, appMatches, appsBadge, appById, badgeText, BUILTIN_APPS, isAppId, NON_APP_VIEWS } from "./registry";

describe("built-in apps", () => {
  // The launcher is the only way into these screens now. A new screen that
  // lands without an app entry would be unreachable, so this fails first.
  it("makes every screen an app or names it as not one", () => {
    const appViews = new Set(BUILTIN_APPS.map(app => app.view));
    for (const view of APP_VIEWS) {
      const covered = appViews.has(view) || NON_APP_VIEWS.includes(view);
      expect(covered, `${view} has no app and is not listed as a non-app view`).toBe(true);
    }
  });

  it("never lists a view as both an app and a non-app", () => {
    for (const app of BUILTIN_APPS) expect(NON_APP_VIEWS).not.toContain(app.view);
  });

  it("gives every app its own id and its own view", () => {
    expect(new Set(BUILTIN_APPS.map(app => app.id)).size).toBe(BUILTIN_APPS.length);
    expect(new Set(BUILTIN_APPS.map(app => app.view)).size).toBe(BUILTIN_APPS.length);
  });

  it("keeps every screen the old sidebar opened", () => {
    const views = BUILTIN_APPS.map(app => app.view);
    for (const view of ["marketplace", "workflows", "projects", "notes", "agent-fleet", "mission-control", "memory", "usage", "pulls", "saved-setups"] as const) {
      expect(views).toContain(view);
    }
  });

  it("finds the app on screen, counting the grid as Mission Control", () => {
    expect(appForView("notes")).toBe("notes");
    expect(appForView("agent-fleet")).toBe("terminals");
    expect(appForView("workspace", "grid")).toBe("mission-control");
    expect(appForView("workspace", "single")).toBeNull();
    expect(appForView("settings")).toBeNull();
  });

  it("checks ids", () => {
    expect(isAppId("notes")).toBe(true);
    expect(isAppId("marketplace")).toBe(false);
    expect(isAppId(3)).toBe(false);
    expect(appById("store").view).toBe("marketplace");
  });

  it("matches names and keywords, case-insensitively", () => {
    expect(appMatches(appById("usage"), "USA")).toBe(true);
    expect(appMatches(appById("usage"), "quota")).toBe(true);
    expect(appMatches(appById("store"), "plugins")).toBe(true);
    expect(appMatches(appById("notes"), "quota")).toBe(false);
    expect(appMatches(appById("notes"), "  ")).toBe(true);
  });
});

describe("badges", () => {
  it("hides a zero and reads a capped count with a plus", () => {
    expect(badgeText(undefined)).toBeUndefined();
    expect(badgeText({ count: 0 })).toBeUndefined();
    expect(badgeText({ count: 2 })).toEqual({ count: "2", label: "2 waiting for you" });
    expect(badgeText({ count: 50, capped: true })?.count).toBe("50+");
  });

  it("sums every app into the Apps row and carries the cap", () => {
    expect(appsBadge({})).toBeUndefined();
    expect(appsBadge({ workflows: { count: 0 }, memory: { count: 0 } })).toBeUndefined();
    expect(appsBadge({ workflows: { count: 2 }, memory: { count: 3 } })?.count).toBe("5");
    expect(appsBadge({ workflows: { count: 2 }, memory: { count: 50, capped: true } })?.count).toBe("52+");
  });
});
