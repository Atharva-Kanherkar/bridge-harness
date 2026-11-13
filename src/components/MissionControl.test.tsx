// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentEvent, Session, SessionForestSnapshot, Workspace } from "../types";
import { isActiveSession, MissionControl } from "./MissionControl";
import { MISSION_LAYOUT_KEY, parseLayout } from "./missionControl/layout";
import { SIDEBAR_CHAT_DRAG, TILE_DRAG } from "./missionControl/drag";
import type { PaneNode } from "../terminal/layout";
import { SHOW_WORKER_CHATS_STORAGE_KEY } from "../missionControlSettings";

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

function transfer(type: string, id: string) {
  const data = new Map([[type, id]]);
  return { types: [type], getData: (key: string) => data.get(key) ?? "", setData: (key: string, value: string) => data.set(key, value), effectAllowed: "none", dropEffect: "none" };
}

async function dragEvent(target: Element, type: string, dataTransfer: ReturnType<typeof transfer>, x = 0, y = 0) {
  const event = new MouseEvent(type, { bubbles: true, cancelable: true, clientX: x, clientY: y });
  Object.defineProperty(event, "dataTransfer", { value: dataTransfer });
  await act(async () => { target.dispatchEvent(event); });
  return event;
}

const savedLayout = () => parseLayout(localStorage.getItem(MISSION_LAYOUT_KEY));
const queued = () => [...host.querySelectorAll<HTMLElement>("[data-queue-id]")].map(el => el.dataset.queueId);
const toolbarButton = (text: string) => [...host.querySelectorAll<HTMLButtonElement>("[aria-label='Tiles on the stage'] button")].find(button => button.textContent === text);

it("shows only active sessions even with hundreds of idle chats", async () => {
  const sessions = [session("a", "working"), session("c", "waiting"), ...Array.from({ length: 501 }, (_, i) => session(`idle-${i}`, "completed"))];
  await render({ sessions });
  expect(tiles().sort()).toEqual(["a", "c"]);
  expect(host.textContent).toContain("1 needs you");
  expect(host.textContent).toContain("1 working");
  expect(host.textContent).not.toContain("Show all");
  expect(vi.mocked(bridgeApi.sessionForest).mock.calls.map(([id]) => id).sort()).toEqual(["a", "c"]);
});

it("explains the empty state without offering idle chats", async () => {
  await render({ sessions: [session("b", "completed")] });
  expect(tiles()).toEqual([]);
  expect(host.textContent).toContain("Chats and agents appear here automatically");
  expect(host.textContent).not.toContain("Show all");
  expect(bridgeApi.sessionForest).not.toHaveBeenCalled();
});

it("recognizes active turns and lifecycle transitions across harnesses", () => {
  for (const status of ["working", "waiting", "starting", "resuming", "checkpointing"] as const) {
    expect(isActiveSession(session("a", status))).toBe(true);
  }
  expect(isActiveSession(session("a", "completed", { activeTurnId: "turn" }))).toBe(true);
  expect(isActiveSession(session("a", "completed"))).toBe(false);
  expect(isActiveSession(session("w", "completed", { parentSessionId: "a" }))).toBe(false);
});

