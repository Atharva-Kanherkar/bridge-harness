import { describe, expect, it } from "vitest";
import type { ConversationItem } from "../conversation";
import type { Session, WorkerRuntimeRecord } from "../types";
import { agentName } from "./agentIdentity";
import {
  fanOutCountParts,
  fanOutElapsed,
  fanOutWorkers,
  formatWorkerClock,
  groupFanOuts,
  isQuietWorkerRow,
  sentenceCase,
  stateOfTone,
  summarizeFanOut,
} from "./workerFanOut";

// Contract: testing/feat-t3-inspired-chat-ui.md, E (fan-out counts by state,
// +N overflow, the waiting line).

const item = (key: string, overrides: Partial<ConversationItem> = {}): ConversationItem => ({
  key, type: "delegation", eventId: 1, role: "system", text: "", data: {}, sequence: 1, turn: 1, ...overrides,
});
const spawn = (child: string, overrides: Partial<ConversationItem> = {}) =>
  item(`spawn-${child}`, { title: `Delegated to ${child}`, text: `Objective ${child}`, data: { childSessionId: child, harness: "codex" }, ...overrides });
/** A spawn with its result folded on, as `foldWorkerDelegations` leaves it. */
const settled = (child: string, data: Record<string, unknown> = {}) =>
  spawn(child, { data: { childSessionId: child, harness: "codex", delivered: true, status: "completed", ...data } });
const prose = (key: string, turn = 1) => item(key, { type: "message", role: "assistant", text: "Delegating.", turn });
const blocked = (child: string, childBlocked = true) => item(`blocked-${child}`, { data: { childSessionId: child, childBlocked } });

const session = (id: string, overrides: Partial<Session> = {}): Session => ({
  id, workspaceId: "w", harness: "claude", label: "Implementation · strong", status: "working", startedAt: "2026-11-21T10:00:00Z",
  endedAt: null, contextPercent: null, usagePercent: null, metricSource: "reported", model: "opus", restorationMode: "hot",
  continuationFidelity: "native", kind: "worker", parentSessionId: "chat", ...overrides,
});
const runtime = (sessionId: string, overrides: Partial<WorkerRuntimeRecord> = {}): WorkerRuntimeRecord => ({
  sessionId, parentSessionId: "chat", lifecycleState: "working", taskFamily: "implementation", compatibilityKey: "k",
  resultStatus: "pending", retryCount: 0, lastResult: null, updatedAt: "2026-11-21T10:00:00Z", ...overrides,
} as WorkerRuntimeRecord);

describe("grouping a delegation's worker rows", () => {
  it("puts a turn's adjacent worker rows in one fan-out, drawn at the first", () => {
    const { byHead, absorbed } = groupFanOuts([prose("p"), spawn("a"), spawn("b"), spawn("c")]);
    expect([...byHead.keys()]).toEqual(["spawn-a"]);
    expect(byHead.get("spawn-a")!.members.map(member => member.key)).toEqual(["spawn-a", "spawn-b", "spawn-c"]);
    // The card holds the other rows, so the transcript skips their slots.
    expect([...absorbed]).toEqual(["spawn-b", "spawn-c"]);
  });

  it("lets other delegation rows sit between members without splitting the fan-out", () => {
    const { byHead } = groupFanOuts([spawn("a"), blocked("a"), spawn("b")]);
    expect(byHead.get("spawn-a")!.members.map(member => member.key)).toEqual(["spawn-a", "spawn-b"]);
  });

  it("never pulls a row up past prose or into another turn", () => {
    const { byHead } = groupFanOuts([spawn("a"), prose("p"), spawn("b"), spawn("c", { turn: 2 })]);
    expect([...byHead.keys()]).toEqual(["spawn-a", "spawn-b", "spawn-c"]);
    expect([...byHead.values()].map(fanOut => fanOut.members.length)).toEqual([1, 1, 1]);
  });

  it("leaves a failed worker's alert row in place and only counts it", () => {
    const failed = settled("b", { status: "failed", failureCause: "the worker stopped responding" });
    const { byHead, absorbed } = groupFanOuts([spawn("a"), failed, spawn("c")]);
    expect(byHead.get("spawn-a")!.members).toHaveLength(3);
    expect(isQuietWorkerRow(failed)).toBe(false);
    expect([...absorbed]).toEqual(["spawn-c"]);
  });

  it("ignores delegation rows that never named a worker", () => {
    const rejected = item("rejected", { data: { willRetry: false, reason: "policy" } });
    expect(groupFanOuts([rejected]).byHead.size).toBe(0);
  });
});

