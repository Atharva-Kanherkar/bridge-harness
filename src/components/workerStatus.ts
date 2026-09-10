import type { Session, WorkerRuntimeRecord } from "../types";

export type WorkerTone = "working" | "waiting" | "attention" | "warm" | "done" | "failed" | "stalled" | "idle";

export type WorkerStatus = { tone: WorkerTone; label: string; detail?: string };

// Trust a typed result only after it is reported. Reused warm workers retain
// their prior result while pending, so lifecycle state remains authoritative.
export function workerStatus(session: Session, runtime?: WorkerRuntimeRecord): WorkerStatus {
  if (!runtime && session.parentSessionId) return { tone: "attention", label: "STATUS UNAVAILABLE", detail: "Worker runtime has not been loaded." };
  const reported = runtime?.resultStatus === "reported";
  const lastResult = reported ? (runtime?.lastResult as { status?: string; summary?: string } | null | undefined) : undefined;
  const resultStatus = typeof lastResult?.status === "string" ? lastResult.status : undefined;
  const summary = typeof lastResult?.summary === "string" ? lastResult.summary : undefined;
  // Bridge's own verdict, sent as a classification. This used to be
  // `/stopped responding/i` against the summary — a regex over a Rust
  // `format!` string, so rewording one line in `live_turn.rs` silently
  // downgraded every stall to a generic failure.
  const failureClass = runtime?.failureClass ?? undefined;
  if (reported && resultStatus) {
    if (resultStatus === "failed") {
      if (failureClass === "stalled") return { tone: "stalled", label: "STALLED", detail: summary };
      if (failureClass === "protocol_invalid") return { tone: "attention", label: "UNREADABLE RESULT", detail: summary };
      return { tone: "failed", label: "FAILED", detail: summary };
    }
    // A cancellation is a decision someone made, not a fault. Rendering it in
    // the same destructive red as a crash made every deliberate stop look
    // like something had gone wrong.
    if (resultStatus === "cancelled") return { tone: "idle", label: "CANCELLED", detail: summary };
    if (resultStatus === "protocol_invalid") return { tone: "attention", label: "UNREADABLE RESULT", detail: summary };
    if (resultStatus === "blocked") return { tone: "attention", label: "BLOCKED", detail: summary };
    if (resultStatus === "needs_delegation") return { tone: "attention", label: "NEEDS DELEGATION", detail: summary };
    if (resultStatus === "completed") return { tone: "done", label: "DONE", detail: summary };
  }
  const lifecycle = runtime?.lifecycleState ?? session.status;
  if (lifecycle === "failed") return { tone: "failed", label: "FAILED" };
  if (lifecycle === "cancelled") return { tone: "idle", label: "CANCELLED" };
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
