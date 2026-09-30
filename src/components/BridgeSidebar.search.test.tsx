// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import type { SearchChatsResult } from "../protocol/generated/protocol";
import type { Session } from "../types";
import { BridgeSidebar, type BridgeSidebarProps } from "./BridgeSidebar";

// Cross-chat search from the rail: typed queries hit the index, Enter either
// opens the clear winner or searches deeper, and a card opens its chat.

const session = (id: string, title: string): Session => ({
  id, workspaceId: null, harness: "codex", label: id, title, model: null, status: "idle",
  startedAt: new Date().toISOString(), endedAt: null, contextPercent: null, usagePercent: null,
  metricSource: "reported", restorationMode: "fresh", continuationFidelity: "native",
} as Session);

const noop = () => {};

const props = (overrides: Partial<BridgeSidebarProps> = {}): BridgeSidebarProps => ({
  chats: [session("plugins", "Plugins catalog"), session("other", "Japan relocation")],
  workspaces: [],
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

const indexResult = (query: string, overrides: Partial<SearchChatsResult> = {}): SearchChatsResult => ({
  query,
  hits: [
    { sessionId: "plugins", title: "Plugins catalog", harness: "codex", lastActiveAt: new Date().toISOString(), matchCount: 3, snippet: "the marketplace stalls on an unbounded CLI call", score: 0.06, why: "topic and 2 messages match", archived: false, ended: false },
    { sessionId: "other", title: "Japan relocation", harness: "claude", lastActiveAt: new Date().toISOString(), matchCount: 1, snippet: "a stall at the station", score: 0.05, why: "1 message matches", archived: true, ended: true },
  ],
  stage: "index",
  confident: false,
  deepAvailable: true,
  terms: ["stall"],
  elapsedMs: 4,
  modelTokens: 0,
  toolCalls: 0,
  ...overrides,
});

let container: HTMLDivElement;
let root: Root;

function mount(overrides: Partial<BridgeSidebarProps> = {}) {
  act(() => { root.render(<BridgeSidebar {...props(overrides)} />); });
}

const text = () => container.textContent ?? "";
const input = () => container.querySelector<HTMLInputElement>('input[aria-label="Filter chats and projects"]')!;

function type(value: string) {
  act(() => {
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
    setter.call(input(), value);
    input().dispatchEvent(new Event("input", { bubbles: true }));
  });
}

function press(key: string) {
  act(() => { input().dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true })); });
}

async function settle(ms = 200) {
  await act(async () => { vi.advanceTimersByTime(ms); });
  await act(async () => { await Promise.resolve(); });
}

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  vi.useFakeTimers();
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

describe("sidebar chat search", () => {
  it("shows index hits under In conversations as you type, index only", async () => {
    const search = vi.spyOn(bridgeApi, "searchChats").mockImplementation(async query => indexResult(query));
    mount();
    act(() => { container.querySelector<HTMLButtonElement>('button[aria-label="Search"]')!.dispatchEvent(new MouseEvent("click", { bubbles: true })); });
    type("stall");
    expect(search).not.toHaveBeenCalled();
    await settle();
    expect(search).toHaveBeenCalledTimes(1);
    expect(search).toHaveBeenLastCalledWith("stall");
    expect(text()).toContain("In conversations");
    expect(text()).toContain("the marketplace stalls on an unbounded CLI call");
    expect(text()).toContain("topic and 2 messages match");
    expect(text()).toContain("archived");
    expect(text()).toContain("Press Enter to search deeper");
  });

  it("searches deeper on Enter when the index is unsure, then shows the model's reasons", async () => {
    let resolveDeep: (value: SearchChatsResult) => void = () => {};
    const search = vi.spyOn(bridgeApi, "searchChats").mockImplementation((query, options) => {
      if (options?.deep) return new Promise(resolve => { resolveDeep = resolve; });
      return Promise.resolve(indexResult(query));
    });
    mount();
    act(() => { container.querySelector<HTMLButtonElement>('button[aria-label="Search"]')!.dispatchEvent(new MouseEvent("click", { bubbles: true })); });
    type("stall");
    await settle();
    press("Enter");
    expect(search).toHaveBeenLastCalledWith("stall", { deep: true });
    expect(text()).toContain("Searching deeper…");
    await act(async () => {
      resolveDeep(indexResult("stall", {
        stage: "model",
        modelTokens: 900,
        hits: [{ ...indexResult("stall").hits[0], why: "the plugins catalog stall you fixed" }],
      }));
      await Promise.resolve();
    });
    expect(text()).not.toContain("Searching deeper…");
    expect(text()).toContain("the plugins catalog stall you fixed");
  });

  it("opens the clear winner on Enter without a deep search", async () => {
    const onOpenSession = vi.fn();
    const search = vi.spyOn(bridgeApi, "searchChats").mockImplementation(async query => indexResult(query, { confident: true, deepAvailable: false }));
    mount({ onOpenSession });
    act(() => { container.querySelector<HTMLButtonElement>('button[aria-label="Search"]')!.dispatchEvent(new MouseEvent("click", { bubbles: true })); });
    type("stall");
    await settle();
    press("Enter");
    expect(onOpenSession).toHaveBeenCalledWith("plugins");
    expect(search).toHaveBeenCalledTimes(1);
    expect(search.mock.calls.every(call => !call[1]?.deep)).toBe(true);
  });

  it("opens a chat when its card is clicked", async () => {
    const onOpenSession = vi.fn();
    vi.spyOn(bridgeApi, "searchChats").mockImplementation(async query => indexResult(query));
    mount({ onOpenSession });
    act(() => { container.querySelector<HTMLButtonElement>('button[aria-label="Search"]')!.dispatchEvent(new MouseEvent("click", { bubbles: true })); });
    type("stall");
    await settle();
    const section = container.querySelector('section[aria-label="In conversations"]')!;
    const card = [...section.querySelectorAll("button")].find(button => button.textContent?.includes("Japan relocation"))!;
    act(() => { card.dispatchEvent(new MouseEvent("click", { bubbles: true })); });
    expect(onOpenSession).toHaveBeenCalledWith("other");
  });

  it("drops an index answer that arrives after the query moved on", async () => {
    const pending: Array<() => void> = [];
    vi.spyOn(bridgeApi, "searchChats").mockImplementation(query => new Promise(resolve => {
      pending.push(() => resolve(indexResult(query, { hits: [{ ...indexResult(query).hits[0], snippet: `about ${query}` }] })));
    }));
    mount();
    act(() => { container.querySelector<HTMLButtonElement>('button[aria-label="Search"]')!.dispatchEvent(new MouseEvent("click", { bubbles: true })); });
    type("first");
    await settle();
    type("second");
    await settle();
    await act(async () => { pending[1](); await Promise.resolve(); });
    await act(async () => { pending[0](); await Promise.resolve(); });
    expect(text()).toContain("about second");
    expect(text()).not.toContain("about first");
  });

  it("a /find request opens the field on its query and searches deeper", async () => {
    const search = vi.spyOn(bridgeApi, "searchChats").mockImplementation(async query => indexResult(query));
    mount({ searchRequest: { query: "plugins stall", nonce: 1 } });
    await settle(0);
    expect(input().value).toBe("plugins stall");
    expect(search).toHaveBeenCalledWith("plugins stall", { deep: true });
  });
});
