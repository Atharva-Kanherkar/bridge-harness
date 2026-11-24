// @vitest-environment jsdom
import { appById, builtinEntry } from "../apps/registry";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MotionGlobalConfig } from "framer-motion";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Session, Workspace } from "../types";
import { BridgeSidebar, type BridgeSidebarProps } from "./BridgeSidebar";
import { CHAT_VIEW_KEY, importedSubagents } from "./sidebarChats";
import { SIDEBAR_CHAT_DRAG } from "./missionControl/drag";

// The static suite covers what the rail renders. This one covers what it does:
// folding a group and following the chat that just opened.

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

const DATE_VIEW = JSON.stringify({ status: "all", agent: "all", groupBy: "date", sortBy: "recency" });

const props = (overrides: Partial<BridgeSidebarProps> = {}): BridgeSidebarProps => ({
  chats: [session("plain", { title: "Japan relocation" }), session("project", { title: "Sidebar redesign", workspaceId: "ws-1" })],
  workspaces: [workspace],
  activeSessionId: undefined,
  settingsActive: false,
  accountName: "cestercian",
  onOpenNewChat: noop,
  onOpenProjects: noop,
  onOpenApps: noop,
  onOpenApp: noop,
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
  // width/collapse/view during render.
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

describe("sidebar keyboard resizing", () => {
  it("resizes with arrow keys, clamps to the limits, and persists the width", () => {
    mount();
    const handle = container.querySelector<HTMLElement>('[role="separator"]')!;
    expect(handle.tabIndex).toBe(0);
    const press = (key: string) => act(() => {
      handle.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true }));
    });
    press("ArrowRight");
    expect(handle.getAttribute("aria-valuenow")).toBe("264");
    expect(localStorage.getItem("bridge.sidebar.width")).toBe("264");
    press("Home");
    press("ArrowLeft");
    expect(handle.getAttribute("aria-valuenow")).toBe("200");
    press("End");
    press("ArrowRight");
    expect(handle.getAttribute("aria-valuenow")).toBe("400");
    press("Escape");
    expect(localStorage.getItem("bridge.sidebar.width")).toBe("400");
  });
});

describe("BridgeSidebar group folding", () => {
  it("hides a group's chats but keeps its header and count", () => {
    localStorage.setItem(CHAT_VIEW_KEY, DATE_VIEW);
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
    localStorage.setItem(CHAT_VIEW_KEY, DATE_VIEW);
    mount();
    click(groupHeader("Today"));
    expect(text()).not.toContain("Japan relocation");
    click(container.querySelector('[aria-label="Filter and group chats"]')!);
    click([...document.querySelectorAll("[role='menuitem']")].find(item => item.textContent?.includes("Group by"))!);
    click([...document.querySelectorAll("[role='menuitemradio']")].find(button => button.textContent?.includes("Project"))!);
    expect(text()).toContain("Japan relocation");
  });
});

