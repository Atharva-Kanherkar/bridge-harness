import { useState } from "react";
import { Square } from "lucide-react";
import type { BridgeEvent, Session, WorkerRuntimeRecord } from "../types";

export function canStopWorker(session: Session, runtime?: WorkerRuntimeRecord): boolean {
  return !session.endedAt && !["completed", "cancelled", "stopped"].includes(runtime?.lifecycleState ?? session.status);
}

export function workerClock(session: Session, runtime: WorkerRuntimeRecord | undefined, now: number): number {
  const end = session.endedAt ?? (runtime?.resultStatus === "reported" ? runtime.updatedAt : undefined);
  return end && Number.isFinite(Date.parse(end)) ? Date.parse(end) : now;
}

const DIAGNOSTICS: Record<string, string> = {
  "worker.route.selected": "Why this route",
  "worktree.dependencies": "Dependency installation",
  "worker.retry.declined": "Retry declined",
  "worker.retry.scheduled": "Retry scheduled",
  "router.harness_substituted": "Harness changed",
  "router.harness_quota_exhausted": "Provider limit reached",
  "router.no_eligible_route": "No eligible route",
  "capability.harness_disabled": "Harness disabled",
  "delegation.steer.refused": "Guidance refused",
  "delegation.steer.undeliverable": "Guidance not delivered",
};

export function workerDiagnostics(reasons: BridgeEvent[], sessionId: string) {
  return reasons.filter(reason => reason.entityId === sessionId && DIAGNOSTICS[reason.kind])
    .sort((a, b) => b.id - a.id).slice(0, 8)
    .map(reason => ({ ...reason, label: DIAGNOSTICS[reason.kind] }));
}

export function WorkerDiagnostics({ reasons, sessionId }: { reasons: BridgeEvent[]; sessionId: string }) {
  const diagnostics = workerDiagnostics(reasons, sessionId);
  if (!diagnostics.length) return null;
  return <details className="rounded-lg border border-border bg-card px-3 py-2 text-xs">
    <summary className="cursor-pointer font-medium text-foreground">{diagnostics[0].label}{diagnostics.length > 1 ? ` · ${diagnostics.length} events` : ""}</summary>
    <ol className="mt-2 space-y-2">{diagnostics.map(reason => <li key={reason.id}><p className="font-medium text-foreground">{reason.label}</p><p className="whitespace-pre-wrap break-words text-muted-foreground">{reason.body}</p></li>)}</ol>
  </details>;
}

export function WorkerStopControl({ session, runtime, onStop }: {
  session: Session; runtime?: WorkerRuntimeRecord; onStop?: (id: string) => Promise<void>;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  if (!onStop || !canStopWorker(session, runtime)) return null;
  return <span className="inline-flex flex-col gap-1">
    <button type="button" disabled={busy} aria-label={`Stop ${session.title || session.label}`} className="inline-flex min-h-7 items-center justify-center gap-1.5 rounded-lg border border-border px-2 text-xs text-muted-foreground hover:bg-accent hover:text-foreground disabled:opacity-50" onClick={() => {
      setBusy(true); setError(undefined);
      void onStop(session.id).catch(cause => setError(String(cause))).finally(() => setBusy(false));
    }}><Square size={10} />{busy ? "Stopping..." : "Stop"}</button>
    {error && <span role="alert" className="max-w-64 text-xs text-destructive">{error}</span>}
  </span>;
}