it("lets a new chat take a finished worker's slot despite a cached working runtime", async () => {
  localStorage.setItem(SHOW_WORKER_CHATS_STORAGE_KEY, "true");
  vi.mocked(bridgeApi.sessionForest).mockImplementation(async id => ({ ...forest(id), workerRuntimes: [{ sessionId: "w", lifecycleState: "working", resultStatus: "pending" }] } as SessionForestSnapshot));
  await render({ sessions: [session("w", "working", { parentSessionId: "a" })] });
  expect(tiles()).toEqual(["w"]);
  await render({ sessions: [session("w", "completed", { parentSessionId: "a" })] });
  expect(tiles()).toEqual(["w"]);
  await render({ sessions: [session("w", "completed", { parentSessionId: "a" }), session("b", "working")] });
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

it("keeps a finished chat in place and lets the next new chat take its slot", async () => {
  await render({ sessions: [session("a", "working")] });
  expect(tiles()).toEqual(["a"]);
  await render({ sessions: [session("a", "working"), session("b", "working")] });
  expect(tiles()).toEqual(["a", "b"]);
  expect(host.querySelector("[role='separator']")).toBeNull();
  await render({ sessions: [session("a", "completed"), session("b", "working")] });
  expect(tiles()).toEqual(["a", "b"]);
  expect(host.querySelector("[data-session-id='a']")?.textContent).not.toContain("Working");
  await render({ sessions: [session("a", "completed"), session("b", "working"), session("c", "working")] });
  expect(tiles()).toEqual(["c", "b"]);
});

it("persists a flat order and migrates a stored split tree in reading order", async () => {
  await render({ sessions: [session("a", "working"), session("b", "working")] });
  const saved = JSON.parse(localStorage.getItem(MISSION_LAYOUT_KEY)!) as { version: number; order: string[] };
  expect(saved).toMatchObject({ version: 2, order: ["a", "b"] });
  act(() => root.unmount());
  const before: PaneNode = { type: "split", direction: "vertical", ratio: 0.3, first: { type: "leaf", leafId: "b" }, second: { type: "split", direction: "horizontal", ratio: 0.5, first: { type: "leaf", leafId: "a" }, second: { type: "leaf", leafId: "gone" } } };
  localStorage.setItem(MISSION_LAYOUT_KEY, JSON.stringify({ version: 1, root: before, expandedLeafId: null }));
  root = createRoot(host);
  await render({ sessions: [session("a", "working"), session("b", "working")] });
  expect(tiles()).toEqual(["b", "a"]);
  expect(savedLayout()).toMatchObject({ version: 2, order: ["b", "a"] });
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
  localStorage.setItem(SHOW_WORKER_CHATS_STORAGE_KEY, "true");
  const stop = vi.fn().mockResolvedValue(undefined);
  await render({ sessions: [session("a", "working"), session("w", "working", { parentSessionId: "a" })], onStopWorker: stop });
  expect(host.querySelector("[data-session-id='a'] button[aria-label='Stop worker']")).toBeNull();
  await act(async () => host.querySelector<HTMLButtonElement>("[data-session-id='w'] button[aria-label='Stop worker']")!.click());
  expect(stop).toHaveBeenCalledWith("w");
});

it("keeps a maximized chat maximized after its turn ends and restores the grid when closed", async () => {
  await render({ sessions: [session("a", "working"), session("b", "working")] });
  await act(async () => host.querySelector<HTMLButtonElement>("[data-session-id='a'] button[aria-label='Maximize tile']")!.click());
  expect(tiles()).toEqual(["a"]);
  expect(queued()).toEqual(["b"]);
  await render({ sessions: [session("a", "completed"), session("b", "working")] });
  expect(tiles()).toEqual(["a"]);
  await act(async () => host.querySelector<HTMLButtonElement>("[data-session-id='a'] button[aria-label='Close chat']")!.click());
  expect(tiles()).toEqual(["b"]);
  expect(savedLayout().expandedId).toBeNull();
});

it("does not transfer a draft when another active chat replaces the only tile", async () => {
  await render({ sessions: [session("a", "working")] });
  const textarea = host.querySelector<HTMLTextAreaElement>("textarea")!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(textarea, "for a only");
    textarea.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await render({ sessions: [session("a", "completed"), session("b", "working")] });
  expect(tiles()).toEqual(["b"]);
  expect(host.querySelector<HTMLTextAreaElement>("textarea")!.value).toBe("");
  expect(bridgeApi.sessionForest).toHaveBeenCalledWith("b");
});

it("accepts an idle sidebar chat into an empty grid and persists it until removed", async () => {
  const sessions = [session("idle", "completed"), session("other", "completed")];
  await render({ sessions });
  const data = transfer(SIDEBAR_CHAT_DRAG, "idle");
  const canvas = host.querySelector("main")!;
  expect((await dragEvent(canvas, "dragover", data)).defaultPrevented).toBe(true);
  expect(data.dropEffect).toBe("copy");
  await dragEvent(canvas, "drop", data);
  expect(tiles()).toEqual(["idle"]);
  expect(savedLayout().pinnedSessionIds).toEqual(["idle"]);
  expect(host.textContent).toContain("0 working");
  expect(host.textContent).toContain("1 pinned");
  expect(bridgeApi.submitInput).not.toHaveBeenCalled();
  act(() => root.unmount()); root = createRoot(host);
  await render({ sessions });
  expect(tiles()).toEqual(["idle"]);
  await act(async () => host.querySelector<HTMLButtonElement>("button[aria-label='Close chat']")!.click());
  expect(tiles()).toEqual([]);
  expect(savedLayout().pinnedSessionIds).toEqual([]);
});

it.each([
  ["left half", 1, "before", ["idle", "a"]],
  ["right half", 419, "after", ["a", "idle"]],
] as const)("puts an idle sidebar chat into the slot on the %s of a tile and pins it", async (_half, x, place, order) => {
  await render({ sessions: [session("a", "working"), session("idle", "completed")] });
  const tile = host.querySelector("[data-session-id='a']")!;
  vi.spyOn(tile, "getBoundingClientRect").mockReturnValue({ left: 0, top: 0, width: 420, height: 360 } as DOMRect);
  const data = transfer(SIDEBAR_CHAT_DRAG, "idle");
  await dragEvent(tile, "dragover", data, x, 100);
  expect(tile.querySelector(`[data-drop-place='${place}']`)).not.toBeNull();
  await dragEvent(tile, "drop", data, x, 100);
  expect(savedLayout().order).toEqual(order);
  expect(savedLayout().pinnedSessionIds).toEqual(["idle"]);
  expect(tiles()).toEqual(order);
});

it("reorders tiles by dragging a header while preserving their drafts", async () => {
  await render({ sessions: [session("a", "working"), session("b", "working")] });
  const textarea = host.querySelector<HTMLTextAreaElement>("[data-session-id='a'] textarea")!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(textarea, "keep this draft");
    textarea.dispatchEvent(new Event("input", { bubbles: true }));
  });
  const data = transfer(TILE_DRAG, "");
  await dragEvent(host.querySelector("[data-session-id='a'] header")!, "dragstart", data);
  expect(data.getData(TILE_DRAG)).toBe("a");
  expect(data.effectAllowed).toBe("move");
  const target = host.querySelector("[data-session-id='b']")!;
  vi.spyOn(target, "getBoundingClientRect").mockReturnValue({ left: 0, top: 0, width: 420, height: 360 } as DOMRect);
  await dragEvent(target, "drop", data, 400, 100);
  expect(savedLayout().order).toEqual(["b", "a"]);
  expect(tiles()).toEqual(["b", "a"]);
  expect(savedLayout().pinnedSessionIds).toEqual([]);
  expect(host.querySelector<HTMLTextAreaElement>("[data-session-id='a'] textarea")!.value).toBe("keep this draft");
  expect(host.querySelector<HTMLTextAreaElement>("[data-session-id='b'] textarea")!.value).toBe("");
});