describe("BridgeSidebar group cap", () => {
  const PROJECT_VIEW = JSON.stringify({ status: "all", agent: "all", groupBy: "project", sortBy: "recency" });
  const second = { id: "ws-2", title: "landing", branch: "main", status: "ready", dirtyFiles: 0 } as Workspace;
  // Oldest first, so "<prefix> 0" is the row the cap hides.
  const chatsIn = (workspaceId: string, prefix: string, count: number) => Array.from({ length: count }, (_, index) =>
    session(`${workspaceId}-${index}`, { title: `${prefix} ${index}`, workspaceId, startedAt: new Date(Date.UTC(2026, 9, 1, index)).toISOString() }));
  const capButton = (label: RegExp) =>
    [...container.querySelectorAll<HTMLButtonElement>("button")].filter(button => label.test(button.textContent ?? ""));

  it("collapses an expanded group back to the cap", () => {
    localStorage.setItem(CHAT_VIEW_KEY, PROJECT_VIEW);
    mount({ chats: chatsIn("ws-1", "Harness", 15) });
    const [more] = capButton(/^Show 3 more$/);
    expect(more.getAttribute("aria-expanded")).toBe("false");
    expect(text()).not.toContain("Harness 0");

    click(more);
    const [less] = capButton(/^Show less$/);
    expect(less.getAttribute("aria-expanded")).toBe("true");
    expect(text()).toContain("Harness 0");
    expect(capButton(/^Show \d+ more$/)).toHaveLength(0);

    click(less);
    expect(text()).not.toContain("Harness 0");
    expect(capButton(/^Show 3 more$/)).toHaveLength(1);
    expect(capButton(/^Show less$/)).toHaveLength(0);
  });

  it("expands and collapses one project without touching another", () => {
    localStorage.setItem(CHAT_VIEW_KEY, PROJECT_VIEW);
    mount({ workspaces: [workspace, second], chats: [...chatsIn("ws-1", "Harness", 14), ...chatsIn("ws-2", "Landing", 13)] });
    click(capButton(/^Show 2 more$/)[0]);
    expect(text()).toContain("Harness 0");
    expect(text()).not.toContain("Landing 0");
    expect(capButton(/^Show 1 more$/)).toHaveLength(1);

    click(capButton(/^Show less$/)[0]);
    expect(text()).not.toContain("Harness 0");
    expect(capButton(/^Show 1 more$/)).toHaveLength(1);

    click(capButton(/^Show 1 more$/)[0]);
    expect(text()).toContain("Landing 0");
    expect(text()).not.toContain("Harness 0");
    expect(capButton(/^Show 2 more$/)).toHaveLength(1);
  });

  it("hides the control while its group is folded", () => {
    localStorage.setItem(CHAT_VIEW_KEY, PROJECT_VIEW);
    mount({ chats: chatsIn("ws-1", "Harness", 15) });
    click(capButton(/^Show 3 more$/)[0]);
    click(groupHeader("harness"));
    expect(capButton(/^Show (less|\d+ more)$/)).toHaveLength(0);

    click(groupHeader("harness"));
    expect(capButton(/^Show less$/)).toHaveLength(1);
    expect(text()).toContain("Harness 0");
  });
});

describe("BridgeSidebar repositories list", () => {
  it("shows plain chats and project chats in the same list", () => {
    mount();
    expect(text()).toContain("Japan relocation");
    expect(text()).toContain("Sidebar redesign");
    expect(text()).toContain("Projects");
    expect(text()).not.toContain("Needs you");
  });

  it("keeps the active chat visible after a re-render", () => {
    mount({ activeSessionId: "project" });
    expect(text()).toContain("Sidebar redesign");
    mount({ activeSessionId: "plain" });
    expect(text()).toContain("Japan relocation");
  });
});

describe("BridgeSidebar apps", () => {
  it("opens the launcher from the Apps row, without a workspace", () => {
    const onOpenApps = vi.fn();
    mount({ workspaces: [], onOpenApps });
    click(container.querySelector('button[aria-label="Apps"]')!);
    expect(onOpenApps).toHaveBeenCalledOnce();
  });

  it("opens a pinned app by its id", () => {
    const onOpenApp = vi.fn();
    mount({ pinnedApps: [{ app: builtinEntry(appById("notes")) }], onOpenApp });
    click(container.querySelector('button[aria-label="Notes"]')!);
    expect(onOpenApp).toHaveBeenCalledWith("notes");
  });
});

