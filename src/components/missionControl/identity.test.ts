import { describe, expect, it } from "vitest";
import type { Project, Session, SessionEntry, Workspace } from "../../types";
import { compactLinks, displayTitle, latestAsk, projectLabel } from "./identity";

describe("displayTitle", () => {
  it("names GitHub pull requests and issues by number", () => {
    expect(displayTitle("https://github.com/Atharva-Kanherkar/kairo/pull/43 reviewe…", "kairo")).toBe("PR #43");
    expect(displayTitle("https://github.com/Atharva-Kanherkar/kairo/pull/43", "Bridge")).toBe("kairo PR #43");
    expect(displayTitle("https://github.com/org/repo/issues/12#issuecomm…", null)).toBe("repo Issue #12");
    expect(displayTitle("https://github.com/org/Repo/pull/7/files", "repo")).toBe("PR #7");
  });

  it("names a repository link by the repository and keeps whole words after it", () => {
    expect(displayTitle("https://github.com/Atharva-Kanherkar/kairo Read repo")).toBe("kairo Read repo");
    expect(displayTitle("https://github.com/Atharva-Kanherkar/kairo.git")).toBe("kairo");
  });

  it("names any other link by its host and last path segment", () => {
    expect(displayTitle("https://vercel.com/team/deployments failing")).toBe("vercel.com deployments failing");
    expect(displayTitle("http://localhost:1420/")).toBe("localhost:1420");
    expect(displayTitle("https://example.com/runs/8f3a9c2e7d1b4a6f9e0c3b5d7a1f2e4c")).toBe("example.com");
  });

  it("drops the repository from a derived link heading inside its own project", () => {
    expect(displayTitle("kairo PR #43", "kairo")).toBe("PR #43");
    expect(displayTitle("bridge-harness issue #1", "Bridge")).toBe("bridge-harness Issue #1");
  });

  it("leaves ordinary and user-chosen titles alone", () => {
    expect(displayTitle("Session supervisor")).toBe("Session supervisor");
    expect(displayTitle("Fix https://example.com later")).toBe("Fix https://example.com later");
    expect(displayTitle("  Pleqase ignore repo ")).toBe("Pleqase ignore repo");
  });
});

describe("projectLabel", () => {
  const session = (extra: Partial<Session> = {}) => ({ id: "s", label: "Chat", ...extra }) as Session;
  const projects = [{ id: "p", name: "bridge-harness", path: "/src/bridge", createdAt: "" }] as Project[];

  it("prefers the owning project, then the workspace, then the working folder", () => {
    expect(projectLabel(session(), { id: "w", title: "Task tree", projectId: "p" } as Workspace, projects)).toBe("bridge-harness");
    expect(projectLabel(session(), { id: "w", title: "Task tree", projectId: "gone" } as Workspace, projects)).toBe("Task tree");
    expect(projectLabel(session({ cwd: "/Users/me/code/portfolio-site/" }))).toBe("portfolio-site");
    expect(projectLabel(session())).toBeNull();
  });
});

describe("latestAsk", () => {
  const entry = (kind: string, payload: unknown) => ({ kind, payload }) as SessionEntry;

  it("returns the newest non-empty user message, collapsed to one line", () => {
    expect(latestAsk([entry("user.message", { text: "first" }), entry("assistant.message", { text: "reply" }), entry("user.message", { message: "second\n  ask" }), entry("user.message", { text: "   " })])).toBe("second ask");
  });

  it("returns null without user messages", () => {
    expect(latestAsk(undefined)).toBeNull();
    expect(latestAsk([entry("assistant.message", { text: "hi" }), entry("user.message", null)])).toBeNull();
  });
});

describe("compactLinks", () => {
  it("drops schemes and GitHub owners so the ask reads as its target", () => {
    expect(compactLinks("https://github.com/Atharva-Kanherkar/kairo/pull/43 review this")).toBe("kairo/pull/43 review this");
    expect(compactLinks("see https://www.vercel.com/team/ and http://localhost:1420/app")).toBe("see vercel.com/team and localhost:1420/app");
    expect(compactLinks("no links here")).toBe("no links here");
  });
});