it("pins an already visible chat once and keeps it after its turn finishes", async () => {
  await render({ sessions: [session("a", "working")] });
  const data = transfer(SIDEBAR_CHAT_DRAG, "a");
  await dragEvent(host.querySelector("[data-session-id='a']")!, "drop", data);
  await dragEvent(host.querySelector("[data-session-id='a']")!, "drop", data);
  expect(tiles()).toEqual(["a"]);
  expect(savedLayout().pinnedSessionIds).toEqual(["a"]);
  await render({ sessions: [session("a", "completed")] });
  expect(tiles()).toEqual(["a"]);
  await render({ sessions: [] });
  expect(tiles()).toEqual([]);
  expect(savedLayout().pinnedSessionIds).toEqual([]);
});

it("ignores unknown chat ids and unrelated drags", async () => {
  await render({ sessions: [session("idle", "completed")] });
  const canvas = host.querySelector("main")!;
  await dragEvent(canvas, "drop", transfer(SIDEBAR_CHAT_DRAG, "missing"));
  await dragEvent(canvas, "drop", transfer("text/plain", "idle"));
  await dragEvent(canvas, "drop", transfer(TILE_DRAG, "idle"));
  expect(tiles()).toEqual([]);
  expect(savedLayout().pinnedSessionIds).toEqual([]);
});

it("treats old saved layouts as unpinned and validates stored pins", () => {
  const root = { type: "leaf", leafId: "a" };
  expect(parseLayout(JSON.stringify({ version: 1, root })).pinnedSessionIds).toEqual([]);
  expect(parseLayout(JSON.stringify({ version: 1, root, pinnedSessionIds: ["a", "a", "missing", 7] })).pinnedSessionIds).toEqual(["a"]);
});

it("treats old saved layouts as having nothing dismissed and dedupes stored dismissals", () => {
  const root = { type: "leaf", leafId: "a" };
  expect(parseLayout(JSON.stringify({ version: 1, root })).dismissedSessionIds).toEqual([]);
  expect(parseLayout(JSON.stringify({ version: 1, root, dismissedSessionIds: ["gone", "gone", 7, null] })).dismissedSessionIds).toEqual(["gone"]);
});

it("forgets a closed chat once it is no longer running, so the closed list stays bounded", async () => {
  await render({ sessions: [session("a", "working"), session("b", "working")] });
  await act(async () => host.querySelector<HTMLButtonElement>("[data-session-id='a'] button[aria-label='Close chat']")!.click());
  expect(savedLayout().dismissedSessionIds).toEqual(["a"]);
  await render({ sessions: [session("a", "completed"), session("b", "working")] });
  expect(savedLayout().dismissedSessionIds).toEqual([]);
  expect(tiles()).toEqual(["b"]);
});

