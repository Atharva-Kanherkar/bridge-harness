// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { asWireKind } from "../transcript/wire";
import type { AgentEvent, Session, Workspace } from "../types";
import { BridgeSidebar, type BridgeSidebarProps } from "./BridgeSidebar";

// The Running group above Projects: which chats it lists, what each row says,
// one ticking clock, and a click that opens the chat.

const NOW = Date.parse("2026-11-21T10:00:00Z");

const session = (id: string, overrides: Partial<Session> = {}): Session => ({
  id,
  workspaceId: null,
  harness: "codex",
  label: id,
  title: id,
  model: null,
  status: "ready",
  activeTurnId: null,
  startedAt: new Date(NOW - 3_600_000).toISOString(),
  endedAt: null,
  contextPercent: null,
  usagePercent: null,
  metricSource: "reported",
  restorationMode: "fresh",
  continuationFidelity: "native",
  kind: "orchestrator",
  ...overrides,
} as Session);

const frame = (id: number, sessionId: string, kind: string, fields: Partial<AgentEvent> = {}): AgentEvent => ({
  id, sessionId, sequence: id, protocolVersion: 1, kind: asWireKind(kind), itemId: null, role: null, status: null,
  title: null, text: null, data: {}, providerMeta: {}, createdAt: new Date(NOW).toISOString(), ...fields,
});

const workspaces = [
  { id: "ws-harness", title: "bridge-harness", branch: "main", status: "ready", dirtyFiles: 0 },
  { id: "ws-landing", title: "landing", branch: "main", status: "ready", dirtyFiles: 0 },
] as Workspace[];

const retry = session("retry", { title: "Add a retry budget", workspaceId: "ws-harness", status: "working", activeTurnId: "turn-1" });
const deck = session("deck", { title: "Polish the Deck shell", workspaceId: "ws-landing", harness: "claude" });
const daemon = session("daemon", { title: "Fix stale daemon attach", workspaceId: "ws-harness" });
const providerUp = session("provider-up", { title: "Provider is up", workspaceId: "ws-harness", status: "working", activeTurnId: null });

const noop = () => {};

