import { describe, expect, it } from "vitest";
import type { AgentEvent, Project, Session, SessionEntry, Workspace } from "../../types";
import { asWireKind } from "../../transcript/wire";
import { compactLinks, displayTitle, latestAsk, projectLabel } from "./identity";

describe("displayTitle", () => {
  it("drops the repository from a derived link heading inside its own project", () => {
    expect(displayTitle("kairo PR #43", "kairo")).toBe("PR #43");
    expect(displayTitle("Kairo pr #43", "kairo")).toBe("PR #43");
    expect(displayTitle("bridge-harness issue #1", "bridge-harness")).toBe("Issue #1");
  });

  it("keeps the repository when it names another project", () => {
    expect(displayTitle("kairo PR #43", "Bridge")).toBe("kairo PR #43");
    expect(displayTitle("bridge-harness issue #1", null)).toBe("bridge-harness Issue #1");
  });

  it("leaves every other title alone, links included", () => {
    expect(displayTitle("  Session supervisor ")).toBe("Session supervisor");
    expect(displayTitle("Pleqase ignore repo", "portfolio-site")).toBe("Pleqase ignore repo");
    expect(displayTitle("https://github.com/o/kairo/pull/43 reviewe…", "kairo")).toBe("https://github.com/o/kairo/pull/43 reviewe…");
    expect(displayTitle("kairo PR #43 follow-up", "kairo")).toBe("kairo PR #43 follow-up");
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
  const event = (id: number, kind: string, role: string | null, text: string | null): AgentEvent => ({ id, sessionId: "s", sequence: id, protocolVersion: 1, kind: asWireKind(kind), itemId: `m${id}`, role, status: "completed", title: null, text, data: {}, providerMeta: {}, createdAt: "now" });

  it("returns the newest non-empty user message, collapsed to one line", () => {
    expect(latestAsk([entry("user.message", { text: "first" }), entry("assistant.message", { text: "reply" }), entry("user.message", { message: "second\n  ask" }), entry("user.message", { text: "   " })])).toBe("second ask");
  });

  it("prefers a live user message over the forest, which reloads later", () => {
    const entries = [entry("user.message", { text: "older ask" })];
    expect(latestAsk(entries, [event(1, "message.completed", "user", "just sent https://github.com/o/kairo/pull/9"), event(2, "message.completed", "assistant", "on it")])).toBe("just sent kairo/pull/9");
    expect(latestAsk(entries, [event(1, "message.completed", "assistant", "reply")])).toBe("older ask");
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