it("pins from the tile header and keeps the chat visible after completion and reopening", async () => {
  await render({ sessions: [session("a", "working")] });
  await act(async () => host.querySelector<HTMLButtonElement>("button[aria-label='Pin chat in Mission Control']")!.click());
  expect(savedLayout().pinnedSessionIds).toEqual(["a"]);
  await render({ sessions: [session("a", "completed")] });
  expect(tiles()).toEqual(["a"]);
  act(() => root.unmount()); root = createRoot(host);
  await render({ sessions: [session("a", "completed")] });
  expect(tiles()).toEqual(["a"]);
  await act(async () => host.querySelector<HTMLButtonElement>("button[aria-label='Close chat']")!.click());
  expect(tiles()).toEqual([]);
});

it("unpins an active chat without stopping it, and a new chat may then replace it once finished", async () => {
  await render({ sessions: [session("a", "working")] });
  await act(async () => host.querySelector<HTMLButtonElement>("button[aria-label='Pin chat in Mission Control']")!.click());
  await act(async () => host.querySelector<HTMLButtonElement>("button[aria-label='Unpin chat']")!.click());
  expect(tiles()).toEqual(["a"]);
  expect(savedLayout().pinnedSessionIds).toEqual([]);
  expect(bridgeApi.interruptTurn).not.toHaveBeenCalled();
  await render({ sessions: [session("a", "completed")] });
  expect(tiles()).toEqual(["a"]);
  await render({ sessions: [session("a", "completed"), session("b", "working")] });
  expect(tiles()).toEqual(["b"]);
});

it("closes an active, unpinned tile immediately and keeps it closed on rerender", async () => {
  await render({ sessions: [session("a", "working"), session("b", "working")] });
  expect(tiles().sort()).toEqual(["a", "b"]);
  await act(async () => host.querySelector<HTMLButtonElement>("[data-session-id='a'] button[aria-label='Close chat']")!.click());
  expect(tiles()).toEqual(["b"]);
  await render({ sessions: [session("a", "working"), session("b", "working")] });
  expect(tiles()).toEqual(["b"]);
});

it("closing a tile does not stop its worker or interrupt its turn", async () => {
  localStorage.setItem(SHOW_WORKER_CHATS_STORAGE_KEY, "true");
  const stop = vi.fn().mockResolvedValue(undefined);
  await render({ sessions: [session("a", "working"), session("w", "working", { parentSessionId: "a" })], onStopWorker: stop });
  await act(async () => host.querySelector<HTMLButtonElement>("[data-session-id='w'] button[aria-label='Close chat']")!.click());
  expect(stop).not.toHaveBeenCalled();
  expect(bridgeApi.interruptTurn).not.toHaveBeenCalled();
});

it("closing a pinned tile clears its pin along with removing it", async () => {
  await render({ sessions: [session("a", "working")] });
  await act(async () => host.querySelector<HTMLButtonElement>("button[aria-label='Pin chat in Mission Control']")!.click());
  expect(savedLayout().pinnedSessionIds).toEqual(["a"]);
  await act(async () => host.querySelector<HTMLButtonElement>("button[aria-label='Close chat']")!.click());
  expect(tiles()).toEqual([]);
  expect(savedLayout().pinnedSessionIds).toEqual([]);
});

it("brings a closed chat back once it is dragged in from the sidebar again", async () => {
  await render({ sessions: [session("a", "working")] });
  await act(async () => host.querySelector<HTMLButtonElement>("button[aria-label='Close chat']")!.click());
  expect(tiles()).toEqual([]);
  const data = transfer(SIDEBAR_CHAT_DRAG, "a");
  const canvas = host.querySelector("main")!;
  await dragEvent(canvas, "drop", data);
  expect(tiles()).toEqual(["a"]);
});

it("hides worker chats from Mission Control by default, but keeps the orchestrator visible", async () => {
  await render({ sessions: [session("a", "working"), session("w", "working", { parentSessionId: "a" })] });
  expect(tiles()).toEqual(["a"]);
});

it("still shows a pinned worker chat even with the default worker-visibility setting", async () => {
  await render({ sessions: [session("a", "working"), session("w", "working", { parentSessionId: "a" })] });
  const data = transfer(SIDEBAR_CHAT_DRAG, "w");
  const canvas = host.querySelector("main")!;
  await dragEvent(canvas, "drop", data);
  expect(tiles().sort()).toEqual(["a", "w"]);
  // The pinned worker is hidden from the visibility filter but still active,
  // so the badge must count it too, not just the auto-surfaced chats.
  expect(host.textContent).toContain("2 working");
});

