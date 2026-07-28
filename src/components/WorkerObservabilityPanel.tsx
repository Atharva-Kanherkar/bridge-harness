import { useMemo, useState } from "react";
import { AlertTriangle, Bot, Check, ChevronDown, Clock3, LoaderCircle, RefreshCw } from "lucide-react";
import type { BridgeEvent, Session, WorkerRuntimeRecord } from "../types";

export type WorkerTone = "working" | "waiting" | "attention" | "warm" | "done" | "failed" | "stalled" | "idle";

export type WorkerStatus = { tone: WorkerTone; label: string; detail?: string };

// Derive a single presentable status. The last typed result is only trusted
// once the worker has actually reported it (`resultStatus === "reported"`);
// a reused warm worker is set back to `pending` while its previous result is
// still attached, so a pending worker always reflects its live lifecycle.
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
  // Not yet reported (or reported without a recognized status): reflect the
  // live lifecycle state so a reused worker shows WORKING, not its old result.
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

// A worker is "running" only when it is genuinely active — not when it holds a
// reported blocked/needs_delegation result awaiting an orchestrator decision.
export function isRunning(tone: WorkerTone): boolean {
  return tone === "working";
}
export function isBroken(tone: WorkerTone): boolean {
  return tone === "failed" || tone === "stalled";
}

// Explicit palette colors, chosen so failed/stalled reads unmistakably in dark
// mode rather than collapsing onto a muted semantic token.
const toneDot: Record<WorkerTone, string> = {
  working: "bg-emerald-400 animate-pulse", waiting: "bg-amber-400", attention: "bg-amber-400",
  warm: "bg-sky-400", done: "bg-sky-500/70", failed: "bg-red-500", stalled: "bg-red-500", idle: "bg-neutral-500",
};
const toneText: Record<WorkerTone, string> = {
  working: "text-emerald-400", waiting: "text-amber-400", attention: "text-amber-400",
  warm: "text-sky-400", done: "text-neutral-400", failed: "text-red-400", stalled: "text-red-400", idle: "text-neutral-500",
};

function StatusIcon({ tone }: { tone: WorkerTone }) {
  if (tone === "working") return <LoaderCircle size={12} className="animate-spin text-emerald-400" aria-hidden="true" />;
  if (tone === "failed" || tone === "stalled") return <AlertTriangle size={12} className="text-red-400" aria-hidden="true" />;
  if (tone === "done") return <Check size={12} className="text-sky-400" aria-hidden="true" />;
  if (tone === "waiting" || tone === "attention") return <Clock3 size={12} className="text-amber-400" aria-hidden="true" />;
  return <span className={`h-1.5 w-1.5 rounded-full ${toneDot[tone]}`} />;
}

/**
 * Live visibility into the orchestrator's delegated workers: each child agent's
 * status, retry count, and latest activity — with dead/stalled workers shown as
 * an unmistakable failed state rather than an endless spinner.
 */
export function WorkerObservabilityPanel({
  workers,
  runtimes,
  reasons,
}: {
  workers: Session[];
  runtimes: WorkerRuntimeRecord[];
  reasons: BridgeEvent[];
}) {
  const [collapsed, setCollapsed] = useState(false);
  const rows = useMemo(() => {
    return workers.map(worker => {
      const runtime = runtimes.find(item => item.sessionId === worker.id);
      const status = workerStatus(worker, runtime);
      const latestReason = reasons
        .filter(reason => reason.entityId === worker.id)
        .sort((a, b) => b.id - a.id)[0]?.body;
      return { worker, runtime, status, activity: status.detail ?? latestReason };
    });
  }, [workers, runtimes, reasons]);

  if (!rows.length) return null;

  const running = rows.filter(row => isRunning(row.status.tone)).length;
  const broken = rows.filter(row => isBroken(row.status.tone)).length;
  const attention = rows.filter(row => row.status.tone === "attention" || row.status.tone === "waiting").length;

  return (
    <div className="mx-auto w-full max-w-2xl px-4 pt-3 sm:px-6">
      <div className="u-glass-soft overflow-hidden rounded-2xl border border-white/[0.06]">
        <button
          type="button"
          onClick={() => setCollapsed(value => !value)}
          className="flex w-full items-center gap-2 px-3 py-2 text-left transition-colors hover:bg-white/[0.03]"
        >
          <Bot size={13} className="text-neutral-400" aria-hidden="true" />
          <span className="text-[11px] font-medium tracking-tight text-neutral-200">Agents</span>
          <span className="text-[10px] text-neutral-500">{rows.length}</span>
          {running > 0 && <span className="inline-flex items-center gap-1 text-[9.5px] text-emerald-400"><span className="h-1.5 w-1.5 rounded-full bg-emerald-400 animate-pulse" />{running} running</span>}
          {attention > 0 && <span className="inline-flex items-center gap-1 text-[9.5px] text-amber-400"><Clock3 size={10} aria-hidden="true" />{attention} waiting</span>}
          {broken > 0 && <span className="inline-flex items-center gap-1 text-[9.5px] text-red-400"><AlertTriangle size={10} aria-hidden="true" />{broken} failed</span>}
          <ChevronDown size={14} className={`ml-auto text-neutral-600 transition-transform ${collapsed ? "-rotate-90" : ""}`} aria-hidden="true" />
        </button>
        {!collapsed && <div className="max-h-[240px] overflow-y-auto border-t border-white/[0.05]">
          {rows.map(({ worker, runtime, status, activity }) => {
            const broken = isBroken(status.tone);
            return (
              <div key={worker.id} className={`flex items-start gap-2.5 border-b border-white/[0.035] px-3 py-2 last:border-0 ${broken ? "border-l-2 border-l-red-500/70 bg-red-500/[0.06]" : ""}`}>
                <span className="mt-0.5 shrink-0"><StatusIcon tone={status.tone} /></span>
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <b className="truncate text-[11.5px] font-medium text-neutral-200">{worker.label}</b>
                    <span className={`shrink-0 text-[8.5px] font-semibold uppercase tracking-[0.06em] ${toneText[status.tone]}`}>{status.label}</span>
                    {runtime && runtime.retryCount > 0 && <span className="inline-flex shrink-0 items-center gap-0.5 text-[8.5px] text-neutral-500"><RefreshCw size={9} aria-hidden="true" />{runtime.retryCount}</span>}
                  </div>
                  {activity && <p className="mt-0.5 line-clamp-2 text-[10.5px] leading-4 text-neutral-500">{activity}</p>}
                </div>
              </div>
            );
          })}
        </div>}
      </div>
    </div>
  );
}
