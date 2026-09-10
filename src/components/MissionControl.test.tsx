// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { AgentEvent, Session, SessionForestSnapshot, Workspace } from "../types";
import { MissionControl } from "./MissionControl";
import { MISSION_LAYOUT_KEY } from "./missionControl/layout";
import { leafIds, type PaneNode } from "../terminal/layout";

vi.mock("../api", () => ({
  bridgeApi: {
    sessionForest: vi.fn(),
    submitInput: vi.fn(),
    resolveApproval: vi.fn(),
    resolveQuestion: vi.fn(),
    interruptTurn: vi.fn(),
  },
}));
vi.mock("./AgentConversation", () => ({
  AgentConversation: ({ session, onResolve }: { session?: Session; onResolve: (eventId: number, decision: string) => unknown }) =>
    <div data-conversation={session?.id}><button type="button" data-approve={session?.id} onClick={() => { void onResolve(7, "accept"); }}>approve</button></div>,
}));

import { bridgeApi } from "../api";

const forest = (sessionId: string): SessionForestSnapshot => ({
  sessionId, entries: [], leaves: [], workerLeases: [], workerRuntimes: [], workerQueue: [], usage: [], reasons: [],
  head: { sessionId, activeEntryId: null, nativeProviderSessionId: null, restorationMode: "fresh", resumeEligibility: "fresh", latestCheckpointEntryId: null, updatedAt: "2026-01-01T00:00:00Z" },
  policyLimits: { maxWorkersPerTurn: 3, maxStrongWorkersPerTurn: 1, maxCapabilityUnitsPerTurn: 24 },
  repositoryDivergence: { status: "unknown", selectedState: null, currentState: { status: "unavailable" } }, completion: null,
  entryWindow: { returned: 0, total: 0, trimmedPayloads: 0 },
} as unknown as SessionForestSnapshot);

const session = (id: string, status: Session["status"], extra: Partial<Session> = {}): Session => ({
  id, label: `Chat ${id}`, status, harness: "claude", kind: "chat", metricSource: "provider", continuationFidelity: "full", restorationMode: "fresh", startedAt: "2026-01-01T00:00:00Z", ...extra,
} as Session);
const workspaces = [{ id: "ws", title: "Bridge", branch: "main" }] as Workspace[];
const noEvents: AgentEvent[] = [];

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  const store = new Map<string, string>();
  vi.stubGlobal("localStorage", { getItem: (key: string) => store.get(key) ?? null, setItem: (key: string, value: string) => store.set(key, value), removeItem: (key: string) => store.delete(key) });
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  vi.mocked(bridgeApi.sessionForest).mockReset().mockImplementation(id => Promise.resolve(forest(id)));
  vi.mocked(bridgeApi.submitInput).mockReset().mockResolvedValue({ disposition: "startedNewTurn", interceptions: [] });
  vi.mocked(bridgeApi.resolveApproval).mockReset().mockResolvedValue({} as never);
  vi.mocked(bridgeApi.interruptTurn).mockReset().mockResolvedValue(undefined);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(() => { act(() => root.unmount()); host.remove(); vi.unstubAllGlobals(); });

const tiles = () => [...host.querySelectorAll<HTMLElement>("[data-session-id]")].map(el => el.dataset.sessionId);
const render = async (props: Partial<Parameters<typeof MissionControl>[0]>) => {
  await act(async () => root.render(<MissionControl sessions={[]} workspaces={workspaces} events={noEvents} onFocusSession={vi.fn()} {...props} />));
};

it("shows only active sessions until Show all is toggled", async () => {
  const sessions = [session("a", "working"), session("b", "completed"), session("c", "waiting")];
  await render({ sessions });
  expect(tiles().sort()).toEqual(["a", "c"]);
  expect(host.textContent).toContain("2 live");
  const toggle = host.querySelector<HTMLButtonElement>("button[aria-pressed]")!;
  await act(async () => toggle.click());
  expect(tiles().sort()).toEqual(["a", "b", "c"]);
  expect(toggle.getAttribute("aria-pressed")).toBe("true");
});

it("explains the empty state and offers Show all", async () => {
  await render({ sessions: [session("b", "completed")] });
  expect(tiles()).toEqual([]);
  expect(host.textContent).toContain("Active chats appear here automatically");
  const button = [...host.querySelectorAll("button")].find(el => el.textContent === "Show all chats")!;
  await act(async () => button.click());
  expect(tiles()).toEqual(["b"]);
});

it("renders the real conversation and a composer per tile", async () => {
  await render({ sessions: [session("a", "working")] });
  expect(host.querySelector("[data-conversation='a']")).not.toBeNull();
  expect(host.querySelector("[data-session-id='a'] textarea")).not.toBeNull();
  expect(bridgeApi.sessionForest).toHaveBeenCalledWith("a");
});

