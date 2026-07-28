import { describe, expect, it } from "vitest";
import { isBroken, isRunning, workerStatus } from "./WorkerObservabilityPanel";
import type { Session, WorkerRuntimeRecord } from "../types";

const session = (over: Partial<Session> = {}): Session => ({
  id: "w1", workspaceId: "ws", harness: "codex", label: "Implementation", status: "working",
  startedAt: null, endedAt: null, contextPercent: null, usagePercent: null, metricSource: "estimated",
  restorationMode: "fresh", parentSessionId: "p", ...over,
});
const runtime = (over: Partial<WorkerRuntimeRecord> = {}): WorkerRuntimeRecord => ({
  sessionId: "w1", parentSessionId: "p", lifecycleState: "working", taskFamily: "implementation",
  compatibilityKey: "k", resultStatus: "pending", retryCount: 0, warmUntil: null,
  worktreePath: null, worktreeBranch: null, lastResult: null, updatedAt: "now", ...over,
});

describe("workerStatus", () => {
  it("ignores a stale last result while the reused worker is pending", () => {
    // Reuse sets result_status back to pending but leaves the old result attached.
    const status = workerStatus(session({ status: "working" }), runtime({
      resultStatus: "pending", lifecycleState: "working",
      lastResult: { status: "completed", summary: "previous task done" },
    }));
    expect(status).toMatchObject({ tone: "working", label: "WORKING" });
  });

  it("shows the reported result once the worker has reported", () => {
    expect(workerStatus(session(), runtime({ resultStatus: "reported", lifecycleState: "completed", lastResult: { status: "completed", summary: "done" } })))
      .toMatchObject({ tone: "done", label: "DONE" });
  });

  it("distinguishes a stalled worker from an ordinary failure", () => {
    expect(workerStatus(session(), runtime({ resultStatus: "reported", lastResult: { status: "failed", summary: "Impl stopped responding (no output for 600s) and was stopped" } })))
      .toMatchObject({ tone: "stalled", label: "STALLED" });
    expect(workerStatus(session(), runtime({ resultStatus: "reported", lastResult: { status: "failed", summary: "hit a compile error" } })))
      .toMatchObject({ tone: "failed", label: "FAILED" });
  });

  it("treats reported blocked / needs_delegation as attention, not running", () => {
    const blocked = workerStatus(session(), runtime({ resultStatus: "reported", lastResult: { status: "blocked", summary: "needs approval" } }));
    const needs = workerStatus(session(), runtime({ resultStatus: "reported", lastResult: { status: "needs_delegation", summary: "hand off to verification" } }));
    expect(blocked.tone).toBe("attention");
    expect(needs.tone).toBe("attention");
    expect(isRunning(blocked.tone)).toBe(false);
    expect(isRunning(needs.tone)).toBe(false);
  });

  it("counts only genuinely-active workers as running and failures as broken", () => {
    expect(isRunning("working")).toBe(true);
    expect(isRunning("waiting")).toBe(false);
    expect(isBroken("stalled")).toBe(true);
    expect(isBroken("failed")).toBe(true);
    expect(isBroken("done")).toBe(false);
  });
});
