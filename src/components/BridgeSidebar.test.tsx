import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it } from "vitest";
import type { Session, Workspace } from "../types";
import { BridgeSidebar, type BridgeSidebarProps } from "./BridgeSidebar";
import { CHAT_VIEW_KEY } from "./sidebarChats";

const session = (id: string, overrides: Partial<Session> = {}): Session => ({
  id,
  workspaceId: "workspace-1",
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
  chats: [session("chat-1", { title: "Policy engine budget", workspaceId: null })],
  workspaces: [workspace],
  activeSessionId: undefined,
  marketplaceActive: false,
  settingsActive: false,
  expanded: new Set<string>(),
  busy: false,
  onOpenNewChat: noop,
  onOpenMarketplace: noop,
  onOpenSettings: noop,
  onOpenSession: noop,
  onToggleWorkspace: noop,
  onNewWorkspace: noop,
  onNewWorkspaceSession: noop,
  onConnectFolder: noop,
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

  it("includes chats that belong to a workspace", () => {
    // The old rail rendered standalone chats only, so a project chat was
    // reachable only by expanding its project.
    const html = render({ chats: [session("in-project", { title: "Inside harness", workspaceId: "workspace-1" })] });
    expect(html).toContain("Inside harness");
  });

  it("keeps harness and model out of the row text but in its tooltip", () => {
    const html = render({
      chats: [session("a", { title: "Policy engine budget", harness: "opencode", model: "qwen3.7-plus", workspaceId: null })],
    });
    expect(html).toContain('title="Policy engine budget — OpenCode · qwen3.7-plus"');
    expect(html).not.toMatch(/>OpenCode · qwen3\.7-plus</);
  });

  it("caps a group and offers the rest behind one control", () => {
    const chats = Array.from({ length: 15 }, (_, index) =>
      session(`c${index}`, { title: `Chat ${index}`, startedAt: daysAgo(0, 1 + index), workspaceId: null }));
    const html = render({ chats });
    expect(html).toContain("Show 3 more");
    expect(html).not.toContain("Chat 0");
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

describe("BridgeSidebar projects", () => {
  it("renders the projects section above the chat history", () => {
    const html = render();
    expect(html.indexOf("Projects")).toBeGreaterThan(-1);
    expect(html.indexOf("Projects")).toBeLessThan(html.indexOf("Chats"));
  });

  it("drops the branch and dirty-file line from an expanded project", () => {
    const html = render({
      workspaces: [{ ...workspace, branch: "feat/router", dirtyFiles: 3 } as Workspace],
      expanded: new Set(["workspace-1"]),
      chats: [session("a", { title: "Inside harness" })],
    });
    expect(html).toContain("Inside harness");
    expect(html).not.toContain("feat/router");
    expect(html).not.toContain("3 changed");
  });

  it("still offers new agent and connect folder inside an expanded project", () => {
    const html = render({ expanded: new Set(["workspace-1"]), workspaces: [{ ...workspace, path: null } as Workspace] });
    expect(html).toContain("New agent");
    expect(html).toContain("Connect folder");
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
