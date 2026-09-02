import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it } from "vitest";
import type { Session, Workspace } from "../types";
import { BridgeSidebar, type BridgeSidebarProps } from "./BridgeSidebar";
import { CHAT_VIEW_KEY } from "./sidebarChats";

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

const DATE_VIEW = JSON.stringify({ status: "all", agent: "all", groupBy: "date", sortBy: "recency" });

const props = (overrides: Partial<BridgeSidebarProps> = {}): BridgeSidebarProps => ({
  chats: [session("chat-1", { title: "Policy engine budget" })],
  workspaces: [workspace],
  activeSessionId: undefined,
  projectsActive: false,
  marketplaceActive: false,
  missionControlActive: false,
  settingsActive: false,
  accountName: "cestercian",
  onOpenNewChat: noop,
  onOpenProjects: noop,
  onOpenMarketplace: noop,
  onOpenMissionControl: noop,
  onOpenWorkBoard: noop,
  onOpenMemory: noop,
  onOpenSettings: noop,
  onOpenSession: noop,
  ...overrides,
});

const render = (overrides: Partial<BridgeSidebarProps> = {}) =>
  renderToStaticMarkup(<BridgeSidebar {...props(overrides)} />);

/** The aside's own opening tag — its classes and style, without the markup of
 *  everything it contains. */
const asideTag = (html: string) => {
  const start = html.indexOf("<aside");
  return html.slice(start, html.indexOf(">", start) + 1);
};

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
    expect(html).toContain("left-0");
    expect(html).toContain("border-r border-sidebar-border");
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

  it("keeps the drag-to-resize handle on the right edge, pointer-capable widths only", () => {
    // The handle is meaningless in the drawer, where width is fixed.
    // Its hit area lives just outside the rail, leaving the edge scrollbar usable.
    expect(render()).toContain("absolute inset-y-0 -right-3");
    expect(render()).toContain("hidden w-3 cursor-col-resize touch-none select-none sm:block");
    expect(render()).toContain("after:left-0");
  });

  it("puts panel beside the traffic lights and chevrons on the right of that strip", () => {
    const html = render();
    expect(html).toContain("pl-24");
    expect(html).toContain("u-traffic-inset");
    expect(html).toContain("Hide sidebar");
    expect(html).toContain("ml-auto");
    expect(html).toContain("aria-label=\"Back\"");
    expect(html).toContain("aria-label=\"Forward\"");
  });

  it("hides those window controls when they live on the title bar", () => {
    const html = render({ showWindowNav: false });
    expect(html).not.toContain("Hide sidebar");
    expect(html).not.toContain("aria-label=\"Back\"");
  });
});

// Collapsing used to leave a 68px column of icons and status dots. It now takes
// the rail off the screen; the panel and history controls move to the canvas'
// chrome row, which App owns.
describe("BridgeSidebar hidden", () => {
  it("gives every pixel back instead of leaving an icon rail", () => {
    localStorage.setItem("bridge.sidebar.collapsed", "1");
    const aside = asideTag(render());
    expect(aside).toContain("--sidebar-w:0px");
    // Not even the seam survives, and nothing spills out of a zero-width box.
    expect(aside).not.toContain("border-r");
    expect(aside).toContain("overflow-hidden");
    expect(aside).not.toContain("68px");
  });

  it("puts the whole rail out of reach of the pointer and the screen reader", () => {
    localStorage.setItem("bridge.sidebar.collapsed", "1");
    const aside = asideTag(render());
    expect(aside).toContain("inert=\"\"");
    expect(aside).toContain("aria-hidden=\"true\"");
  });

  it("keeps the width it was left at, so reopening lands where the user had it", () => {
    localStorage.setItem("bridge.sidebar.width", "320");
    localStorage.setItem("bridge.sidebar.collapsed", "1");
    // The panel behind the clip stays full width, which is what makes the wipe
    // a wipe rather than a reflow — and what the rail reopens to.
    expect(asideTag(render())).toContain("--sidebar-panel-w:320px");

    localStorage.setItem("bridge.sidebar.collapsed", "0");
    expect(asideTag(render())).toContain("--sidebar-w:320px");
  });

  it("still fills the drawer below sm, which is a separate axis", () => {
    localStorage.setItem("bridge.sidebar.collapsed", "1");
    const html = render({ mobileOpen: true });
    expect(asideTag(html)).not.toContain("inert");
    expect(html).toContain("Policy engine budget");
    expect(html).toContain("Repositories");
  });
});