describe("a fan-out's workers and their states", () => {
  const states = (items: ConversationItem[], lookup = {}) => {
    const fanOuts = groupFanOuts(items);
    const [fanOut] = fanOuts.byHead.values();
    return fanOutWorkers(fanOut!, fanOuts.blocked, lookup).map(worker => worker.state);
  };

  it("reads the transcript alone when Bridge has no session for a worker", () => {
    expect(states([
      spawn("a"),
      settled("b"),
      settled("c", { status: "failed", failureCause: "boom" }),
      settled("d", { status: "cancelled" }),
      spawn("e"),
      blocked("e"),
    ])).toEqual(["working", "done", "failed", "stopped", "needs-you"]);
    // A resolved approval sends the worker back to work.
    expect(states([spawn("a"), blocked("a", false)])).toEqual(["working"]);
  });

  it("takes the live state from the worker's session and runtime while it has no result", () => {
    const lookup = {
      sessions: [session("a"), session("b", { status: "waiting" }), session("c", { status: "failed" })],
      runtimes: [runtime("a"), runtime("b", { lifecycleState: "waiting" }), runtime("c", { lifecycleState: "failed" })],
    };
    expect(states([spawn("a"), spawn("b"), spawn("c")], lookup)).toEqual(["working", "needs-you", "failed"]);
  });

  it("keeps a folded result final even when the worker was reused for later work", () => {
    const lookup = { sessions: [session("a")], runtimes: [runtime("a", { lifecycleState: "working" })] };
    expect(states([settled("a")], lookup)).toEqual(["done"]);
  });

  it("maps every tone onto the five states", () => {
    expect(stateOfTone("working")).toBe("working");
    expect(stateOfTone("waiting")).toBe("needs-you");
    expect(stateOfTone("attention")).toBe("needs-you");
    expect(stateOfTone("done")).toBe("done");
    expect(stateOfTone("warm")).toBe("done");
    expect(stateOfTone("failed")).toBe("failed");
    expect(stateOfTone("stalled")).toBe("failed");
    // A cancellation is a decision, not a fault.
    expect(stateOfTone("idle")).toBe("stopped");
  });

  it("names a worker by its topic when Bridge cut one, else by its own name", () => {
    const fanOuts = groupFanOuts([spawn("a"), spawn("b")]);
    const workers = fanOutWorkers(fanOuts.byHead.get("spawn-a")!, fanOuts.blocked, {
      sessions: [session("a", { title: "Sidebar running section" }), session("b", { title: null })],
      runtimes: [runtime("a"), runtime("b")],
    });
    expect(workers.map(worker => worker.name)).toEqual(["Sidebar running section", agentName("b")]);
    // The session's harness wins; the spawn's own word is the fallback.
    expect(workers.map(worker => worker.harness)).toEqual(["claude", "claude"]);
    expect(fanOutWorkers(fanOuts.byHead.get("spawn-a")!, fanOuts.blocked)[0]!.harness).toBe("codex");
  });
});

describe("the summary a card draws", () => {
  const worker = (child: string, state: "working" | "needs-you" | "done" | "failed" | "stopped", startedAt?: number, endedAt?: number) =>
    ({ childSessionId: child, item: spawn(child), name: child, harness: "codex", state, startedAt, endedAt });

  it("counts by state in a fixed order and names only the states present", () => {
    const summary = summarizeFanOut([worker("a", "done"), worker("b", "working"), worker("c", "needs-you"), worker("d", "failed")]);
    expect(summary.total).toBe(4);
    expect(fanOutCountParts(summary.counts).map(part => part.text)).toEqual(["1 working", "1 needs you", "1 done", "1 failed"]);
    expect(fanOutCountParts(summarizeFanOut([worker("a", "done"), worker("b", "done")]).counts).map(part => part.text)).toEqual(["2 done"]);
  });

  it("stacks three marks, then +N", () => {
    expect(summarizeFanOut([worker("a", "working"), worker("b", "working")])).toMatchObject({ overflow: 0 });
    const five = summarizeFanOut(["a", "b", "c", "d", "e"].map(id => worker(id, "working")));
    expect(five.marks.map(mark => mark.childSessionId)).toEqual(["a", "b", "c"]);
    expect(five.overflow).toBe(2);
  });

  it("waits on the workers still working or asking, in order", () => {
    const summary = summarizeFanOut([worker("a", "done"), worker("b", "needs-you"), worker("c", "working"), worker("d", "failed")]);
    expect(summary.waiting.map(item => item.name)).toEqual(["b", "c"]);
    expect(summary.live).toBe(true);
    expect(summarizeFanOut([worker("a", "done"), worker("b", "stopped")]).waiting).toEqual([]);
  });

  it("runs the clock from the first start while live and holds it once every worker settled", () => {
    const live = summarizeFanOut([worker("a", "working", 1_000), worker("b", "done", 5_000, 20_000)]);
    expect(fanOutElapsed(live, 73_000)).toBe(72_000);
    const settledSummary = summarizeFanOut([worker("a", "done", 1_000, 50_000), worker("b", "failed", 5_000, 20_000)]);
    expect(fanOutElapsed(settledSummary, 999_000)).toBe(49_000);
    // Settled with no end Bridge knows of: no clock rather than an invented one.
    expect(fanOutElapsed(summarizeFanOut([worker("a", "done", 1_000)]), 999_000)).toBeUndefined();
    expect(fanOutElapsed(summarizeFanOut([worker("a", "working")]), 999_000)).toBeUndefined();
  });

  it("writes the clock as minutes and seconds", () => {
    expect(formatWorkerClock(0)).toBe("0:00");
    expect(formatWorkerClock(48_900)).toBe("0:48");
    expect(formatWorkerClock(72_000)).toBe("1:12");
    expect(formatWorkerClock(3_725_000)).toBe("1:02:05");
    expect(formatWorkerClock(-5)).toBe("0:00");
  });

  it("writes a worker label in sentence case", () => {
    expect(sentenceCase("NEEDS YOU")).toBe("Needs you");
    expect(sentenceCase("WORKING")).toBe("Working");
  });
});
