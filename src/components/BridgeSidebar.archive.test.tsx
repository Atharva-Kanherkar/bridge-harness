// @vitest-environment jsdom
//
// The archive affordance, and the wiring that makes it reachable.
//
// The second test here exists because of a real miss: the row rendered its
// button correctly and `App` held a working handler, but the prop between them
// was never passed, so the feature was unreachable in the desktop app while
// every component test stayed green. A component test cannot see that gap, so
// this file also asserts the wiring at the source level — the same trick
// `src-tauri/src/lib.rs` uses to pin `generate_handler!` against the method
// registry.
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { Session, Workspace } from "../types";
import { BridgeSidebar, type BridgeSidebarProps } from "./BridgeSidebar";

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
  chats: [session("plain", { title: "Japan relocation" })],
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

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  // This jsdom instance has no storage of its own, and the rail reads persisted
  // width/collapse/view during render — same shim the interaction suite uses.
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

function mount(overrides: Partial<BridgeSidebarProps> = {}) {
  act(() => {
    root.render(<BridgeSidebar {...props(overrides)} />);
  });
}

const archiveButton = () =>
  [...container.querySelectorAll<HTMLButtonElement>("button")]
    .find(button => button.getAttribute("aria-label")?.startsWith("Archive "));

it("offers an archive action per chat and hands back the chat itself", () => {
  const onArchiveChat = vi.fn();
  mount({ onArchiveChat });

  const button = archiveButton();
  expect(button).toBeTruthy();
  expect(button?.getAttribute("aria-label")).toBe("Archive Japan relocation");

  act(() => {
    button?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
  expect(onArchiveChat).toHaveBeenCalledTimes(1);
  expect(onArchiveChat.mock.calls[0][0].id).toBe("plain");
});

it("does not open the chat when its archive action is clicked", () => {
  const onOpenSession = vi.fn();
  mount({ onArchiveChat: noop, onOpenSession });
  act(() => {
    archiveButton()?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
  expect(onOpenSession).not.toHaveBeenCalled();
});

it("renders no archive action when the host cannot archive", () => {
  mount();
  expect(archiveButton()).toBeUndefined();
});

// The reachability guard. A component test proves the row works; only this
// proves a desktop user can get to it.
it("is wired from App, so the affordance is reachable in the real app", () => {
  // `import.meta.url` is not a file URL under this transform, so resolve from
  // the project root instead.
  const app = readFileSync(resolve(process.cwd(), "src/App.tsx"), "utf8");
  expect(app).toMatch(/onArchiveChat=\{/);
  const sidebar = app.slice(app.indexOf("<BridgeSidebar"));
  expect(sidebar.slice(0, sidebar.indexOf("/>"))).toMatch(/onArchiveChat=\{/);
});