it("submits through the tile's own session id and clears the draft", async () => {
  await render({ sessions: [session("a", "working"), session("b", "waiting")] });
  const textarea = host.querySelector<HTMLTextAreaElement>("[data-session-id='b'] textarea")!;
  await act(async () => {
    const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
    setter.call(textarea, "ship it"); textarea.dispatchEvent(new Event("input", { bubbles: true }));
  });
  expect(textarea.value).toBe("ship it");
  await act(async () => { textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
  expect(bridgeApi.submitInput).toHaveBeenCalledWith("b", "ship it");
  expect(textarea.value).toBe("");
});

it("resolves approvals against the tile's session, not the selected one", async () => {
  await render({ sessions: [session("a", "working"), session("b", "waiting")], activeSessionId: "a" });
  await act(async () => host.querySelector<HTMLButtonElement>("[data-approve='b']")!.click());
  expect(bridgeApi.resolveApproval).toHaveBeenCalledWith("b", 7, "accept", undefined);
  expect(host.querySelector("[data-session-id='a']")?.className).toContain("ring-1");
  expect(host.querySelector("[data-session-id='b']")?.className).not.toContain("ring-1");
});

it("auto-inserts a new active session and drops one that stopped", async () => {
  await render({ sessions: [session("a", "working")] });
  expect(tiles()).toEqual(["a"]);
  await render({ sessions: [session("a", "working"), session("b", "working")] });
  expect(tiles().sort()).toEqual(["a", "b"]);
  expect(host.querySelector("[role='separator']")).not.toBeNull();
  await render({ sessions: [session("a", "completed"), session("b", "working")] });
  expect(tiles()).toEqual(["b"]);
  expect(host.querySelector("[role='separator']")).toBeNull();
});

it("persists the layout and restores it, dropping stale leaves", async () => {
  await render({ sessions: [session("a", "working"), session("b", "working")] });
  const saved = JSON.parse(localStorage.getItem(MISSION_LAYOUT_KEY)!) as { version: number; root: PaneNode };
  expect(saved.version).toBe(1);
  expect(leafIds(saved.root).sort()).toEqual(["a", "b"]);
  act(() => root.unmount());
  // a stale leaf nested beside a live one collapses away; the surviving split keeps its direction and ratio.
  const before: PaneNode = { type: "split", direction: "vertical", ratio: 0.3, first: { type: "leaf", leafId: "b" }, second: { type: "split", direction: "horizontal", ratio: 0.5, first: { type: "leaf", leafId: "a" }, second: { type: "leaf", leafId: "gone" } } };
  localStorage.setItem(MISSION_LAYOUT_KEY, JSON.stringify({ version: 1, root: before, expandedLeafId: null }));
  root = createRoot(host);
  await render({ sessions: [session("a", "working"), session("b", "working")] });
  const restored = JSON.parse(localStorage.getItem(MISSION_LAYOUT_KEY)!) as { root: PaneNode };
  expect(leafIds(restored.root)).toEqual(["b", "a"]);
  expect(host.querySelector("[role='separator']")?.getAttribute("aria-orientation")).toBe("horizontal");
  expect(host.querySelector("[role='separator']")?.getAttribute("aria-valuenow")).toBe("30");
});

it("focuses a session from the tile header and maximizes a tile", async () => {
  const focus = vi.fn();
  await render({ sessions: [session("a", "working"), session("b", "working")], onFocusSession: focus });
  await act(async () => host.querySelector<HTMLButtonElement>("[data-session-id='b'] button[aria-label='Focus chat']")!.click());
  expect(focus).toHaveBeenCalledWith("b");
  await act(async () => host.querySelector<HTMLButtonElement>("[data-session-id='b'] button[aria-label='Maximize tile']")!.click());
  expect(tiles()).toEqual(["b"]);
  await act(async () => host.querySelector<HTMLButtonElement>("button[aria-label='Restore grid']")!.click());
  expect(tiles().sort()).toEqual(["a", "b"]);
});

it("offers Stop only for workers and routes it to onStopWorker", async () => {
  const stop = vi.fn().mockResolvedValue(undefined);
  await render({ sessions: [session("a", "working"), session("w", "working", { parentSessionId: "a" })], onStopWorker: stop });
  expect(host.querySelector("[data-session-id='a'] button[aria-label='Stop worker']")).toBeNull();
  await act(async () => host.querySelector<HTMLButtonElement>("[data-session-id='w'] button[aria-label='Stop worker']")!.click());
  expect(stop).toHaveBeenCalledWith("w");
});
