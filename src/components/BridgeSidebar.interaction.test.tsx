// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Session, Workspace } from "../types";
import { BridgeSidebar, type BridgeSidebarProps } from "./BridgeSidebar";
import { CHAT_SCOPE_KEY } from "./sidebarChats";

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
    click(scopeTab("Home"));
    expect(text()).toContain("Japan relocation");
  });
});

describe("BridgeSidebar scope switching", () => {
  it("shows plain chats under Home and project chats under Code", () => {
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
    expect(scopeTab("Home").getAttribute("aria-selected")).toBe("true");
    // A project chat opened from the projects screen must not vanish into a list
    // the rail is not showing.
    mount({ activeSessionId: "project" });
    expect(scopeTab("Code").getAttribute("aria-selected")).toBe("true");
    expect(text()).toContain("Sidebar redesign");
  });

  it("follows a plain chat back to Home", () => {
    localStorage.setItem(CHAT_SCOPE_KEY, "code");
    mount({ activeSessionId: "project" });
    expect(scopeTab("Code").getAttribute("aria-selected")).toBe("true");
    mount({ activeSessionId: "plain" });
    expect(scopeTab("Home").getAttribute("aria-selected")).toBe("true");
  });

  it("leaves a manual switch alone once it has followed a chat", () => {
    mount({ activeSessionId: "plain" });
    click(scopeTab("Code"));
    // A poll re-renders with the same active id; the manual choice must hold.
    mount({ activeSessionId: "plain" });
    expect(scopeTab("Code").getAttribute("aria-selected")).toBe("true");
  });
});
