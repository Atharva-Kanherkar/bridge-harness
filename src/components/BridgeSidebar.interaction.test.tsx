// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Session, Workspace } from "../types";
import { BridgeSidebar, type BridgeSidebarProps } from "./BridgeSidebar";
import { CHAT_SCOPE_KEY, CHAT_VIEW_KEY, readChatView } from "./sidebarChats";

// The static suite covers what the rail renders. This one covers what it does:
// folding a group, switching scope, and following the chat that just opened.

const session = (id: string, overrides: Partial<Session> = {}): Session => ({
  id,
  workspaceId: null,
  harness: "codex",
  label: id,
  title: id,
  model: null,
  status: "idle",
  startedAt: new Date().toISOString(),
  endedAt: null,
  contextPercent: null,
  usagePercent: null,
  metricSource: "reported",
  restorationMode: "fresh",
  continuationFidelity: "native",
  ...overrides,
} as Session);

const workspace = { id: "ws-1", title: "harness", branch: "main", status: "ready", dirtyFiles: 0 } as Workspace;

const noop = () => {};

const props = (overrides: Partial<BridgeSidebarProps> = {}): BridgeSidebarProps => ({
  chats: [session("plain", { title: "Japan relocation" }), session("project", { title: "Sidebar redesign", workspaceId: "ws-1" })],
  workspaces: [workspace],
  activeSessionId: undefined,
  workBoardActive: false,
  workNeedsYouCount: 0,
  projectsActive: false,
  marketplaceActive: false,
  settingsActive: false,
  onOpenNewChat: noop,
  onOpenWorkBoard: () => {},
  onOpenProjects: noop,
  onOpenMarketplace: noop,
  onOpenSettings: noop,
  onOpenSession: noop,
  ...overrides,
});

let container: HTMLDivElement;
let root: Root;

function mount(overrides: Partial<BridgeSidebarProps> = {}) {
  act(() => {
    root.render(<BridgeSidebar {...props(overrides)} />);
  });
}

const text = () => container.textContent ?? "";
const scopeTab = (label: string) => container.querySelector<HTMLButtonElement>(`[role="tab"][aria-label="${label}"]`)!;
const groupHeader = (label: string) =>
  [...container.querySelectorAll<HTMLButtonElement>("button")].find(button => button.getAttribute("aria-expanded") !== null && button.textContent?.includes(label))!;
