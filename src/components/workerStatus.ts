import type { Session, WorkerRuntimeRecord } from "../types";

export type WorkerTone = "working" | "waiting" | "attention" | "warm" | "done" | "failed" | "stalled" | "idle";

export type WorkerStatus = { tone: WorkerTone; label: string; detail?: string };

// Trust a typed result only after it is reported. Reused warm workers retain
// their prior result while pending, so lifecycle state remains authoritative.
export function workerStatus(session: Session, runtime?: WorkerRuntimeRecord): WorkerStatus {
  const reported = runtime?.resultStatus === "reported";
  const lastResult = reported ? (runtime?.lastResult as { status?: string; summary?: string } | null | undefined) : undefined;
  const resultStatus = typeof lastResult?.status === "string" ? lastResult.status : undefined;
  const summary = typeof lastResult?.summary === "string" ? lastResult.summary : undefined;
  if (reported && resultStatus) {
    if (resultStatus === "failed") {
      const stalled = !!summary && /stopped responding/i.test(summary);
      return { tone: stalled ? "stalled" : "failed", label: stalled ? "STALLED" : "FAILED", detail: summary };
    }
    if (resultStatus === "cancelled") return { tone: "failed", label: "CANCELLED", detail: summary };
    if (resultStatus === "blocked") return { tone: "attention", label: "BLOCKED", detail: summary };
    if (resultStatus === "needs_delegation") return { tone: "attention", label: "NEEDS DELEGATION", detail: summary };
    if (resultStatus === "completed") return { tone: "done", label: "DONE", detail: summary };
  }
  const lifecycle = runtime?.lifecycleState ?? session.status;
  if (lifecycle === "failed") return { tone: "failed", label: "FAILED" };
  if (lifecycle === "cancelled") return { tone: "failed", label: "CANCELLED" };
  if (lifecycle === "working") return { tone: "working", label: "WORKING" };
  if (lifecycle === "waiting") return { tone: "waiting", label: "NEEDS YOU" };
  if (lifecycle === "warm") return { tone: "warm", label: "WARM" };
  if (lifecycle === "checkpointing") return { tone: "warm", label: "CHECKPOINTING" };
  if (lifecycle === "resuming" || lifecycle === "restored" || lifecycle === "starting") return { tone: "working", label: lifecycle.toUpperCase() };
  if (lifecycle === "completed" || lifecycle === "stopped" || lifecycle === "ready") return { tone: "done", label: "DONE" };
  return { tone: "idle", label: (lifecycle ?? "idle").toUpperCase() };
}

export function isRunning(tone: WorkerTone): boolean {
  return tone === "working";
}

export function isBroken(tone: WorkerTone): boolean {
  return tone === "failed" || tone === "stalled";
}

export function isWaiting(tone: WorkerTone): boolean {
  return tone === "waiting" || tone === "attention";
}

export function isVisibleWorker(session: Session, runtime?: WorkerRuntimeRecord): boolean {
  return workerStatus(session, runtime).tone !== "done";
}
