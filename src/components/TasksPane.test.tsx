// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Session, WorkerRuntimeRecord } from "../types";
import type { QueuedWorkerRequest } from "../protocol/generated/protocol";
import { bridgeApi } from "../api";
import { TasksPane, activeAgentRows, agentLineage, descendantIds, worktreeLabel } from "./TasksPane";
import { agentName } from "./agentIdentity";

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
    // States read in sentence case, as the lineage writes them.
    expect(container.textContent).toContain("Needs you");
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

    await click(byLabel(`View ${agentName("w1")}`));
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
    expect(byLabel(`Stop worker ${agentName("w1")}`)).toBeNull();
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
    expect(byLabel(`Unpin ${agentName("finished")}`)?.getAttribute("aria-pressed")).toBe("true");
    // Nothing left to stop on a finished worker.
    expect(byLabel(`Stop worker ${agentName("finished")}`)).toBeNull();

    await click(byLabel(`Pin ${agentName("live")}`));
    expect(onTogglePin).toHaveBeenCalledWith("live");
  });

  // Contract: testing/fix-agent-starting-state.md, header and rows.
  it("names each agent, with its role and what it is working on", async () => {
    const research = session("w1", { label: "Research · standard", title: "Rank the gateway repos", status: "starting" });
    await mount({ sessions: [chat, research], runtimes: [runtime("w1", { lifecycleState: "starting", taskFamily: "research" })] });
    const row = container.querySelector('[data-agent-row="w1"]')!;
    expect(row.querySelector("b")?.textContent).toBe(agentName("w1"));
    expect(row.textContent).toContain("Research");
    expect(row.textContent).not.toContain("standard");
    expect(row.textContent).toContain("Rank the gateway repos");
    expect(row.textContent).toContain("Starting");

    await click(byLabel(`View ${agentName("w1")}`));
    expect(container.querySelector("b")?.textContent).toBe(agentName("w1"));
    expect(container.textContent).toContain("Rank the gateway repos");
    expect(container.textContent).toContain("Codex · m · research");
  });

  it("stops a live worker through the host", async () => {
    const onStopWorker = vi.fn(async () => undefined);
    await mount({ sessions: [chat, session("w1")], runtimes: [runtime("w1")], onStopWorker });
    await click(byLabel(`Stop worker ${agentName("w1")}`));
    expect(onStopWorker).toHaveBeenCalledWith("w1");
  });

  it("routes the shells row to the terminal pane", async () => {
    const onOpenTerminal = vi.fn();
    await mount({ terminalActivity: { running: 1, attention: false }, onOpenTerminal });
    await click(byLabel("Reveal shells in the terminal pane"));
    expect(onOpenTerminal).toHaveBeenCalledTimes(1);
  });
});