const click = (element: Element) => {
  act(() => {
    element.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
};

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  // This jsdom instance has no storage of its own, and the rail reads persisted
  // width/collapse/scope during render.
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
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

describe("BridgeSidebar group folding", () => {
  it("hides a group's chats but keeps its header and count", () => {
    mount();
    expect(text()).toContain("Japan relocation");
    const header = groupHeader("Today");
    expect(header.getAttribute("aria-expanded")).toBe("true");

    click(header);
    expect(groupHeader("Today").getAttribute("aria-expanded")).toBe("false");
    expect(text()).not.toContain("Japan relocation");
    // The header survives, so there is something to unfold from.
    expect(text()).toContain("Today");

    click(groupHeader("Today"));
    expect(text()).toContain("Japan relocation");
  });

  it("forgets folds when the grouping changes under them", () => {
    mount();
    click(groupHeader("Today"));
    expect(text()).not.toContain("Japan relocation");
    // Switching scope re-keys every group, so a stale fold must not survive.
    click(scopeTab("Code"));
    click(scopeTab("Work"));
    expect(text()).toContain("Japan relocation");
  });
});

describe("BridgeSidebar scope switching", () => {
  it("shows plain chats under Work and project chats under Code", () => {
    mount();
    expect(text()).toContain("Japan relocation");
    expect(text()).not.toContain("Sidebar redesign");

    click(scopeTab("Code"));
    expect(text()).toContain("Sidebar redesign");
    expect(text()).not.toContain("Japan relocation");
    expect(localStorage.getItem(CHAT_SCOPE_KEY)).toBe("code");
  });

  it("follows the chat that just opened into its own scope", () => {
    mount();
    expect(scopeTab("Work").getAttribute("aria-selected")).toBe("true");
    // A project chat opened from the projects screen must not vanish into a list
    // the rail is not showing.
    mount({ activeSessionId: "project" });
    expect(scopeTab("Code").getAttribute("aria-selected")).toBe("true");
    expect(text()).toContain("Sidebar redesign");
  });

  it("follows a plain chat back to Work", () => {
    localStorage.setItem(CHAT_SCOPE_KEY, "code");
    mount({ activeSessionId: "project" });
    expect(scopeTab("Code").getAttribute("aria-selected")).toBe("true");
    mount({ activeSessionId: "plain" });
    expect(scopeTab("Work").getAttribute("aria-selected")).toBe("true");
  });

  it("corrects a project grouping carried into Work, where nothing has a project", () => {
    localStorage.setItem(CHAT_SCOPE_KEY, "code");
    localStorage.setItem(CHAT_VIEW_KEY, JSON.stringify({ status: "all", agent: "all", groupBy: "project", sortBy: "recency" }));
    mount();
    // Code groups by project name.
    expect(text()).toContain("harness");

    click(scopeTab("Work"));
    // Work falls back to day headers instead of one "No project" bucket, and the
    // correction is persisted so the two never disagree.
    expect(text()).toContain("Today");
    expect(text()).not.toContain("No project");
    expect(readChatView().groupBy).toBe("date");
  });

  it("leaves a manual switch alone once it has followed a chat", () => {
    mount({ activeSessionId: "plain" });
    click(scopeTab("Code"));
    // A poll re-renders with the same active id; the manual choice must hold.
    mount({ activeSessionId: "plain" });
    expect(scopeTab("Code").getAttribute("aria-selected")).toBe("true");
  });
});

// The Work board is the surface behind the pill, so the pill is what opens it and
// the rail's own row is how you get back to it from a chat.
describe("BridgeSidebar and the Work board", () => {
  it("opens the board when the pill flips to Work", () => {
    const onOpenWorkBoard = vi.fn();
    localStorage.setItem(CHAT_SCOPE_KEY, "code");
    mount({ onOpenWorkBoard });
    expect(onOpenWorkBoard).not.toHaveBeenCalled();
    act(() => scopeTab("Work").click());
    expect(onOpenWorkBoard).toHaveBeenCalledOnce();
  });

  it("does not open the board when the pill flips to Code", () => {
    // Code's surface is a conversation. The rail already follows whichever chat is
    // active, so flipping to Code must not reach for the board.
    const onOpenWorkBoard = vi.fn();
    localStorage.setItem(CHAT_SCOPE_KEY, "work");
    mount({ onOpenWorkBoard });
    act(() => scopeTab("Code").click());
    expect(onOpenWorkBoard).not.toHaveBeenCalled();
  });

  it("marks the Needs you row as the current page while the board is open", () => {
    localStorage.setItem(CHAT_SCOPE_KEY, "work");
    mount({ workBoardActive: true });
    const row = [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => button.textContent?.includes("Needs you"));
    expect(row?.getAttribute("aria-current")).toBe("page");
  });

  it("leaves the row uncurrent once a chat is open", () => {
    localStorage.setItem(CHAT_SCOPE_KEY, "work");
    mount({ workBoardActive: false });
    const row = [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => button.textContent?.includes("Needs you"));
    expect(row?.getAttribute("aria-current")).toBeNull();
  });

  it("shows a count only when something needs you", () => {
    localStorage.setItem(CHAT_SCOPE_KEY, "work");
    mount({ workNeedsYouCount: 5 });
    expect(text()).toContain("Needs you5");
    // A zero would be a number that is always there, which is a number nobody reads.
    mount({ workNeedsYouCount: 0 });
    expect(text()).toContain("Needs you");
    expect(text()).not.toContain("Needs you0");
  });

  it("hides the row entirely in Code, where the board is not the surface", () => {
    localStorage.setItem(CHAT_SCOPE_KEY, "code");
    mount();
    expect(text()).not.toContain("Needs you");
  });

  it("returns to the board when the row is clicked", () => {
    const onOpenWorkBoard = vi.fn();
    localStorage.setItem(CHAT_SCOPE_KEY, "work");
    mount({ onOpenWorkBoard, workBoardActive: false });
    const row = [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => button.textContent?.includes("Needs you"))!;
    act(() => row.click());
    expect(onOpenWorkBoard).toHaveBeenCalledOnce();
  });
});
