import { expect, it } from "vitest";
import type { Session, WorkerRuntimeRecord, BridgeEvent } from "../types";
import { canStopWorker, reportedResult, workerClock, workerDiagnostics } from "./WorkerControls";

it("offers stop during every non-terminal lifecycle and freezes reported clocks", () => {
  const session = { id: "w", status: "working", endedAt: null } as Session;
  for (const lifecycleState of ["starting", "working", "waiting", "warm", "checkpointing", "resuming", "failed"]) {
    expect(canStopWorker(session, { lifecycleState } as WorkerRuntimeRecord)).toBe(true);
  }
  expect(canStopWorker({ ...session, endedAt: "2026-09-09T00:00:00Z" })).toBe(false);
  expect(workerClock(session, { resultStatus: "reported", updatedAt: "2026-09-09T00:00:00Z" } as WorkerRuntimeRecord, Date.now())).toBe(Date.parse("2026-09-09T00:00:00Z"));
});
it("bounds diagnostics, preserves refusals, and excludes other workers", () => {
  const reasons = Array.from({ length: 20 }, (_, id) => ({ id, entityId: "w", kind: "worker.retry.declined", body: "No automatic attempts left" } as BridgeEvent));
  reasons.push({ id: 21, entityId: "other", kind: "router.no_eligible_route" } as BridgeEvent);
  const result = workerDiagnostics(reasons, "w");
  expect(result).toHaveLength(8);
  expect(result[0].id).toBe(19);
  expect(result[0].label).toBe("Retry declined");
});
it("reads a result only once the worker has reported, and keeps the shapes it can show", () => {
  const last = { status: "completed", summary: " Rotation added. ", filesChanged: ["src/auth/store.rs", 7], tests: [{ command: "cargo test auth", status: "passed" }, { status: "failed" }] };
  expect(reportedResult("pending", last)).toBeUndefined();
  expect(reportedResult("reported", null)).toBeUndefined();
  expect(reportedResult("reported", last)).toEqual({
    status: "completed", summary: "Rotation added.", filesChanged: ["src/auth/store.rs"], tests: [{ command: "cargo test auth", status: "passed" }],
  });
});