// Contract: testing/feat-t3-inspired-chat-ui.md, E. The pane is the agent
// tree: the orchestrator first, then each worker indented under whoever
// started it, with its harness, worktree, model, state and a clock.
describe("lineage", () => {
  const depths = () => [...container.querySelectorAll("[data-agent-row]")].map(row => [row.getAttribute("data-agent-row"), row.getAttribute("data-depth")]);

  it("orders the tree depth first, pinned siblings ahead, and indents by generation", () => {
    const sessions = [chat, session("w1"), session("w2", { startedAt: "2026-08-25T10:01:00Z" }), session("w1a", { parentSessionId: "w1" }), session("w2a", { parentSessionId: "w2" })];
    const rows = activeAgentRows("chat", sessions, sessions.slice(1).map(item => runtime(item.id, { parentSessionId: item.parentSessionId! })), new Set(["w2"]));
    expect(agentLineage("chat", rows, sessions).map(row => [row.session.id, row.depth])).toEqual([["w2", 1], ["w2a", 2], ["w1", 1], ["w1a", 2]]);
  });

  it("hangs a worker whose parent is not listed from the nearest listed ancestor", () => {
    const sessions = [chat, session("done", { status: "completed" }), session("child", { parentSessionId: "done" }), session("grand", { parentSessionId: "child" })];
    const rows = activeAgentRows("chat", sessions, [runtime("done", { lifecycleState: "completed" }), runtime("child", { parentSessionId: "done" }), runtime("grand", { parentSessionId: "child" })], new Set());
    expect(agentLineage("chat", rows, sessions).map(row => [row.session.id, row.depth])).toEqual([["child", 1], ["grand", 2]]);
  });

  it("never loses a row to a parent loop", () => {
    const sessions = [chat, session("a", { parentSessionId: "b" }), session("b", { parentSessionId: "a" })];
    const rows = [{ runtime: runtime("a"), session: sessions[1]!, pinned: false }, { runtime: runtime("b"), session: sessions[2]!, pinned: false }];
    expect(agentLineage("chat", rows, sessions).map(row => row.session.id).sort()).toEqual(["a", "b"]);
  });

  it("draws the orchestrator on top, waiting on the workers it started", async () => {
    const sessions = [{ ...chat, harness: "codex", model: "gpt-5.6-luna" }, session("w1", { harness: "claude", model: "opus" }), session("w2", { status: "waiting" }), session("w1a", { parentSessionId: "w1" })];
    await mount({
      sessions,
      runtimes: [runtime("w1", { worktreePath: "/repo/.worktrees/deck-sidebar" }), runtime("w2", { lifecycleState: "waiting" }), runtime("w1a", { parentSessionId: "w1" })],
    });
    const lineage = container.querySelector("[data-lineage]")!;
    const orchestrator = lineage.firstElementChild!;
    expect(orchestrator.hasAttribute("data-lineage-orchestrator")).toBe(true);
    expect(orchestrator.textContent).toContain("Orchestrator");
    expect(orchestrator.textContent).toContain("GPT Luna · this chat");
    // Its own workers, not its workers' workers.
    expect(orchestrator.textContent).toContain("Waiting on 2");
    expect(orchestrator.querySelector("svg[data-harness]")?.getAttribute("data-harness")).toBe("codex");
    expect(depths()).toEqual([["w1", "1"], ["w1a", "2"], ["w2", "1"]]);
    expect(container.querySelector('[data-agent-row="w1a"] > div')?.className).toContain("pl-13");
    expect(container.querySelector("h2")?.textContent).toBe("3 workers · 2 running");

    const row = container.querySelector('[data-agent-row="w1"]')!;
    expect(row.querySelector("svg[data-harness]")?.getAttribute("data-harness")).toBe("claude");
    expect(row.textContent).toContain(".worktrees/deck-sidebar · Opus");
    expect(row.querySelector('[data-agent-state="working"]')?.className).toContain("text-info");
    expect(container.querySelector('[data-agent-row="w2"] [data-agent-state="needs-you"]')?.className).toContain("text-warning");
  });

  it("says how many files a pinned finished worker changed", async () => {
    await mount({
      sessions: [chat, session("w1", { status: "completed" })],
      runtimes: [runtime("w1", { lifecycleState: "completed", resultStatus: "reported", lastResult: { status: "completed", filesChanged: ["a.ts", "b.ts", "c.ts"] } })],
      pinned: new Set(["w1"]),
    });
    const state = container.querySelector('[data-agent-row="w1"] [data-agent-state="done"]')!;
    expect(state.textContent).toBe("Done · 3 files");
    expect(state.className).toContain("text-success");
    // Nothing is waited on once it is done.
    expect(container.querySelector("[data-lineage-orchestrator]")?.textContent).not.toContain("Waiting on");
  });

  it("ticks each live worker's clock once a second", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval", "Date"] });
    try {
      vi.setSystemTime(Date.parse("2026-08-25T10:00:48Z"));
      await mount({ sessions: [chat, session("w1")], runtimes: [runtime("w1")] });
      const clock = () => container.querySelector('[data-agent-row="w1"] .tabular-nums')?.textContent;
      expect(clock()).toBe("0:48");
      await act(async () => { vi.advanceTimersByTime(1_000); });
      expect(clock()).toBe("0:49");
    } finally {
      vi.useRealTimers();
    }
  });

  it("teaches the pane's chord and Enter on a worker", async () => {
    await mount({ sessions: [chat, session("w1")], runtimes: [runtime("w1")] });
    const hints = [...container.querySelectorAll("kbd")].map(kbd => kbd.textContent);
    expect(hints).toHaveLength(2);
    expect(hints[0]).toMatch(/6$/);
    expect(hints[1]).toBe("↵");
  });

  it("goes back to the lineage when a focus request names no agent", async () => {
    vi.spyOn(bridgeApi, "sessionForestDigest").mockResolvedValue("d");
    vi.spyOn(bridgeApi, "sessionForest").mockRejectedValue(new Error("not in this test"));
    const sessions = [chat, session("w1")];
    await mount({ sessions, runtimes: [runtime("w1")], focus: { sessionId: "w1", nonce: 1 } });
    expect(rowIds()).toEqual([]);
    await mount({ sessions, runtimes: [runtime("w1")], focus: { sessionId: null, nonce: 2 } });
    expect(rowIds()).toEqual(["w1"]);
  });

  it("shortens a worktree path to what a row has room for", () => {
    expect(worktreeLabel("/Users/me/repo/.worktrees/deck-dock")).toBe(".worktrees/deck-dock");
    expect(worktreeLabel("/tmp/bridge/worker-1w")).toBe("worker-1w");
    expect(worktreeLabel(null)).toBeUndefined();
  });
});

