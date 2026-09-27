// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Session, WorkerRuntimeRecord } from "../types";
import type { QueuedWorkerRequest } from "../protocol/generated/protocol";
import { bridgeApi } from "../api";
import { TasksPane, activeAgentRows, descendantIds } from "./TasksPane";

// The Agents pane lists the agents this chat has running now, and pinned ones,
// and opens any of them to its chat.

const session = (id: string, overrides: Partial<Session> = {}): Session => ({
  id, workspaceId: "w", harness: "codex", label: `Worker ${id}`, status: "working", startedAt: "2026-08-25T10:00:00Z",
  endedAt: null, contextPercent: null, usagePercent: null, metricSource: "reported", model: "m", restorationMode: "hot",
  continuationFidelity: "native", kind: "worker", parentSessionId: "chat", ...overrides,
});

const runtime = (sessionId: string, overrides: Partial<WorkerRuntimeRecord> = {}): WorkerRuntimeRecord => ({
  sessionId, parentSessionId: "chat", lifecycleState: "working", taskFamily: "implementation", compatibilityKey: "k",
  resultStatus: "pending", retryCount: 0, lastResult: null, lastActivityAt: "2026-08-25T10:00:00Z", ...overrides,
} as WorkerRuntimeRecord);

const queued = (id: string, overrides: Partial<QueuedWorkerRequest> = {}): QueuedWorkerRequest => ({
  id, parentSessionId: "chat", workspaceId: "w", turnId: "t", sequence: 1, attemptCount: 0,
  request: { role: "implementation", objective: `Objective ${id}`, reason: "owned_path_conflict" },
  actualModel: "gpt-terra", queueStatus: "queued", dispatchedSessionId: null, lastError: null,
  blockedAt: null, claimedAt: null, expiresAt: "later", createdAt: "now", updatedAt: "now", ...overrides,
} as QueuedWorkerRequest);

const chat = session("chat", { kind: "orchestrator", parentSessionId: null, label: "Orchestrator" });

let container: HTMLDivElement;
let root: Root;

async function mount(props: Partial<Parameters<typeof TasksPane>[0]> = {}) {
  await act(async () => {
    root.render(<TasksPane
      chatSessionId="chat"
      sessions={[chat]}
      pinned={new Set()}
      onTogglePin={() => undefined}
      liveEvents={[]}
      onOpenSession={() => undefined}
      {...props}
    />);
  });
}

const click = async (element: Element | null | undefined) => {
  expect(element, "expected the element to be in the tree").toBeTruthy();
  await act(async () => { element!.dispatchEvent(new MouseEvent("click", { bubbles: true })); });
};
const byLabel = (label: string) => container.querySelector(`button[aria-label="${label}"]`);
const rowIds = () => [...container.querySelectorAll("[data-agent-row]")].map(row => row.getAttribute("data-agent-row"));

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

describe("which agents are listed", () => {
  it("lists only this chat's agents that are running or waiting on you", async () => {
    const sessions = [
      chat,
      session("running"),
      session("asking", { status: "waiting" }),
      session("done", { status: "completed" }),
      session("failed", { status: "failed" }),
      session("elsewhere", { parentSessionId: "another-chat" }),
    ];
    const runtimes = [
      runtime("running"),
      runtime("asking", { lifecycleState: "waiting", waitingReason: "approval_required" }),
      runtime("done", { lifecycleState: "completed", resultStatus: "reported", lastResult: { status: "completed", summary: "ok" } }),
      runtime("failed", { lifecycleState: "failed" }),
      // The snapshot carries the whole workspace; this one belongs to another chat.
      runtime("elsewhere", { parentSessionId: "another-chat" }),
    ];
    await mount({ sessions, runtimes });
    expect(rowIds()).toEqual(["running", "asking"]);
    expect(container.textContent).toContain("NEEDS YOU");
    expect(container.textContent).toContain("waiting: approval required");
  });

  it("counts a worker's own workers as the chat's", () => {
    const sessions = [chat, session("w1"), session("w1a", { parentSessionId: "w1" }), session("other", { parentSessionId: "x" })];
    expect([...descendantIds("chat", sessions)].sort()).toEqual(["w1", "w1a"]);
    expect(activeAgentRows("chat", sessions, [runtime("w1"), runtime("w1a", { parentSessionId: "w1" })], new Set()).map(row => row.session.id)).toEqual(["w1", "w1a"]);
  });

  it("shows only requests genuinely queued for this chat", async () => {
    await mount({
      queue: [
        queued("q-mine"),
        queued("q-dispatched", { queueStatus: "dispatched" }),
        queued("q-rejected", { queueStatus: "rejected" }),
        queued("q-other", { parentSessionId: "another-chat" }),
      ],
    });
    expect(container.textContent).toContain("Objective q-mine");
    expect(container.textContent).not.toContain("Objective q-dispatched");
    expect(container.textContent).not.toContain("Objective q-rejected");
    expect(container.textContent).not.toContain("Objective q-other");
    expect(container.textContent).toContain("1 queued");
  });

  it("says so when nothing is running", async () => {
    await mount({ runtimes: [runtime("done", { lifecycleState: "completed" })], sessions: [chat, session("done", { status: "completed" })] });
    expect(container.textContent).toContain("No agents running");
    expect(rowIds()).toEqual([]);
  });
});

