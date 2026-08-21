import { describe, expect, it } from "vitest";
import { WORKER_PANEL_FEED_LINES, workerFeedLines, workerPanelModel } from "./workerPanel";
import type { AgentEvent, Session, WorkerRuntimeRecord } from "../types";

const session = (id: string, overrides: Partial<Session> = {}): Session => ({
  id, workspaceId: "w", harness: "claude", label: "Implementation · strong", title: null, status: "working",
  model: null, effort: null, requestedTier: null, parentSessionId: "parent", providerSessionId: null,
  activeTurnId: "turn-1", startedAt: "2026-08-21T10:00:00Z", endedAt: null, depth: 1, kind: "workspace",
  cwd: null, contextPercent: null, usagePercent: null, metricSource: "reported",
  restorationMode: "fresh", continuationFidelity: "native", ...overrides,
});

const runtime = (overrides: Partial<WorkerRuntimeRecord> = {}): WorkerRuntimeRecord => ({
  sessionId: "child", parentSessionId: "parent", lifecycleState: "working", taskFamily: "implementation",
  compatibilityKey: "key", resultStatus: "pending", retryCount: 0, warmUntil: null, worktreePath: null,
  worktreeBranch: null, lastResult: null, lastActivityAt: null, waitingSince: null, waitingReason: null,
  progressSummary: null, updatedAt: "2026-08-21T10:01:00Z", ...overrides,
});

const event = (id: number, overrides: Partial<AgentEvent> = {}): AgentEvent => ({
  id, sessionId: "child", sequence: id, protocolVersion: 1, kind: "tool.started", itemId: null, role: null,
  status: null, title: null, text: null, data: {}, providerMeta: {}, createdAt: "now", ...overrides,
});

describe("worker panel projection", () => {
  it("projects the live facts a panel needs", () => {
    const model = workerPanelModel(
      "child",
      [session("child")],
      [runtime({ retryCount: 2, progressSummary: "editing src/auth/store.rs", waitingReason: "approval_requested", waitingSince: "2026-08-21T10:02:00Z" })],
      [event(1, { title: "read store.rs" })],
    );
    expect(model).not.toBeNull();
    expect(model?.status.label).toBe("WORKING");
    expect(model?.retryCount).toBe(2);
    expect(model?.progressSummary).toBe("editing src/auth/store.rs");
    expect(model?.waitingReason).toBe("approval_requested");
    expect(model?.waitingSince).toBe("2026-08-21T10:02:00Z");
    expect(model?.startedAt).toBe("2026-08-21T10:00:00Z");
    expect(model?.taskFamily).toBe("implementation");
    expect(model?.reported).toBe(false);
    expect(model?.result).toBeUndefined();
  });

  it("returns null when the child session is unknown", () => {
    // A worker whose session row has not arrived yet — the spawn event beat the
    // state poll. The card falls back rather than rendering a half-empty panel.
    expect(workerPanelModel("ghost", [session("child")], [runtime()], [])).toBeNull();
  });

  it("works without a runtime record", () => {
    const model = workerPanelModel("child", [session("child")], [], []);
    expect(model?.retryCount).toBe(0);
    expect(model?.status.label).toBe("WORKING");
  });

  it("surfaces the reported result once the envelope is in", () => {
    const model = workerPanelModel("child", [session("child", { status: "stopped" })], [runtime({
      resultStatus: "reported",
      lifecycleState: "completed",
      lastResult: {
        status: "completed",
        summary: "rotation added",
        filesChanged: ["src/auth/store.rs", "src/auth/mod.rs"],
        tests: [{ command: "cargo test auth", status: "passed" }, { status: "passed" }],
      },
    })], []);
    expect(model?.reported).toBe(true);
    expect(model?.result?.status).toBe("completed");
    expect(model?.result?.filesChanged).toEqual(["src/auth/store.rs", "src/auth/mod.rs"]);
    // A test entry with no command is unusable in a list, so it is dropped
    // rather than rendered as a blank row.
    expect(model?.result?.tests).toEqual([{ command: "cargo test auth", status: "passed" }]);
  });

  it("does not trust a result that has not been reported yet", () => {
    // A reused warm worker keeps its previous envelope while the new run is
    // pending. Reading it would show last run's outcome as this one's.
    const model = workerPanelModel("child", [session("child")], [runtime({
      resultStatus: "pending",
      lastResult: { status: "completed", summary: "the previous run" },
    })], []);
    expect(model?.reported).toBe(false);
    expect(model?.result).toBeUndefined();
  });
});

describe("worker feed lines", () => {
  it("caps the mini-feed and keeps the newest lines", () => {
    const events = Array.from({ length: 50 }, (_, index) => event(index + 1, { title: `step ${index + 1}` }));
    const lines = workerFeedLines(events, "child");
    expect(lines).toHaveLength(WORKER_PANEL_FEED_LINES);
    expect(lines.map(line => line.text)).toEqual(["step 48", "step 49", "step 50"]);
  });

  it("ignores events belonging to other sessions", () => {
    const lines = workerFeedLines([
      event(1, { sessionId: "someone-else", title: "not mine" }),
      event(2, { title: "mine" }),
    ], "child");
    expect(lines.map(line => line.text)).toEqual(["mine"]);
  });

  it("skips frames with nothing legible in them", () => {
    const lines = workerFeedLines([
      event(1, { kind: "message.delta", text: "" }),
      event(2, { kind: "turn.started" }),
      event(3, { text: "  reading the store  " }),
    ], "child");
    expect(lines.map(line => line.text)).toEqual(["reading the store"]);
  });

  it("collapses repeated identical lines", () => {
    const lines = workerFeedLines([
      event(1, { title: "running cargo test" }),
      event(2, { title: "running cargo test" }),
      event(3, { title: "running cargo test" }),
      event(4, { title: "done" }),
    ], "child");
    expect(lines.map(line => line.text)).toEqual(["running cargo test", "done"]);
    // The collapsed line adopts the newest event id so React re-keys as it moves.
    expect(lines[0].id).toBe(3);
  });

  it("prefers text over title when both are present", () => {
    const lines = workerFeedLines([event(1, { title: "Bash", text: "cargo test auth" })], "child");
    expect(lines[0].text).toBe("cargo test auth");
  });
});