describe("harness subagent sessions", () => {
  it("lists runtime-free children with workers, scopes them and retains pinned outcomes", () => {
    const sessions=[chat,session("child",{kind:"harness_subagent"}),session("other",{kind:"harness_subagent",parentSessionId:"elsewhere"}),session("done",{kind:"harness_subagent",status:"completed"}),session("worker")];
    expect(activeAgentRows("chat",sessions,[runtime("worker")],new Set(["done"])).map(row=>row.session.id)).toEqual(["done","worker","child"]);
  });
  it("opens the child with provenance and sends Stop to the child session", async () => {
    const onStopWorker=vi.fn().mockResolvedValue(undefined);
    const child=session("child",{kind:"harness_subagent",title:"Check authentication",harness:"claude"});
    await mount({sessions:[chat,child],onStopWorker,onSteer:async()=>undefined});
    expect(container.textContent).toContain("subagent");
    expect(container.textContent).not.toContain("STATUS UNAVAILABLE");
    await click(byLabel(`View ${agentName("child")}`));
    expect(container.textContent).toContain("Started by Claude in this chat.");
    expect(container.textContent).toContain("Not a Bridge worker.");
    expect(container.querySelector("textarea")).toBeNull();
    await click(byLabel(`Stop subagent ${agentName("child")}`));
    expect(onStopWorker).toHaveBeenCalledWith("child");
  });
});

describe("lineage with provider subagents", () => {
  it("lists a harness subagent in the tree as a subagent, without a runtime record", async () => {
    const child = session("child", { kind: "harness_subagent", title: "Check authentication", harness: "claude" });
    const onStopWorker = vi.fn().mockResolvedValue(undefined);
    await mount({ sessions: [chat, session("w1"), child], runtimes: [runtime("w1")], onStopWorker });
    const row = container.querySelector('[data-agent-row="child"]');
    expect(row, "the subagent is in the lineage").toBeTruthy();
    expect(row!.getAttribute("data-depth")).toBe("1");
    expect(row!.textContent).toContain("Check authentication");
    expect(container.textContent).toContain("1 worker · 1 subagent");
    await click(byLabel(`Stop subagent ${agentName("child")}`));
    expect(onStopWorker).toHaveBeenCalledWith("child");
  });
});