describe("BridgeSidebar action rows", () => {
  it("exports idle chats for Mission Control without opening the chat", () => {
    const onOpenSession = vi.fn();
    mount({ onOpenSession });
    const row = container.querySelector<HTMLButtonElement>("button[title^='Japan relocation']")!;
    expect(row.draggable).toBe(true);
    const dataTransfer = { setData: vi.fn(), effectAllowed: "none" };
    const event = new Event("dragstart", { bubbles: true });
    Object.defineProperty(event, "dataTransfer", { value: dataTransfer });
    act(() => row.dispatchEvent(event));
    expect(dataTransfer.setData).toHaveBeenCalledWith(SIDEBAR_CHAT_DRAG, "plain");
    expect(dataTransfer.effectAllowed).toBe("copy");
    expect(onOpenSession).not.toHaveBeenCalled();
  });
  it("fires the matching handler from each action row", () => {
    const onOpenNewChat = vi.fn();
    const onOpenApps = vi.fn();
    const onOpenSettings = vi.fn();
    mount({ onOpenNewChat, onOpenApps, onOpenSettings });
    click(container.querySelector('button[aria-label="New Chat"]')!);
    click(container.querySelector('button[aria-label="Apps"]')!);
    click(container.querySelector('button[aria-label="Open settings for cestercian"]')!);
    expect(onOpenNewChat).toHaveBeenCalledOnce();
    expect(onOpenApps).toHaveBeenCalledOnce();
    expect(onOpenSettings).toHaveBeenCalledOnce();
  });

  it("keeps Terminals and the Work board out of the sidebar", () => {
    mount();
    expect(container.querySelector('button[aria-label="Terminals"]')).toBeNull();
    expect(container.querySelector('button[aria-label="Work board"]')).toBeNull();
  });

  it("opens the filter from Search and restores New Chat on Escape", () => {
    mount();
    expect(container.querySelector('input[aria-label="Filter chats and projects"]')).toBeNull();
    click(container.querySelector('button[aria-label="Search"]')!);
    const input = container.querySelector<HTMLInputElement>('input[aria-label="Filter chats and projects"]')!;
    expect(input).toBeTruthy();
    // The input takes the wide slot, with compose still available beside it.
    expect(container.querySelector('button[aria-label="Search"]')).toBeNull();
    act(() => {
      input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    expect(container.querySelector('input[aria-label="Filter chats and projects"]')).toBeNull();
    expect(container.querySelector('button[aria-label="Search"]')).not.toBeNull();
  });

  it("opens Projects from the new-folder control", () => {
    const onOpenProjects = vi.fn();
    mount({ onOpenProjects });
    click(container.querySelector('button[aria-label="New folder"]')!);
    expect(onOpenProjects).toHaveBeenCalledOnce();
  });

  it("keeps New Chat in place when focus leaves an empty filter for compose", () => {
    const onOpenNewChat = vi.fn();
    mount({ onOpenNewChat });
    click(container.querySelector('button[aria-label="Search"]')!);
    const compose = container.querySelector<HTMLButtonElement>('button[aria-label="New Chat"]')!;
    // A mouse click focuses the button before mouseup. Closing the input on
    // that blur would move compose away from the pointer and lose the click.
    act(() => compose.focus());
    expect(container.querySelector('input[aria-label="Filter chats and projects"]')).not.toBeNull();
    click(compose);
    expect(onOpenNewChat).toHaveBeenCalledOnce();
    expect(container.querySelector('input[aria-label="Filter chats and projects"]')).toBeNull();
    expect(compose.getAttribute("aria-label")).toBe("New Chat");
    expect(compose.textContent).toContain("New Chat");

    click(container.querySelector('button[aria-label="Search"]')!);
    act(() => container.querySelector<HTMLButtonElement>('button[aria-label="Apps"]')!.focus());
    expect(container.querySelector('input[aria-label="Filter chats and projects"]')).toBeNull();
  });

  it("disables New Chat while a session is being created", () => {
    const onOpenNewChat = vi.fn();
    mount({ newChatBusy: true, onOpenNewChat });
    const button = container.querySelector<HTMLButtonElement>('button[aria-label="New Chat"]')!;
    expect(button.disabled).toBe(true);
    click(button);
    expect(onOpenNewChat).not.toHaveBeenCalled();
  });
});

describe("BridgeSidebar collapse", () => {
  it("reports the change when its own header hides it", () => {
    const onCollapsedChange = vi.fn();
    mount({ collapsed: false, onCollapsedChange });
    click(container.querySelector('button[aria-label="Hide sidebar"]')!);
    expect(onCollapsedChange).toHaveBeenCalledWith(true);
  });

  it("takes the rail off the layout entirely, with nothing inside reachable", () => {
    mount({ collapsed: true, onCollapsedChange: noop });
    const aside = container.querySelector("aside")!;
    expect(aside.style.getPropertyValue("--sidebar-w")).toBe("0px");
    // `inert` is what makes it unreachable: no click, no focus, no screen reader.
    expect(aside.hasAttribute("inert")).toBe(true);
    expect(aside.getAttribute("aria-hidden")).toBe("true");
    expect(aside.className).not.toContain("border-r");
  });

  it("comes back at the width it was hidden at", () => {
    localStorage.setItem("bridge.sidebar.width", "288");
    mount({ collapsed: true, onCollapsedChange: noop });
    expect(container.querySelector("aside")!.style.getPropertyValue("--sidebar-w")).toBe("0px");

    mount({ collapsed: false, onCollapsedChange: noop });
    const aside = container.querySelector("aside")!;
    expect(aside.style.getPropertyValue("--sidebar-w")).toBe("288px");
    expect(aside.hasAttribute("inert")).toBe(false);
    expect(container.querySelector('button[aria-label="Hide sidebar"]')).not.toBeNull();
  });

  it("leaves the drawer usable below sm even while the desktop rail is hidden", () => {
    mount({ collapsed: true, mobileOpen: true, onCollapsedChange: noop });
    const aside = container.querySelector("aside")!;
    expect(aside.hasAttribute("inert")).toBe(false);
    expect(text()).toContain("Japan relocation");
  });
});

describe("mobile drawer scrim", () => {
  const scrim = () => container.querySelector<HTMLElement>('button[aria-label="Close navigation"]');

  beforeEach(() => {
    MotionGlobalConfig.skipAnimations = true;
  });

  afterEach(() => {
    MotionGlobalConfig.skipAnimations = false;
  });

  it("fades out on close instead of blinking away", async () => {
    mount({ mobileOpen: true });
    expect(scrim()).not.toBeNull();

    mount({ mobileOpen: false });
    // Still mounted: AnimatePresence is holding it for its fade.
    expect(scrim()).not.toBeNull();

    await act(async () => {
      await new Promise(resolve => setTimeout(resolve, 40));
    });
    expect(scrim()).toBeNull();
  });

  it("keeps the drawer's own Tailwind slide rather than animating it in JS", () => {
    mount({ mobileOpen: false });
    const aside = container.querySelector("aside");
    expect(aside?.className).toContain("transition-transform");
    expect(aside?.className).toContain("-translate-x-full");
  });
});

describe("BridgeSidebar pinned Notes", () => {
  it("sits right after Apps, opens Notes, and shows where you are", () => {
    const onOpenApp = vi.fn();
    mount({ pinnedApps: [{ app: builtinEntry(appById("notes")) }], onOpenApp });
    const rows = [...container.querySelectorAll<HTMLButtonElement>("nav[aria-label='Main navigation'] button")].map(button => button.getAttribute("aria-label"));
    expect(rows).toEqual(["Apps", "Notes"]);
    const notes = container.querySelector<HTMLButtonElement>("button[aria-label='Notes']")!;
    expect(notes.title).toContain("Notes");
    expect(notes.getAttribute("aria-current")).toBeNull();
    click(notes);
    expect(onOpenApp).toHaveBeenCalledWith("notes");
    mount({ pinnedApps: [{ app: builtinEntry(appById("notes")) }], onOpenApp, activeApp: "notes" });
    expect(container.querySelector("button[aria-label='Notes']")?.getAttribute("aria-current")).toBe("page");
  });

  it("unpins a pinned app from the button beside its row, without opening it", () => {
    const onOpenApp = vi.fn();
    const onUnpinApp = vi.fn();
    mount({ pinnedApps: [{ app: builtinEntry(appById("notes")) }, { app: builtinEntry(appById("pulls")) }], onOpenApp, onUnpinApp });
    expect(container.querySelector("button[aria-label='Unpin Apps']")).toBeNull();
    const unpin = container.querySelector<HTMLButtonElement>("button[aria-label='Unpin Pulls']")!;
    // A sibling of the row button, never nested inside it.
    expect(unpin.closest("button[aria-label='Pulls']")).toBeNull();
    click(unpin);
    expect(onUnpinApp).toHaveBeenCalledWith("pulls");
    expect(onOpenApp).not.toHaveBeenCalled();
  });

  it("shows no Unpin when the host cannot unpin", () => {
    mount({ pinnedApps: [{ app: builtinEntry(appById("notes")) }] });
    expect(container.querySelector("button[aria-label='Unpin Notes']")).toBeNull();
  });
});

describe("imported subagents in the rail", () => {
  beforeEach(() => {
    localStorage.setItem(CHAT_VIEW_KEY, DATE_VIEW);
  });

  const parent = session("imp", { title: "Fix the login", kind: "imported", harness: "opencode" });
  const kids = [
    session("kid-a", { title: "Audit the tests", kind: "imported", harness: "opencode", parentSessionId: "imp", startedAt: "2026-11-01T10:01:00Z" }),
    session("kid-b", { title: "Scan the docs", kind: "imported", harness: "opencode", parentSessionId: "imp", startedAt: "2026-11-01T10:02:00Z" }),
  ];
  const all = [parent, session("plain", { title: "Japan relocation" }), ...kids];
  const toggle = () => container.querySelector<HTMLButtonElement>("[aria-label$='subagents of Fix the login']")!;

  it("counts a chat's imported subagents, folded, and unfolds to open one", () => {
    const opened: string[] = [];
    mount({ chats: [parent, all[1]], subagents: importedSubagents(all), onOpenSession: id => opened.push(id) });

    expect(toggle().textContent).toBe("2 subagents");
    expect(toggle().getAttribute("aria-expanded")).toBe("false");
    expect(container.querySelector("[data-subagent-row]")).toBeNull();
    expect(text()).not.toContain("Audit the tests");

    click(toggle());
    const rows = [...container.querySelectorAll<HTMLButtonElement>("[data-subagent-row]")];
    expect(rows.map(row => row.getAttribute("data-subagent-row"))).toEqual(["kid-a", "kid-b"]);
    click(rows[1]);
    expect(opened).toEqual(["kid-b"]);

    click(toggle());
    expect(container.querySelector("[data-subagent-row]")).toBeNull();
  });

  it("unfolds the chat holding the open subagent by itself", () => {
    mount({ chats: [parent, all[1]], subagents: importedSubagents(all), activeSessionId: "kid-a" });

    expect(toggle().getAttribute("aria-expanded")).toBe("true");
    const active = container.querySelector("[data-subagent-row='kid-a']")!;
    expect(active.getAttribute("aria-current")).toBe("page");
  });

  it("filters subagents with their chat and never lists them at the top", () => {
    mount({ chats: [all[1]], subagents: importedSubagents(all) });

    expect(toggle()).toBeNull();
    expect(text()).not.toContain("Audit the tests");
  });
});


describe("sidebar trail button", () => {
  const trail = {
    id: "trail-1", title: "Archive a chat", state: "needsYou", stage: 3, harnesses: [{ harness: "codex", chats: 1 }],
    sessionIds: ["plain"], memberCount: 3, needsYouReason: "vitest is failing on #12",
    createdAt: "2026-11-20T00:00:00Z", updatedAt: "2026-11-20T00:00:00Z",
  } as const;

  it("shows a trail button only on chats on a trail, revealed with the row", () => {
    mount({ trails: [{ ...trail, sessionIds: [...trail.sessionIds], harnesses: [...trail.harnesses] }], onOpenTrail: vi.fn() });
    const buttons = [...container.querySelectorAll<HTMLButtonElement>("[data-sidebar-trail]")];
    expect(buttons).toHaveLength(1);
    expect(buttons[0].getAttribute("aria-label")).toBe("Trail: Archive a chat");
    // It belongs to the "Japan relocation" row, not the other chat.
    expect(buttons[0].closest(".group\\/row")?.textContent).toContain("Japan relocation");
    // Hidden at rest; the row's hover, focus inside it or an open card reveals it.
    expect(buttons[0].className).toContain("hidden");
    for (const reveal of ["group-hover/row:grid", "group-focus-within/row:grid", "data-popup-open:grid"]) expect(buttons[0].className).toContain(reveal);
  });

  it("opens the trail on click without opening the chat", () => {
    const onOpenTrail = vi.fn();
    const onOpenSession = vi.fn();
    mount({ trails: [{ ...trail, sessionIds: [...trail.sessionIds], harnesses: [...trail.harnesses] }], onOpenTrail, onOpenSession });
    click(container.querySelector("[data-sidebar-trail]")!);
    expect(onOpenTrail).toHaveBeenCalledWith("trail-1");
    expect(onOpenSession).not.toHaveBeenCalled();
  });

  it("draws no trail button without trails or a way to open one", () => {
    mount({ onOpenTrail: vi.fn() });
    expect(container.querySelector("[data-sidebar-trail]")).toBeNull();
    mount({ trails: [{ ...trail, sessionIds: [...trail.sessionIds], harnesses: [...trail.harnesses] }] });
    expect(container.querySelector("[data-sidebar-trail]")).toBeNull();
  });
});