describe("BridgeSidebar theming", () => {
  it("spaces folders and chat rows while letting the scrollbar reach the rail edge", () => {
    const html = render();
    expect(html).toContain("mb-0.5 gap-0.5");
    expect(html).not.toContain("border-b border-sidebar-border pr-2");
    expect(html).toContain("-mr-2 min-h-0 flex-1 overflow-y-auto pr-2");
  });

  it("paints the rail from the sidebar ladder tokens, never a literal", () => {
    const html = render();
    expect(html).toContain("bg-sidebar");
    expect(html).not.toMatch(/bg-\[#|bg-white\/|backdrop-blur/);
  });

  it("uses the chat-row accent for the active repository instead of an opaque black header", () => {
    localStorage.setItem(CHAT_VIEW_KEY, JSON.stringify({ status: "all", agent: "all", groupBy: "project", sortBy: "recency" }));
    const html = render({
      activeSessionId: "chat-1",
      chats: [session("chat-1", { workspaceId: "workspace-1" })],
    });
    const repository = html.split("<div").find(chunk => chunk.includes('title="Hide harness"')) ?? "";
    expect(repository).toContain("bg-accent");
    expect(repository).not.toContain("bg-sidebar ");
  });

  it("fills only the repository that holds the open chat", () => {
    localStorage.setItem(CHAT_VIEW_KEY, JSON.stringify({ status: "all", agent: "all", groupBy: "project", sortBy: "recency" }));
    const other: Workspace = { ...workspace, id: "workspace-2", title: "deck-shell" };
    const html = render({
      workspaces: [workspace, other],
      activeSessionId: "chat-1",
      chats: [
        session("chat-1", { title: "Policy engine budget", workspaceId: "workspace-1" }),
        session("chat-2", { title: "Deck polish", workspaceId: "workspace-2" }),
      ],
    });
    const groups = html.split("<div");
    const active = groups.find(chunk => chunk.includes('title="Hide harness"')) ?? "";
    const idle = groups.find(chunk => chunk.includes('title="Hide deck-shell"')) ?? "";
    expect(active).toContain("bg-accent font-medium");
    expect(idle).not.toContain("bg-accent font-medium");
    expect(idle).toContain("hover:bg-accent/70");
    expect(idle.split("hover:bg-accent/70").join("")).not.toContain("bg-accent/70");
  });

  it("lets repository groups scroll out instead of pinning the active repository", () => {
    localStorage.setItem(CHAT_VIEW_KEY, JSON.stringify({ status: "all", agent: "all", groupBy: "project", sortBy: "recency" }));
    const html = render({
      activeSessionId: "chat-1",
      chats: [session("chat-1", { workspaceId: "workspace-1" })],
    });
    const repository = html.split("<div").find(chunk => chunk.includes('title="Hide harness"')) ?? "";
    expect(repository).not.toContain("sticky");
    expect(repository).not.toContain("top-0");
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
    localStorage.setItem(CHAT_VIEW_KEY, DATE_VIEW);
    const html = render({
      chats: [
        session("now", { title: "Today chat", startedAt: daysAgo(0) }),
        session("prev", { title: "Yesterday chat", startedAt: daysAgo(1) }),
      ],
    });
    expect(html).toContain("Today");
    expect(html).toContain("Yesterday");
  });

  it("lists plain chats and project chats together", () => {
    const chats = [
      session("plain", { title: "Japan relocation planning" }),
      session("in-project", { title: "Inside harness", workspaceId: "workspace-1" }),
    ];
    const html = render({ chats });
    expect(html).toContain("Japan relocation planning");
    expect(html).toContain("Inside harness");
    expect(html).toContain("Repositories");
    expect(html).toContain("No project");
    expect(html).toContain("harness");
  });

  it("keeps harness and model out of the row text but in its tooltip", () => {
    const html = render({
      chats: [session("a", { title: "Policy engine budget", harness: "opencode", model: "qwen3.7-plus" })],
    });
    expect(html).toContain('title="Policy engine budget — OpenCode · qwen3.7-plus"');
    expect(html).not.toMatch(/>OpenCode · qwen3\.7-plus</);
  });

  it("caps a group and offers the rest behind one control", () => {
    localStorage.setItem(CHAT_VIEW_KEY, DATE_VIEW);
    const chats = Array.from({ length: 15 }, (_, index) =>
      session(`c${index}`, { title: `Chat ${index}`, startedAt: daysAgo(0, 1 + index) }));
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

  it("labels the list Repositories", () => {
    expect(render()).toContain("Repositories");
    expect(render()).not.toContain(">Chats<");
  });

  it("indents rows under a group and shows a compact time", () => {
    localStorage.setItem(CHAT_VIEW_KEY, DATE_VIEW);
    const html = render({
      chats: [session("now", { title: "Today chat", startedAt: new Date(Date.now() - 2_000).toISOString() })],
    });
    expect(html).toContain("pl-7");
    expect(html).toMatch(/>now</);
  });

  it("shows a git badge only when the chat's workspace has a branch", () => {
    const chats = [session("branched", { title: "On main", workspaceId: "workspace-1" })];
    expect(render({ chats })).toContain("On a git branch");
    expect(render({ chats, workspaces: [{ ...workspace, branch: null }] })).not.toContain("On a git branch");
  });

  it("never renders a cloud/sync badge", () => {
    expect(render()).not.toContain("Synced");
    expect(render()).not.toContain("aria-label=\"Synced\"");
  });

  it("offers a per-project new-chat action on a real project group", () => {
    const html = render({
      chats: [session("in-project", { title: "Inside harness", workspaceId: "workspace-1" })],
      onNewChatInProject: noop,
    });
    expect(html).toContain('aria-label="New chat in harness"');
  });

  it("omits the per-project new-chat action without a handler, on No project, and off project grouping", () => {
    const chats = [
      session("plain", { title: "Japan relocation planning" }),
      session("in-project", { title: "Inside harness", workspaceId: "workspace-1" }),
    ];
    expect(render({ chats })).not.toContain("New chat in");

    localStorage.setItem(CHAT_VIEW_KEY, JSON.stringify({ status: "all", agent: "all", groupBy: "project", sortBy: "recency" }));
    const noProjectHtml = render({ chats, onNewChatInProject: noop });
    expect(noProjectHtml).not.toContain('aria-label="New chat in No project"');

    localStorage.setItem(CHAT_VIEW_KEY, DATE_VIEW);
    expect(render({ chats, onNewChatInProject: noop })).not.toContain("New chat in");
  });
});

describe("BridgeSidebar without the projects tree", () => {
  it("carries no project rows and no new-project control", () => {
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

  it("marks Projects active when that screen is open", () => {
    const projectsButton = (html: string) => html.split("<button").find(chunk => chunk.includes('aria-label="Projects"')) ?? "";
    expect(projectsButton(render())).toBeTruthy();
    expect(projectsButton(render())).not.toContain("aria-current");
    expect(projectsButton(render({ projectsActive: true }))).toContain('aria-current="page"');
  });

  it("marks Memory current when that screen is open", () => {
    const memoryButton = (html: string) => html.split("<button").find(chunk => chunk.includes('aria-label="Memory"')) ?? "";
    expect(memoryButton(render())).not.toContain("aria-current");
    expect(memoryButton(render({ memoryActive: true }))).toContain('aria-current="page"');
  });

  it("uses a real local account row as the settings entry", () => {
    const html = render();
    expect(html).toContain("Projects");
    expect(html).toContain("Memory");
    expect(html).toContain("Marketplace");
    expect(html).toContain("cestercian");
    expect(html).toContain('aria-label="Open settings for cestercian"');
    expect(html).not.toContain("Yashaswi");
  });

  it("still labels project groups, which is why it keeps the workspaces prop", () => {
    localStorage.setItem(CHAT_VIEW_KEY, JSON.stringify({ status: "all", agent: "all", groupBy: "project", sortBy: "recency" }));
    const html = render({ chats: [session("a", { workspaceId: "workspace-1" })] });
    expect(html).toContain("harness");
  });
});

describe("BridgeSidebar list", () => {
  it("has no Work / Code switch", () => {
    const html = render();
    expect(html).not.toContain('aria-label="Work"');
    expect(html).not.toContain('aria-label="Code"');
    expect(html).not.toContain("Needs you");
    // Hidden until v2 (#458) — see "BridgeSidebar action rows" below.
    expect(html).not.toContain('aria-label="Work board"');
  });

  it("says how New Chat picks a repo when the list is empty", () => {
    expect(render({ chats: [] })).toContain("No chats yet. New Chat opens in the repo you were last in.");
    expect(render({ chats: [] })).not.toContain("New chat asks which project");
  });
});

describe("BridgeSidebar search", () => {
  it("keeps the field closed until the search control is used", () => {
    const html = render();
    expect(html).toContain('aria-label="Search"');
    expect(html).not.toContain("Search chats");
    expect(html).not.toContain("Filter chats and projects");
  });
});

describe("BridgeSidebar action rows", () => {
  it("offers New Chat, Marketplace, Projects, and Memory near the top", () => {
    const html = render();
    expect(html).toContain("New Chat");
    expect(html).toContain("Marketplace");
    expect(html.indexOf("New Chat")).toBeLessThan(html.indexOf("Marketplace"));
    expect(html.indexOf("Marketplace")).toBeLessThan(html.indexOf("Projects"));
    expect(html.indexOf("Projects")).toBeLessThan(html.indexOf("Memory"));
    expect(html).not.toContain("Customize");
    expect(html).not.toContain("Needs you");
  });

  it("hides Mission Control and Work board from the nav for now (#457, #458)", () => {
    const html = render();
    expect(html).not.toContain("Mission Control");
    expect(html).not.toContain("Work board");
  });

  it("drops the filled primary new-chat button", () => {
    expect(render()).not.toContain("bg-primary text-primary-foreground");
  });

  it("labels every row rather than reducing any of them to a bare icon", () => {
    const html = render();
    for (const label of ["New Chat", "Search", "Marketplace", "Projects", "Memory"]) {
      expect(html).toContain(`aria-label="${label}"`);
      expect(html).toContain(`>${label}</button>`);
    }
    // The icon-rail squares are gone with the rail itself.
    expect(html).not.toContain("mx-auto size-9");
    expect(html).not.toContain("mx-auto size-10");
  });

  it("marks Marketplace and account settings current", () => {
    const marketplace = (html: string) => html.split("<button").find(chunk => chunk.includes('aria-label="Marketplace"')) ?? "";
    const account = (html: string) => html.split("<button").find(chunk => chunk.includes('aria-label="Open settings for cestercian"')) ?? "";
    expect(marketplace(render({ marketplaceActive: true }))).toContain('aria-current="page"');
    expect(account(render({ settingsActive: true }))).toContain('aria-current="page"');
    expect(marketplace(render())).not.toContain("aria-current");
  });
});

describe("BridgeSidebar without the worker panel", () => {
  it("shows no live-worker strip in the expanded rail", () => {
    const html = render({ chats: [session("a", { status: "working" })] });
    expect(html).not.toContain("Live workers");
    expect(html).not.toContain("NEEDS DELEGATION");
  });

  it("offers account Memory with no workspace at all", () => {
    // Account memory is not workspace memory; a plain chat reaches it too.
    expect(render({ workspaces: [] })).toContain("Memory");
  });
});