const props = (overrides: Partial<BridgeSidebarProps> = {}): BridgeSidebarProps => ({
  chats: [retry, deck, daemon],
  liveAgents: new Map([["deck", ["deck-w1"]]]),
  sessions: [retry, deck, daemon, session("deck-w1", { parentSessionId: "deck", status: "waiting", kind: "worker", startedAt: new Date(NOW - 72_000).toISOString() })],
  liveEvents: [
    frame(1, "retry", "turn.started", { createdAt: new Date(NOW - 13_000).toISOString() }),
    frame(2, "retry", "tool.started", { itemId: "run", status: "inProgress", data: { type: "commandExecution", command: "cargo test" } }),
  ],
  workspaces,
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

const group = () => container.querySelector<HTMLElement>("[data-sidebar-running]");
const rows = () => [...container.querySelectorAll<HTMLButtonElement>("[data-running-row]")];
const row = (id: string) => container.querySelector<HTMLButtonElement>(`[data-running-row="${id}"]`)!;

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  vi.useFakeTimers({ toFake: ["setInterval", "clearInterval", "setTimeout", "clearTimeout", "Date"] });
  vi.setSystemTime(NOW);
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
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("sidebar Running group", () => {
  it("lists every chat with a turn or a worker in flight, across projects, above Projects", () => {
    mount();
    expect(group()?.getAttribute("aria-label")).toBe("Running");
    expect(group()!.textContent).toContain("Running2");
    expect(rows().map(button => button.dataset.runningRow)).toEqual(["retry", "deck"]);
    // Above the project tree, not inside it.
    const projects = [...container.querySelectorAll("span")].find(node => node.textContent === "Projects")!;
    expect(group()!.compareDocumentPosition(projects) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("keeps each running chat in its project folder too", () => {
    mount();
    const outsideGroup = [...container.querySelectorAll("[aria-description]")].map(node => node.textContent ?? "");
    expect(outsideGroup.some(textContent => textContent.includes("Add a retry budget"))).toBe(true);
    expect(outsideGroup.some(textContent => textContent.includes("Polish the Deck shell"))).toBe(true);
  });

  it("says the step in flight, or what the workers need, with the harness mark last", () => {
    mount();
    expect(row("retry").textContent).toBe("Add a retry budget0:13·Running cargo test");
    expect(row("deck").textContent).toBe("Polish the Deck shell1:12·1 worker needs you");
    expect(row("deck").querySelector('[data-running-detail="needsYou"]')?.className).toContain("text-warning");
    expect(row("retry").lastElementChild?.getAttribute("data-harness")).toBe("codex");
  });

  it("names the chat in full on hover without taking the chat row's title", () => {
    mount();
    // Hosts find a chat's own row by its title; the running row must not answer first.
    expect(row("retry").hasAttribute("title")).toBe(false);
    expect(row("retry").querySelector("[title]")?.getAttribute("title")).toBe("Add a retry budget · Codex");
    expect(container.querySelector('button[title^="Add a retry budget"]')?.hasAttribute("data-running-row")).toBe(false);
  });

  it("leaves out a chat whose provider is up with no turn, and hides when nothing runs", () => {
    mount({ chats: [providerUp, daemon], liveAgents: new Map(), sessions: [providerUp, daemon], liveEvents: [] });
    expect(group()).toBeNull();
    expect(container.textContent).not.toContain("Running");
    // The chat is still in its folder, as it was.
    expect(container.textContent).toContain("Provider is up");
  });

  it("joins a chat the moment a send to it is on its way", () => {
    mount({ chats: [daemon], liveAgents: new Map(), sessions: [daemon], liveEvents: [], sendingChatIds: new Set(["daemon"]) });
    expect(rows().map(button => button.dataset.runningRow)).toEqual(["daemon"]);
    expect(row("daemon").textContent).toContain("0:00·Working");
  });

  it("ticks every timer once a second off a single interval", () => {
    const intervals = vi.spyOn(window, "setInterval");
    mount();
    expect(intervals.mock.calls.filter(([, delay]) => delay === 1000)).toHaveLength(1);
    act(() => { vi.advanceTimersByTime(1000); });
    expect(row("retry").querySelector("[data-running-elapsed]")!.textContent).toBe("0:14");
    expect(row("deck").querySelector("[data-running-elapsed]")!.textContent).toBe("1:13");
    act(() => { vi.advanceTimersByTime(47_000); });
    expect(row("retry").querySelector("[data-running-elapsed]")!.textContent).toBe("1:01");
  });

  it("keeps a row's clock when the live buffer drops its start", () => {
    mount();
    act(() => { vi.advanceTimersByTime(5_000); });
    mount({ liveEvents: [] });
    expect(row("retry").querySelector("[data-running-elapsed]")!.textContent).toBe("0:18");
  });

  it("opens the chat on click and marks the open one", () => {
    const onOpenSession = vi.fn();
    mount({ onOpenSession, activeSessionId: "retry" });
    expect(row("retry").dataset.active).toBe("true");
    expect(row("retry").className).toContain("bg-selection");
    expect(row("deck").dataset.active).toBeUndefined();
    act(() => { row("deck").dispatchEvent(new MouseEvent("click", { bubbles: true })); });
    expect(onOpenSession).toHaveBeenCalledWith("deck");
  });

  it("steps aside while a search has the rail", () => {
    mount();
    const search = container.querySelector<HTMLButtonElement>('button[aria-label="Search"]')!;
    act(() => { search.dispatchEvent(new MouseEvent("click", { bubbles: true })); });
    const field = container.querySelector<HTMLInputElement>('input[aria-label="Filter chats and projects"]')!;
    act(() => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
      setter.call(field, "deck");
      field.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(group()).toBeNull();
  });
});
