// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AgentConversation } from "./AgentConversation";
import { asWireKind } from "../transcript/wire";
import type { AgentEvent, Session, WorkerRuntimeRecord } from "../types";

// Contract: testing/feat-t3-inspired-chat-ui.md, E. One fan-out card per
// delegation in the orchestrator's transcript: stacked marks, counts by state,
// a clock, a way into the Agents pane, and the waiting line with Stop.

const START = Date.parse("2026-11-21T10:00:00Z");

const chat: Session = {
  id: "chat", workspaceId: "w", harness: "codex", label: "Orchestrator", status: "working", startedAt: "2026-11-21T09:59:00Z",
  endedAt: null, contextPercent: null, usagePercent: null, metricSource: "reported", model: "gpt-5.6-luna",
  restorationMode: "fresh", continuationFidelity: "native", kind: "orchestrator",
} as Session;

const worker = (id: string, overrides: Partial<Session> = {}): Session => ({
  ...chat, id, harness: "claude", label: "Implementation · strong", kind: "worker", parentSessionId: "chat",
  startedAt: "2026-11-21T10:00:00Z", model: "opus", ...overrides,
} as Session);

const runtime = (sessionId: string, overrides: Partial<WorkerRuntimeRecord> = {}): WorkerRuntimeRecord => ({
  sessionId, parentSessionId: "chat", lifecycleState: "working", taskFamily: "implementation", compatibilityKey: "k",
  resultStatus: "pending", retryCount: 0, lastResult: null, updatedAt: "2026-11-21T10:00:00Z", ...overrides,
} as WorkerRuntimeRecord);

let nextId = 1;
const event = (kind: string, overrides: Partial<AgentEvent> = {}): AgentEvent => {
  const id = nextId++;
  return {
    id, sessionId: "chat", sequence: id, protocolVersion: 1, kind: asWireKind(kind), itemId: `i-${id}`, role: null,
    status: "completed", title: null, text: null, data: {}, providerMeta: {}, createdAt: "2026-11-21T10:00:00Z", ...overrides,
  } as AgentEvent;
};
const user = (text: string) => event("message.completed", { role: "user", text });
const spawn = (child: string, harness = "claude") => event("delegation.spawned", {
  itemId: `spawn-${child}`, role: "system", status: "working", title: `Delegated to ${child}`, text: `Objective for ${child}`,
  data: { childSessionId: child, harness, modelLabel: "Opus" },
});
const result = (child: string, data: Record<string, unknown> = {}) => event("delegation.result", {
  itemId: `result-${child}`, role: "system", status: "completed", title: "Worker result", text: "Done.",
  data: { childSessionId: child, delivered: true, status: "completed", ...data },
});

let host: HTMLDivElement;
let root: Root;

async function mount(props: Partial<Parameters<typeof AgentConversation>[0]>) {
  await act(async () => root.render(<AgentConversation session={chat} forestEntries={[]} onResolve={() => undefined} {...props}/>));
}
const cards = () => [...host.querySelectorAll("[data-fan-out]")];
const button = (scope: ParentNode, text: string) => [...scope.querySelectorAll("button")].find(item => item.textContent?.trim() === text || item.getAttribute("aria-label") === text);
const click = async (element: Element | undefined) => {
  expect(element, "expected the button to be in the tree").toBeTruthy();
  await act(async () => { element!.dispatchEvent(new MouseEvent("click", { bubbles: true })); });
};

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  nextId = 1;
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  vi.useRealTimers();
});