it("shows worker chats once the setting is turned on", async () => {
  localStorage.setItem(SHOW_WORKER_CHATS_STORAGE_KEY, "true");
  await render({ sessions: [session("a", "working"), session("w", "working", { parentSessionId: "a" })] });
  expect(tiles().sort()).toEqual(["a", "w"]);
});

it("live-syncs worker visibility from an external write, without remounting or re-rendering with new props", async () => {
  const sessions = [session("a", "working"), session("w", "working", { parentSessionId: "a" })];
  await render({ sessions });
  expect(tiles()).toEqual(["a"]);

  // Simulates another window (e.g. Settings) flipping the preference: the
  // mounted MissionControl instance must pick this up through its own
  // storage-event listener, with no new props and no remount.
  await act(async () => {
    localStorage.setItem(SHOW_WORKER_CHATS_STORAGE_KEY, "true");
    window.dispatchEvent(new Event("storage"));
  });
  expect(tiles().sort()).toEqual(["a", "w"]);

  await act(async () => {
    localStorage.setItem(SHOW_WORKER_CHATS_STORAGE_KEY, "false");
    window.dispatchEvent(new Event("storage"));
  });
  expect(tiles()).toEqual(["a"]);
});

const entry = (sessionId: string, sequence: number, kind: string, payload: Record<string, unknown>) =>
  ({ id: `${sessionId}-${sequence}`, sessionId, sequence, kind, payload, contextVisibility: "eligible", createdAt: "2026-01-01T00:00:00Z", semanticSchemaVersion: 1 });

it("names each tile by project and drops the repo from its own link heading", async () => {
  const projects = [{ id: "p", name: "kairo", path: "/src/kairo", createdAt: "2026-01-01T00:00:00Z" }];
  const spaces = [...workspaces, { id: "k", title: "Review worktree", projectId: "p" }] as Workspace[];
  await render({
    sessions: [session("a", "working", { workspaceId: "k", title: "kairo PR #43" }), session("b", "working", { workspaceId: "ws", title: "Session supervisor" })],
    workspaces: spaces, projects,
  });
  const header = (id: string) => host.querySelector(`[data-session-id='${id}'] header`)!;
  expect(header("a").textContent).toContain("kairo");
  expect(header("a").querySelector("h2")?.textContent).toBe("PR #43");
  expect(header("a").querySelector("h2")?.getAttribute("title")).toBe("kairo PR #43");
  expect(header("b").textContent).toContain("Bridge");
  expect(header("b").querySelector("h2")?.textContent).toBe("Session supervisor");
});

it("shows the latest thing you asked each chat under its title", async () => {
  vi.mocked(bridgeApi.sessionForest).mockImplementation(async id => ({ ...forest(id), entries: [
    entry(id, 1, "user.message", { text: "Build the supervisor" }),
    entry(id, 2, "assistant.message", { text: "Working on it" }),
    entry(id, 3, "user.message", { text: "  now add   retries  " }),
  ] } as unknown as SessionForestSnapshot));
  await render({ sessions: [session("a", "working", { title: "Supervisor" })] });
  expect(host.querySelector("[data-session-id='a'] [data-tile-subtitle]")?.textContent).toBe("now add retries");
});

it("titles an unnamed chat by what was asked", async () => {
  vi.mocked(bridgeApi.sessionForest).mockImplementation(async id => ({ ...forest(id), entries: [entry(id, 1, "user.message", { text: "Port the grid to Swift" })] } as unknown as SessionForestSnapshot));
  await render({ sessions: [session("a", "working", { label: "Orchestrator", title: null, model: "opus" })] });
  expect(host.querySelector("[data-session-id='a'] h2")?.textContent).toBe("Port the grid to Swift");
  expect(host.querySelector("[data-session-id='a'] [data-tile-subtitle]")?.textContent).toBe("Claude · opus");
});

it("titles an untitled direct chat by what was asked", async () => {
  vi.mocked(bridgeApi.sessionForest).mockImplementation(async id => ({ ...forest(id), entries: [entry(id, 1, "user.message", { text: "Port the grid to Swift" })] } as unknown as SessionForestSnapshot));
  await render({ sessions: [session("a", "working", { label: "Chat", kind: "direct", title: null, model: "opus" })] });
  expect(host.querySelector("[data-session-id='a'] h2")?.textContent).toBe("Port the grid to Swift");
});

