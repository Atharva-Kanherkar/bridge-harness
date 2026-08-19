import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it } from "vitest";
import type { Session, Workspace } from "../types";
import { BridgeSidebar, type BridgeSidebarProps } from "./BridgeSidebar";
import { CHAT_SCOPE_KEY, CHAT_VIEW_KEY } from "./sidebarChats";

const session = (id: string, overrides: Partial<Session> = {}): Session => ({
  id,
  workspaceId: null,
  harness: "codex",
  label: id,
  status: "working",
  startedAt: "2026-08-18T10:00:00Z",
  endedAt: null,
  contextPercent: null,
  usagePercent: null,
  metricSource: "reported",
  restorationMode: "fresh",
  continuationFidelity: "native",
  ...overrides,
} as Session);

const workspace: Workspace = {
  id: "workspace-1",
  title: "harness",
  branch: "main",
  status: "ready",
  dirtyFiles: 0,
} as Workspace;

const noop = () => {};

const props = (overrides: Partial<BridgeSidebarProps> = {}): BridgeSidebarProps => ({
  chats: [session("chat-1", { title: "Policy engine budget" })],
  workspaces: [workspace],
  activeSessionId: undefined,
  projectsActive: false,
  marketplaceActive: false,
  settingsActive: false,
  onOpenNewChat: noop,
  onOpenProjects: noop,
  onOpenMarketplace: noop,
  onOpenSettings: noop,
  onOpenSession: noop,
  ...overrides,
});

const render = (overrides: Partial<BridgeSidebarProps> = {}) =>
  renderToStaticMarkup(<BridgeSidebar {...props(overrides)} />);

/** Chats stamped relative to now, so day headers are stable whenever this runs. */
const daysAgo = (days: number, hour = 12) => {
  const date = new Date();
  date.setDate(date.getDate() - days);
  date.setHours(hour, 0, 0, 0);
  return date.toISOString();
};

// These tests run in the default node environment; the rail reads persisted
// width/collapse/view state during render, so it needs a minimal storage stub.
beforeEach(() => {
  const store = new Map<string, string>();
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => { store.set(key, value); },
      removeItem: (key: string) => { store.delete(key); },
      clear: () => store.clear(),
    },
  });
});

describe("BridgeSidebar responsive rail", () => {
  it("stays off-canvas on narrow windows until it is opened", () => {
    const html = render({ mobileOpen: false });
    expect(html).toContain("-translate-x-full");
    // It must still be laid out normally from the sm breakpoint up.
    expect(html).toContain("sm:translate-x-0");
    expect(html).toContain("sm:relative");
  });

  it("slides in and offers a dismiss target when opened", () => {
    const html = render({ mobileOpen: true });
    expect(html).toContain("translate-x-0");
    expect(html).not.toContain("-translate-x-full");
    expect(html).toContain("Close navigation");
  });

  it("does not render the dismiss scrim while closed", () => {
    expect(render({ mobileOpen: false })).not.toContain("Close navigation");
  });

  it("keeps the drag-to-resize handle to pointer-capable widths only", () => {
    // The handle is meaningless in the drawer, where width is fixed.
    expect(render()).toContain("hidden w-3 cursor-col-resize touch-none select-none sm:block");
  });
});