describe("the fan-out card", () => {
  it("draws one card per delegation, holding every worker's row", async () => {
    await mount({ events: [user("Split it."), spawn("a"), spawn("b", "codex"), spawn("c"), result("c")], onOpenAgent: () => undefined });
    expect(cards()).toHaveLength(1);
    const card = cards()[0]!;
    expect(card.getAttribute("aria-label")).toBe("3 workers");
    expect(card.textContent).toContain("3 workers");
    expect(card.querySelector('[data-fan-out-count="working"]')?.textContent).toBe("2 working");
    expect(card.querySelector('[data-fan-out-count="done"]')?.textContent).toBe("1 done");
    // Status ink, not chrome: live is info, done is success.
    expect(card.querySelector('[data-fan-out-count="working"]')?.className).toContain("text-info");
    expect(card.querySelector('[data-fan-out-count="done"]')?.className).toContain("text-success");
    // One mark per worker, in order.
    expect([...card.querySelectorAll("svg[data-harness]")].map(mark => mark.getAttribute("data-harness"))).toEqual(["claude", "codex", "claude"]);
    // Grouping keeps every row and its way into the worker's chat.
    expect(card.textContent).toContain("Delegated · a");
    expect(card.textContent).toContain("Delegated · b");
    expect(card.textContent).toContain("Worker finished");
    expect([...card.querySelectorAll("button")].filter(item => item.textContent?.includes("Show in Agents"))).toHaveLength(3);
    // Nothing is drawn twice outside the card.
    expect(host.textContent!.split("Delegated · b").length - 1).toBe(1);
  });

  it("stacks three marks, then +N", async () => {
    await mount({ events: [spawn("a"), spawn("b"), spawn("c"), spawn("d"), spawn("e")] });
    const card = cards()[0]!;
    expect(card.querySelectorAll("svg[data-harness]")).toHaveLength(3);
    expect(card.querySelector("[data-fan-out-overflow]")?.textContent).toBe("+2");
  });

  it("starts a new card for a later delegation", async () => {
    await mount({ events: [user("First."), spawn("a"), result("a"), user("Second."), spawn("b")] });
    expect(cards().map(card => card.getAttribute("aria-label"))).toEqual(["1 worker", "1 worker"]);
  });

  it("names the workers it is waiting on, by topic, and stops them through the host", async () => {
    const onStopWorker = vi.fn(async () => undefined);
    await mount({
      events: [spawn("a"), spawn("b"), spawn("c")],
      workerSessions: [chat, worker("a", { title: "Sidebar running section" }), worker("b", { title: "Dock work pane", status: "waiting" }), worker("c", { title: "Composer footer", status: "completed" })],
      workerRuntimes: [runtime("a"), runtime("b", { lifecycleState: "waiting", waitingReason: "approval_required" }), runtime("c", { lifecycleState: "completed", resultStatus: "reported", lastResult: { status: "completed" } })],
      onStopWorker,
    });
    const card = cards()[0]!;
    expect(card.textContent).toContain("1 working · 1 needs you · 1 done");
    expect(card.querySelector('[data-fan-out-count="needs-you"]')?.className).toContain("text-warning");
    const waiting = card.querySelector("[data-fan-out-waiting]")!;
    expect(waiting.textContent).toContain("Waiting on Sidebar running section, Dock work pane");
    expect(waiting.textContent).not.toContain("Composer footer");

    await click(button(waiting, "Stop"));
    // Only the workers still being waited on, never the finished one.
    expect(onStopWorker.mock.calls.map(call => (call as unknown[])[0])).toEqual(["a", "b"]);
  });

  it("says what went wrong when a stop fails", async () => {
    const onStopWorker = vi.fn(async () => { throw new Error("worker is gone"); });
    await mount({ events: [spawn("a")], onStopWorker });
    await click(button(cards()[0]!, "Stop"));
    expect(cards()[0]!.querySelector('[role="alert"]')?.textContent).toBe("Worker is gone.");
  });

  it("opens the Agents pane from its chevron", async () => {
    const onOpenAgents = vi.fn();
    await mount({ events: [spawn("a")], onOpenAgents });
    await click(button(cards()[0]!, "Open the Agents pane"));
    expect(onOpenAgents).toHaveBeenCalledTimes(1);
  });

  it("offers Stop and the opener only when the host can do them", async () => {
    await mount({ events: [spawn("a")] });
    const card = cards()[0]!;
    expect(card.querySelector("[data-fan-out-waiting]")?.textContent).toContain("Waiting on");
    expect(button(card, "Stop")).toBeUndefined();
    expect(button(card, "Open the Agents pane")).toBeUndefined();
  });

  it("drops the waiting line and Stop on a read-only transcript", async () => {
    await mount({ events: [spawn("a")], readOnly: true, onStopWorker: async () => undefined });
    const card = cards()[0]!;
    expect(card.querySelector("[data-fan-out-waiting]")).toBeNull();
    expect(button(card, "Stop")).toBeUndefined();
  });

  it("drops the waiting line once every worker has settled", async () => {
    await mount({ events: [spawn("a"), spawn("b"), result("a"), result("b")], onStopWorker: async () => undefined });
    const card = cards()[0]!;
    expect(card.textContent).toContain("2 done");
    expect(card.querySelector("[data-fan-out-waiting]")).toBeNull();
  });

  it("keeps a failed worker's alert and Retry outside the card, and counts it", async () => {
    const onRetryWorker = vi.fn(async () => undefined);
    await mount({
      events: [spawn("a"), spawn("b"), result("b", { status: "failed", failureCause: "the worker stopped responding", failureClass: "stalled", canRetry: true })],
      onRetryWorker,
    });
    const card = cards()[0]!;
    expect(card.querySelector('[data-fan-out-count="failed"]')?.textContent).toBe("1 failed");
    expect(card.querySelector('[data-fan-out-count="failed"]')?.className).toContain("text-destructive");
    expect(card.textContent).not.toContain("the worker stopped responding");
    const alert = [...host.querySelectorAll('[role="alert"]')].find(node => node.textContent?.includes("the worker stopped responding"));
    expect(alert).toBeTruthy();
    expect(card.contains(alert!)).toBe(false);
    await click(button(alert!, "Retry this task"));
    expect(onRetryWorker).toHaveBeenCalledWith("b");
  });

  it("ticks its clock once a second while a worker is live", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval", "Date"] });
    vi.setSystemTime(START + 72_000);
    await mount({ events: [spawn("a")], workerSessions: [chat, worker("a")], workerRuntimes: [runtime("a")] });
    const clock = () => cards()[0]!.querySelector("[data-fan-out-clock]")?.textContent;
    expect(clock()).toBe("1:12");
    await act(async () => { vi.advanceTimersByTime(1_000); });
    expect(clock()).toBe("1:13");
  });

  it("holds its clock at the last worker's end once all have settled", async () => {
    await mount({
      events: [spawn("a"), result("a")],
      workerSessions: [chat, worker("a", { status: "completed", endedAt: "2026-11-21T10:00:36Z" })],
      workerRuntimes: [runtime("a", { lifecycleState: "completed", resultStatus: "reported" })],
    });
    expect(cards()[0]!.querySelector("[data-fan-out-clock]")?.textContent).toBe("0:36");
  });
});