it("does not repeat the chat title in the composer", async () => {
  await render({ sessions: [session("a", "working", { title: "Session supervisor" }), session("b", "completed", { title: "Idle one" })] });
  expect(host.querySelector<HTMLTextAreaElement>("[data-session-id='a'] textarea")?.placeholder).toBe("Steer this chat…");
  localStorage.setItem(MISSION_LAYOUT_KEY, JSON.stringify({ version: 1, root: { type: "leaf", leafId: "b" }, pinnedSessionIds: ["b"] }));
  act(() => root.unmount()); root = createRoot(host);
  await render({ sessions: [session("b", "completed", { title: "Idle one" })] });
  expect(host.querySelector<HTMLTextAreaElement>("[data-session-id='b'] textarea")?.placeholder).toBe("Reply…");
});

it("flags only tiles that need you and jumps to the next one after where you are", async () => {
  await render({ sessions: [session("a", "working"), session("b", "waiting"), session("c", "waiting")] });
  const flagged = () => [...host.querySelectorAll<HTMLElement>("[data-attention='true']")].map(el => el.dataset.sessionId).sort();
  expect(flagged()).toEqual(["b", "c"]);
  expect(host.querySelector("[data-session-id='a']")?.className).not.toContain("border-foreground/45");
  expect(host.querySelector("[data-session-id='b']")?.className).toContain("border-foreground/45");
  const order = tiles().filter(id => id !== "a");
  const jump = [...host.querySelectorAll("button")].find(button => button.textContent === "2 needs you")!;
  const focusedTile = () => document.activeElement?.closest("[data-session-id]")?.getAttribute("data-session-id");
  const visits: (string | null | undefined)[] = [];
  for (let press = 0; press < 3; press += 1) { await act(async () => jump.click()); visits.push(focusedTile()); }
  expect(visits).toEqual([order[0], order[1], order[0]]);
  // anchored to where you are: from the last waiting tile it wraps to the first.
  await act(async () => host.querySelector<HTMLTextAreaElement>(`[data-session-id='${order[1]}'] textarea`)!.focus());
  await act(async () => jump.click());
  expect(focusedTile()).toBe(order[0]);
});

it("does not count a worker's blocked result as needing you", async () => {
  localStorage.setItem(SHOW_WORKER_CHATS_STORAGE_KEY, "true");
  vi.mocked(bridgeApi.sessionForest).mockImplementation(async id => ({ ...forest(id), workerRuntimes: [{ sessionId: "w", lifecycleState: "working", resultStatus: "reported", lastResult: { status: "blocked", summary: "needs a decision" } }] } as unknown as SessionForestSnapshot));
  await render({ sessions: [session("w", "working", { parentSessionId: "o" })] });
  expect(host.querySelector("[data-session-id='w'] h2")).not.toBeNull();
  expect(host.querySelector("[data-session-id='w']")?.textContent).toContain("Blocked");
  expect(host.querySelector("[data-attention='true']")).toBeNull();
  expect(host.textContent).not.toContain("needs you");
});

it("announces only the needs-you count to screen readers", async () => {
  await render({ sessions: [session("a", "working"), session("b", "working")] });
  const live = () => [...host.querySelectorAll("[aria-live]")];
  expect(live()).toHaveLength(1);
  expect(live()[0].textContent).toBe("");
  await render({ sessions: [session("a", "working"), session("b", "waiting")] });
  expect(live()).toHaveLength(1);
  expect(live()[0].textContent).toBe("1 needs you");
  expect(live()[0].textContent).not.toContain("working");
});

