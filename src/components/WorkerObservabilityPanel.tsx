import { useMemo, useState } from "react";
import { AlertTriangle, Bot, Check, ChevronDown, Clock3, LoaderCircle, RefreshCw } from "lucide-react";
import type { BridgeEvent, Session, WorkerRuntimeRecord } from "../types";

type Tone = "working" | "waiting" | "warm" | "done" | "failed" | "stalled" | "idle";

type WorkerStatus = { tone: Tone; label: string; detail?: string };

// Derive a single presentable status from the worker's lifecycle state and its
// last typed result. A stalled worker (stopped responding) is distinguished
// from an ordinary crash so the operator knows the watchdog intervened.
function workerStatus(session: Session, runtime?: WorkerRuntimeRecord): WorkerStatus {
  const lastResult = runtime?.lastResult as { status?: string; summary?: string } | null | undefined;
  const resultStatus = typeof lastResult?.status === "string" ? lastResult.status : undefined;
  const summary = typeof lastResult?.summary === "string" ? lastResult.summary : undefined;
  const lifecycle = runtime?.lifecycleState ?? session.status;
  if (resultStatus === "failed" || lifecycle === "failed") {
    const stalled = !!summary && /stopped responding/i.test(summary);
    return { tone: stalled ? "stalled" : "failed", label: stalled ? "STALLED" : "FAILED", detail: summary };
  }
  if (resultStatus === "cancelled" || lifecycle === "cancelled") return { tone: "failed", label: "CANCELLED", detail: summary };
  if (resultStatus === "blocked") return { tone: "waiting", label: "BLOCKED", detail: summary };
  if (resultStatus === "needs_delegation") return { tone: "waiting", label: "NEEDS DELEGATION", detail: summary };
  if (resultStatus === "completed" || lifecycle === "completed" || lifecycle === "stopped" || lifecycle === "ready") return { tone: "done", label: "DONE", detail: summary };
  if (lifecycle === "working") return { tone: "working", label: "WORKING" };
  if (lifecycle === "waiting") return { tone: "waiting", label: "NEEDS YOU" };
  if (lifecycle === "warm") return { tone: "warm", label: "WARM" };
  if (lifecycle === "checkpointing") return { tone: "warm", label: "CHECKPOINTING" };
  if (lifecycle === "resuming" || lifecycle === "restored" || lifecycle === "starting") return { tone: "working", label: lifecycle.toUpperCase() };
  return { tone: "idle", label: (lifecycle ?? "idle").toUpperCase() };
}

const toneDot: Record<Tone, string> = {
  working: "bg-success animate-pulse", waiting: "bg-warning", warm: "bg-info",
  done: "bg-info", failed: "bg-destructive", stalled: "bg-destructive", idle: "bg-ring",
};
const toneText: Record<Tone, string> = {
  working: "text-success", waiting: "text-warning", warm: "text-info",
  done: "text-neutral-400", failed: "text-destructive", stalled: "text-destructive", idle: "text-neutral-500",
};

function StatusIcon({ tone }: { tone: Tone }) {
  if (tone === "working") return <LoaderCircle size={12} className="animate-spin text-success" aria-hidden="true" />;
  if (tone === "failed" || tone === "stalled") return <AlertTriangle size={12} className="text-destructive" aria-hidden="true" />;
  if (tone === "done") return <Check size={12} className="text-info" aria-hidden="true" />;
  if (tone === "waiting") return <Clock3 size={12} className="text-warning" aria-hidden="true" />;
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

  const live = rows.filter(row => row.status.tone === "working" || row.status.tone === "waiting").length;
  const broken = rows.filter(row => row.status.tone === "failed" || row.status.tone === "stalled").length;

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
          {live > 0 && <span className="inline-flex items-center gap-1 text-[9.5px] text-success"><span className="h-1.5 w-1.5 rounded-full bg-success animate-pulse" />{live} running</span>}
          {broken > 0 && <span className="inline-flex items-center gap-1 text-[9.5px] text-destructive"><AlertTriangle size={10} aria-hidden="true" />{broken} failed</span>}
          <ChevronDown size={14} className={`ml-auto text-neutral-600 transition-transform ${collapsed ? "-rotate-90" : ""}`} aria-hidden="true" />
        </button>
        {!collapsed && <div className="max-h-[240px] overflow-y-auto border-t border-white/[0.05]">
          {rows.map(({ worker, runtime, status, activity }) => (
            <div key={worker.id} className="flex items-start gap-2.5 border-b border-white/[0.035] px-3 py-2 last:border-0">
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
          ))}
        </div>}
      </div>
    </div>
  );
}