describe("the fan-out card beside a turn's end", () => {
  it("keeps the turn's end when the turn ends on a worker row the card absorbed", async () => {
    const settled = { ...chat, status: "ready" } as Session;
    const edit = event("file_change.completed", { title: "policy.rs", data: { path: "src/policy.rs", additions: 2, deletions: 1 } });
    await mount({
      session: settled,
      working: false,
      events: [user("Split it"), edit, spawn("a"), spawn("b"), result("a"), result("b")],
    });
    expect(cards()).toHaveLength(1);
    expect(host.querySelectorAll("[data-turn-changes]"), "exactly one changes card").toHaveLength(1);
    // The last worker row is folded into the card, but the turn it closes
    // still draws its end (the turn line) after the card, not nowhere.
    const ends = [...host.querySelectorAll("[data-turn-end]")];
    const card = cards()[0]!;
    expect(ends.some(end => card.compareDocumentPosition(end) & Node.DOCUMENT_POSITION_FOLLOWING), "a turn end after the fan-out card").toBe(true);
  });
});

describe("the fan-out card and provider subagents", () => {
  it("counts and groups Bridge workers only, leaving a harness subagent its own row", async () => {
    const subagentStarted = event("subagent.started", {
      status: "working",
      data: { childSessionId: "provider-child", subagent: { sessionId: "p-1", agent: "Explore", title: "Check authentication" } },
    });
    await mount({ events: [user("Split it"), spawn("a"), spawn("b"), subagentStarted] });
    expect(cards()).toHaveLength(1);
    const card = cards()[0]!;
    expect(card.textContent).toContain("2 workers");
    expect(card.textContent).not.toContain("Check authentication");
    // The provider's child keeps its own quiet row outside the card.
    const outside = [...host.querySelectorAll("button")].filter(button => !card.contains(button) && button.textContent?.includes("Subagent"));
    expect(outside.length).toBeGreaterThan(0);
  });
});