it("shows what you just sent on the ask line before the stream records it", async () => {
  vi.mocked(bridgeApi.sessionForest).mockImplementation(async id => ({ ...forest(id), entries: [entry(id, 1, "user.message", { text: "first ask" })] } as unknown as SessionForestSnapshot));
  let finish: () => void = () => {};
  vi.mocked(bridgeApi.submitInput).mockImplementation(() => new Promise(resolve => { finish = () => resolve({ disposition: "startedNewTurn", interceptions: [] }); }));
  await render({ sessions: [session("a", "working", { title: "Supervisor" })] });
  const subtitle = () => host.querySelector("[data-session-id='a'] [data-tile-subtitle]")?.textContent;
  expect(subtitle()).toBe("first ask");
  const textarea = host.querySelector<HTMLTextAreaElement>("[data-session-id='a'] textarea")!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(textarea, "now add retries");
    textarea.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => { textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
  expect(subtitle()).toBe("now add retries");
  await act(async () => finish());
  expect(subtitle()).toBe("now add retries");
});

it("highlights one project's tiles from the legend and dims the rest", async () => {
  const spaces = [...workspaces, { id: "k", title: "kairo" }] as Workspace[];
  await render({ sessions: [session("a", "working", { workspaceId: "ws" }), session("b", "working", { workspaceId: "k" }), session("c", "working", { workspaceId: "ws" })], workspaces: spaces });
  const legend = host.querySelector("[aria-label='Projects on the board']")!;
  expect(legend.textContent).toBe("Bridge2kairo1");
  const bridge = [...legend.querySelectorAll("button")].find(button => button.textContent?.startsWith("Bridge"))!;
  await act(async () => bridge.click());
  expect(bridge.getAttribute("aria-pressed")).toBe("true");
  const lit = [...host.querySelectorAll<HTMLElement>("[data-project-highlight='true']")].map(el => el.dataset.sessionId).sort();
  expect(lit).toEqual(["a", "c"]);
  expect(host.querySelector("[data-session-id='b']")?.className).toContain("opacity-40");
  await act(async () => bridge.click());
  expect(host.querySelector("[data-session-id='b']")?.className).not.toContain("opacity-40");
});

it("hides the legend when every tile belongs to one project", async () => {
  await render({ sessions: [session("a", "working", { workspaceId: "ws" }), session("b", "working", { workspaceId: "ws" })] });
  expect(host.querySelector("[aria-label='Projects on the board']")).toBeNull();
});

it("adds a new active chat at the end without moving the others", async () => {
  await render({ sessions: [session("a", "working"), session("b", "working")] });
  await render({ sessions: [session("a", "working"), session("b", "working"), session("c", "working")] });
  expect(savedLayout().order).toEqual(["a", "b", "c"]);
});

it("caps the stage from the tiles control, queues the rest and persists the limit", async () => {
  await render({ sessions: [session("a", "working"), session("b", "working"), session("c", "working")] });
  expect([...host.querySelectorAll("button")].some(button => button.textContent === "Arrange")).toBe(false);
  await act(async () => toolbarButton("2")!.click());
  expect(toolbarButton("2")!.getAttribute("aria-pressed")).toBe("true");
  expect(tiles()).toEqual(["a", "b"]);
  expect(queued()).toEqual(["c"]);
  expect(savedLayout().stageLimit).toBe(2);
  await act(async () => toolbarButton("Auto")!.click());
  expect(tiles()).toEqual(["a", "b", "c"]);
  expect(savedLayout().stageLimit).toBeNull();
});

it("moves a tile earlier or later from its header", async () => {
  await render({ sessions: [session("a", "working"), session("b", "working"), session("c", "working")] });
  expect(host.querySelector("[data-session-id='a'] button[aria-label='Move earlier']")).toBeNull();
  expect(host.querySelector("[data-session-id='c'] button[aria-label='Move later']")).toBeNull();
  await act(async () => host.querySelector<HTMLButtonElement>("[data-session-id='c'] button[aria-label='Move earlier']")!.click());
  expect(tiles()).toEqual(["a", "c", "b"]);
  await act(async () => host.querySelector<HTMLButtonElement>("[data-session-id='a'] button[aria-label='Move later']")!.click());
  expect(tiles()).toEqual(["c", "a", "b"]);
});

describe("measured board", () => {
  let observers: ResizeObserverCallback[] = [];
  beforeEach(() => {
    observers = [];
    vi.stubGlobal("ResizeObserver", class { constructor(private callback: ResizeObserverCallback) {} observe() { observers.push(this.callback); } unobserve() {} disconnect() {} });
  });
  const resize = async (width: number, height: number) => {
    await act(async () => { for (const callback of observers) callback([{ contentRect: { width, height } } as ResizeObserverEntry], {} as ResizeObserver); });
  };
  const seven = () => ["a", "b", "c", "d", "e", "f", "g"].map(id => session(id, "working"));
  const stage = () => host.querySelector<HTMLElement>("[data-stage]")!;

  it("fills the stage from the measured size and queues the rest", async () => {
    await render({ sessions: seven() });
    await resize(2296, 1274);
    expect(tiles()).toEqual(["a", "b", "c", "d", "e", "f", "g"]);
    expect(queued()).toEqual([]);
    expect([stage().dataset.cols, stage().dataset.rows]).toEqual(["4", "2"]);
    await resize(1116, 694);
    expect(tiles()).toEqual(["a", "b", "c", "d"]);
    expect(queued()).toEqual(["e", "f", "g"]);
    expect(host.querySelector("[aria-label='More chats']")?.className).toContain("w-64");
    await resize(760, 694);
    expect(host.querySelector("[aria-label='More chats']")?.className).toContain("h-9");
    await resize(500, 300);
    expect(tiles()).toEqual(["a"]);
    expect(stage().style.gridTemplateColumns).toBe("repeat(1, minmax(0, 1fr))");
    expect(stage().getAttribute("style")).not.toContain("min-width");
    expect(savedLayout().order).toEqual(["a", "b", "c", "d", "e", "f", "g"]);
  });

  it("brings a queued chat to the front of the stage when clicked", async () => {
    await render({ sessions: seven() });
    await resize(1116, 694);
    await act(async () => host.querySelector<HTMLButtonElement>("[data-queue-id='f']")!.click());
    expect(tiles()).toEqual(["f", "a", "b", "c"]);
    expect(queued()).toEqual(["d", "e", "g"]);
  });

  it("drops a queued chat onto a tile's slot and pushes the last tile to the queue", async () => {
    await render({ sessions: seven() });
    await resize(1116, 694);
    const data = transfer(TILE_DRAG, "");
    await dragEvent(host.querySelector("[data-queue-id='g']")!, "dragstart", data);
    expect(data.getData(TILE_DRAG)).toBe("g");
    await dragEvent(host.querySelector("[data-session-id='b']")!, "drop", data, 0, 0);
    expect(tiles()).toEqual(["a", "g", "b", "c"]);
    expect(queued()).toEqual(["d", "e", "f"]);
  });

  it("keeps a chat that needs you on the stage even when it is last", async () => {
    const sessions = seven();
    await render({ sessions });
    await resize(500, 300);
    expect(tiles()).toEqual(["a"]);
    await render({ sessions: [...sessions.slice(0, 6), session("g", "waiting")] });
    expect(tiles()).toEqual(["g"]);
    expect(host.querySelector("[data-queue-id='a']")).not.toBeNull();
    expect(savedLayout().order.at(-1)).toBe("g");
  });

  it("does not move or resize other tiles when one chat finishes", async () => {
    await render({ sessions: seven() });
    await resize(1116, 694);
    const before = [tiles(), queued(), stage().dataset.cols, stage().dataset.rows];
    await render({ sessions: seven().map(item => item.id === "b" ? { ...item, status: "completed" as const } : item) });
    expect([tiles(), queued(), stage().dataset.cols, stage().dataset.rows]).toEqual(before);
  });
});

it("keeps the view title out of the board toolbar", async () => {
  await render({ sessions: [session("a", "working")] });
  expect(host.querySelector("h1")?.className).toContain("sr-only");
  expect(host.querySelector("[role='toolbar']")?.textContent).not.toContain("Mission Control");
});

const newChatButton = () => [...host.querySelectorAll("button")].find(button => button.textContent === "New chat");

it("starts a new chat straight away when there is only one project", async () => {
  const onNewChat = vi.fn();
  await render({ sessions: [session("a", "working", { workspaceId: "ws" })], onNewChat });
  await act(async () => newChatButton()!.click());
  expect(onNewChat).toHaveBeenCalledWith(workspaces[0].id);
});

it("asks which project a new chat is for when there are several", async () => {
  const spaces = [...workspaces, { id: "k", title: "kairo" }] as Workspace[];
  const onNewChat = vi.fn();
  await render({ sessions: [session("a", "working", { workspaceId: "ws" })], workspaces: spaces, onNewChat });
  await act(async () => newChatButton()!.click());
  expect(onNewChat).not.toHaveBeenCalled();
  const kairo = [...document.querySelectorAll<HTMLElement>("[role='menuitem']")].find(item => item.textContent === "kairo")!;
  await act(async () => kairo.click());
  expect(onNewChat).toHaveBeenCalledWith("k");
});

it("shows running workflows as chips inside the board toolbar, not a separate card", async () => {
  const runs = [{ id: "r1", workflowId: "wf", status: "running" }, { id: "r2", workflowId: "wf", status: "succeeded" }] as never[];
  const open = vi.fn();
  await render({ sessions: [session("a", "working")], workflowRuns: runs, workflowNames: new Map([["wf", "Morning brief"]]), onOpenWorkflowRun: open });
  const chips = host.querySelector("[role='toolbar'] [aria-label='Workflows running']")!;
  expect(chips).not.toBeNull();
  expect(chips.className).not.toContain("rounded-xl");
  expect(host.querySelectorAll("[aria-label='Workflows running']")).toHaveLength(1);
  await act(async () => chips.querySelector("button")!.click());
  expect(open).toHaveBeenCalledWith(runs[0]);
  await render({ sessions: [], workflowRuns: runs, workflowNames: new Map(), onOpenWorkflowRun: open });
  expect(host.querySelector("[role='toolbar'] [aria-label='Workflows running']")).not.toBeNull();
  expect(host.querySelector("[aria-label='Tiles on the stage']")).toBeNull();
});
