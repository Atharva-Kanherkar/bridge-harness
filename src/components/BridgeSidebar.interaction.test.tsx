// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MotionGlobalConfig } from "framer-motion";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Session, Workspace } from "../types";
import { BridgeSidebar, type BridgeSidebarProps } from "./BridgeSidebar";
import { CHAT_VIEW_KEY } from "./sidebarChats";
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
  projectsActive: false,
  marketplaceActive: false,
  agentFleetActive: false,
  missionControlActive: false,
  settingsActive: false,
  accountName: "cestercian",
  onOpenNewChat: noop,
  onOpenProjects: noop,
  onOpenMarketplace: noop,
  onOpenAgentFleet: noop,
  onOpenMissionControl: noop,
  onOpenWorkBoard: noop,
  onOpenMemory: noop,
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

describe("BridgeSidebar repositories list", () => {
  it("shows plain chats and project chats in the same list", () => {
    mount();
    expect(text()).toContain("Japan relocation");
    expect(text()).toContain("Sidebar redesign");
    expect(text()).toContain("Repositories");
    expect(text()).not.toContain("Needs you");
  });

  it("keeps the active chat visible after a re-render", () => {
    mount({ activeSessionId: "project" });
    expect(text()).toContain("Sidebar redesign");
    mount({ activeSessionId: "plain" });
    expect(text()).toContain("Japan relocation");
  });
});

describe("BridgeSidebar account actions", () => {
  it("the memory row opens account memory without a workspace", () => {
    const onOpenMemory = vi.fn();
    mount({ workspaces: [], onOpenMemory });
    const row = [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => button.textContent === "Memory")!;
    act(() => row.click());
    expect(onOpenMemory).toHaveBeenCalledOnce();
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
    const onOpenMarketplace = vi.fn();
    const onOpenSettings = vi.fn();
    mount({ onOpenNewChat, onOpenMarketplace, onOpenSettings });
    click(container.querySelector('button[aria-label="New Chat"]')!);
    click(container.querySelector('button[aria-label="Marketplace"]')!);
    click(container.querySelector('button[aria-label="Open settings for cestercian"]')!);
    expect(onOpenNewChat).toHaveBeenCalledOnce();
    expect(onOpenMarketplace).toHaveBeenCalledOnce();
    expect(onOpenSettings).toHaveBeenCalledOnce();
  });

  it("renders Agent Fleet and keeps the Work board hidden", () => {
    mount();
    expect(container.querySelector('button[aria-label="Agent Fleet"]')).not.toBeNull();
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
    act(() => container.querySelector<HTMLButtonElement>('button[aria-label="Marketplace"]')!.focus());
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