describe("BridgeSidebar theming", () => {
  it("paints the rail from the sidebar ladder tokens, never a literal", () => {
    const html = render();
    expect(html).toContain("bg-sidebar");
    expect(html).not.toMatch(/bg-\[#|bg-white\/|backdrop-blur/);
  });

  it("carries session status on semantic tokens", () => {
    const html = render({
      chats: [
        session("a", { status: "working" }),
        session("b", { status: "waiting" }),
        session("c", { status: "failed" }),
      ],
    });
    expect(html).toContain("bg-success");
    expect(html).toContain("bg-warning");
    expect(html).toContain("bg-destructive");
    expect(html).not.toMatch(/emerald-|amber-|sky-|red-4/);
  });

  it("leaves an idle chat without a full-strength dot", () => {
    expect(render({ chats: [session("idle", { status: "completed" })] })).toContain("bg-muted-foreground/25");
  });
});

describe("BridgeSidebar history", () => {
  it("groups chats under day headers", () => {
    const html = render({
      chats: [
        session("now", { title: "Today chat", startedAt: daysAgo(0) }),
        session("prev", { title: "Yesterday chat", startedAt: daysAgo(1) }),
      ],
    });
    expect(html).toContain("Today");
    expect(html).toContain("Yesterday");
  });

  it("keeps a project chat out of Home and shows it under Code", () => {
    const chats = [session("in-project", { title: "Inside harness", workspaceId: "workspace-1" })];
    expect(render({ chats })).not.toContain("Inside harness");
    localStorage.setItem(CHAT_SCOPE_KEY, "code");
    expect(render({ chats })).toContain("Inside harness");
  });

  it("keeps harness and model out of the row text but in its tooltip", () => {
    const html = render({
      chats: [session("a", { title: "Policy engine budget", harness: "opencode", model: "qwen3.7-plus" })],
    });
    expect(html).toContain('title="Policy engine budget — OpenCode · qwen3.7-plus"');
    expect(html).not.toMatch(/>OpenCode · qwen3\.7-plus</);
  });

  it("caps a group and offers the rest behind one control", () => {
    const chats = Array.from({ length: 15 }, (_, index) =>
      session(`c${index}`, { title: `Chat ${index}`, startedAt: daysAgo(0, 1 + index) }));
    const html = render({ chats });
    expect(html).toContain("Show 3 more");
    expect(html).not.toContain("Chat 0");
  });

  it("does not cap the collapsed rail, which has nowhere to put the reveal control", () => {
    localStorage.setItem("bridge.sidebar.collapsed", "1");
    localStorage.setItem(CHAT_VIEW_KEY, JSON.stringify({ status: "all", agent: "all", groupBy: "none", sortBy: "recency" }));
    const chats = Array.from({ length: 15 }, (_, index) =>
      session(`c${index}`, { title: `Chat ${index}`, startedAt: daysAgo(0, 1 + index) }));
    const html = render({ chats });
    expect(html).not.toContain("Show 3 more");
    // Every chat keeps a row; a cap with no control would strand the last three.
    expect(html.match(/rounded-md px-0/g) ?? []).toHaveLength(15);
  });

  it("honours a persisted grouping choice", () => {
    localStorage.setItem(CHAT_VIEW_KEY, JSON.stringify({ status: "all", agent: "all", groupBy: "status", sortBy: "recency" }));
    const html = render({ chats: [session("w", { status: "waiting" }), session("f", { status: "failed" })] });
    expect(html).toContain("Waiting on you");
    expect(html).toContain("Failed");
    expect(html).not.toContain("Yesterday");
  });

  it("says so when a filter empties the list", () => {
    localStorage.setItem(CHAT_VIEW_KEY, JSON.stringify({ status: "failed", agent: "all", groupBy: "date", sortBy: "recency" }));
    expect(render({ chats: [session("a", { status: "working" })] })).toContain("No chat matches this filter");
  });
});

describe("BridgeSidebar without the projects tree", () => {
  it("carries no project rows and no new-project control", () => {
    localStorage.setItem(CHAT_SCOPE_KEY, "code");
    const html = render({
      workspaces: [workspace],
      chats: [session("a", { title: "Inside harness", workspaceId: "workspace-1" })],
    });
    // Under Code the chat is listed flat; what is gone is the tree around it.
    expect(html).toContain("Inside harness");
    expect(html).not.toContain("New project");
    expect(html).not.toContain("New agent");
    expect(html).not.toContain("Connect folder");
  });

  it("offers Projects in the footer and marks it active when that screen is open", () => {
    // Read the Projects button out of the markup rather than matching across it.
    const projectsButton = (html: string) => html.split("<button").find(chunk => chunk.includes("Projects")) ?? "";
    expect(projectsButton(render())).toBeTruthy();
    expect(projectsButton(render())).not.toContain("bg-accent text-foreground");
    // Same active treatment Marketplace and Settings get.
    expect(projectsButton(render({ projectsActive: true }))).toContain("bg-accent text-foreground");
  });

  it("still labels project groups, which is why it keeps the workspaces prop", () => {
    localStorage.setItem(CHAT_SCOPE_KEY, "code");
    localStorage.setItem(CHAT_VIEW_KEY, JSON.stringify({ status: "all", agent: "all", groupBy: "project", sortBy: "recency" }));
    const html = render({ chats: [session("a", { workspaceId: "workspace-1" })] });
    expect(html).toContain("harness");
  });
});

describe("BridgeSidebar scope switch", () => {
  it("offers Home and Code, with Home selected by default", () => {
    const html = render();
    expect(html).toContain('aria-label="Home"');
    expect(html).toContain('aria-label="Code"');
    const home = html.split("<button").find(chunk => chunk.includes('aria-label="Home"')) ?? "";
    expect(home).toContain('aria-selected="true"');
  });

  it("honours a persisted scope", () => {
    localStorage.setItem(CHAT_SCOPE_KEY, "code");
    const code = render().split("<button").find(chunk => chunk.includes('aria-label="Code"')) ?? "";
    expect(code).toContain('aria-selected="true"');
  });

  it("splits plain chats from project chats", () => {
    const chats = [
      session("plain", { title: "Japan relocation planning" }),
      session("project", { title: "Sidebar redesign", workspaceId: "workspace-1" }),
    ];
    const home = render({ chats });
    expect(home).toContain("Japan relocation planning");
    expect(home).not.toContain("Sidebar redesign");

    localStorage.setItem(CHAT_SCOPE_KEY, "code");
    const code = render({ chats });
    expect(code).toContain("Sidebar redesign");
    expect(code).not.toContain("Japan relocation planning");
  });

  it("says where project chats come from when Code is empty", () => {
    localStorage.setItem(CHAT_SCOPE_KEY, "code");
    expect(render({ chats: [session("plain")] })).toContain("New chat asks which project");
  });

  it("keeps the switch reachable in the collapsed rail", () => {
    localStorage.setItem("bridge.sidebar.collapsed", "1");
    const html = render();
    expect(html).toContain('aria-label="Home"');
    expect(html).toContain('aria-label="Code"');
  });
});

describe("BridgeSidebar search", () => {
  it("keeps the field closed until the search control is used", () => {
    const html = render();
    expect(html).toContain("Search chats");
    expect(html).not.toContain("Filter chats and projects");
  });
});

describe("BridgeSidebar without the worker panel", () => {
  it("shows no live-worker strip in the expanded rail", () => {
    const html = render({ chats: [session("a", { status: "working" })] });
    expect(html).not.toContain("Live workers");
    expect(html).not.toContain("NEEDS DELEGATION");
  });

  it("shows no worker tile in the collapsed rail", () => {
    localStorage.setItem("bridge.sidebar.collapsed", "1");
    const html = render({ chats: [session("a", { status: "working" })] });
    expect(html).not.toContain("Live workers");
    expect(html).not.toMatch(/workers?: /);
  });
});