describe("a row", () => {
  // The chat renders through AgentConversation, which polls the worker's forest.
  beforeEach(() => {
    vi.spyOn(bridgeApi, "sessionForestDigest").mockResolvedValue("d");
    vi.spyOn(bridgeApi, "sessionForest").mockRejectedValue(new Error("not in this test"));
  });
  const chatRegion = (name: string) => container.querySelector(`[role="region"][aria-label="Agent ${name}"]`);

  it("opens the agent's chat in the pane and goes back to the list", async () => {
    await mount({ sessions: [chat, session("w1")], runtimes: [runtime("w1", { progressSummary: "editing src/auth/store.rs" })] });
    expect(container.textContent).toContain("editing src/auth/store.rs");
    expect(chatRegion("Worker w1")).toBeNull();

    await click(byLabel("View Worker w1"));
    expect(chatRegion("Worker w1")).not.toBeNull();
    // The list gives way to the chat, and it is part of the pane, not an overlay.
    expect(rowIds()).toEqual([]);
    expect(container.querySelector('[role="dialog"]')).toBeNull();

    await click(byLabel("All agents"));
    expect(chatRegion("Worker w1")).toBeNull();
    expect(rowIds()).toEqual(["w1"]);
  });

  it("opens the agent a focus request names, again after Back when the nonce moves", async () => {
    const sessions = [chat, session("w1"), session("w2")];
    const runtimes = [runtime("w1"), runtime("w2")];
    await mount({ sessions, runtimes, focus: { sessionId: "w2", nonce: 1 } });
    expect(chatRegion("Worker w2")).not.toBeNull();

    await click(byLabel("All agents"));
    expect(chatRegion("Worker w2")).toBeNull();
    await mount({ sessions, runtimes, focus: { sessionId: "w2", nonce: 2 } });
    expect(chatRegion("Worker w2")).not.toBeNull();
  });

  it("ignores a focus request for another chat's agent", async () => {
    await mount({ sessions: [chat, session("w1"), session("other", { parentSessionId: "x" })], runtimes: [runtime("w1")], focus: { sessionId: "other", nonce: 1 } });
    expect(chatRegion("Worker other")).toBeNull();
    expect(rowIds()).toEqual(["w1"]);
  });

  it("keeps an open agent open after it finishes, even unpinned", async () => {
    await mount({ sessions: [chat, session("w1")], runtimes: [runtime("w1")], focus: { sessionId: "w1", nonce: 1 } });
    expect(chatRegion("Worker w1")).not.toBeNull();
    await mount({ sessions: [chat, session("w1", { status: "completed" })], runtimes: [runtime("w1", { lifecycleState: "completed", resultStatus: "reported" })], focus: { sessionId: "w1", nonce: 1 } });
    expect(chatRegion("Worker w1")).not.toBeNull();
    // Nothing left to stop.
    expect(byLabel("Stop worker Worker w1")).toBeNull();
    await click(byLabel("All agents"));
    expect(container.textContent).toContain("No agents running");
  });

  it("hands Open session to the host", async () => {
    const onOpenSession = vi.fn();
    await mount({ sessions: [chat, session("w1")], runtimes: [runtime("w1")], focus: { sessionId: "w1", nonce: 1 }, onOpenSession });
    await click(byLabel("Open session"));
    expect(onOpenSession).toHaveBeenCalledWith("w1");
  });

  it("keeps a pinned worker listed after it finishes, first in the list", async () => {
    const onTogglePin = vi.fn();
    await mount({
      sessions: [chat, session("live"), session("finished", { status: "completed" })],
      runtimes: [runtime("live"), runtime("finished", { lifecycleState: "completed" })],
      pinned: new Set(["finished"]),
      onTogglePin,
    });
    expect(rowIds()).toEqual(["finished", "live"]);
    expect(byLabel("Unpin Worker finished")?.getAttribute("aria-pressed")).toBe("true");
    // Nothing left to stop on a finished worker.
    expect(byLabel("Stop worker Worker finished")).toBeNull();

    await click(byLabel("Pin Worker live"));
    expect(onTogglePin).toHaveBeenCalledWith("live");
  });

  it("stops a live worker through the host", async () => {
    const onStopWorker = vi.fn(async () => undefined);
    await mount({ sessions: [chat, session("w1")], runtimes: [runtime("w1")], onStopWorker });
    await click(byLabel("Stop worker Worker w1"));
    expect(onStopWorker).toHaveBeenCalledWith("w1");
  });

  it("routes the shells row to the terminal pane", async () => {
    const onOpenTerminal = vi.fn();
    await mount({ terminalActivity: { running: 1, attention: false }, onOpenTerminal });
    await click(byLabel("Reveal shells in the terminal pane"));
    expect(onOpenTerminal).toHaveBeenCalledTimes(1);
  });
});
