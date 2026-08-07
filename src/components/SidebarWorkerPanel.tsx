import { useEffect, useState } from "react";
import { AlertTriangle, Bot, Check, Clock3, LoaderCircle, RefreshCw } from "lucide-react";
import type { BridgeEvent, Session, WorkerRuntimeRecord } from "../types";
import { cn } from "@/lib/utils";
import { isBroken, isRunning, isWaiting, workerStatus, type WorkerTone } from "./workerStatus";

const toneDot: Record<WorkerTone, string> = {
  working: "bg-emerald-400",
  waiting: "bg-amber-400",
  attention: "bg-amber-400",
  warm: "bg-sky-400",
  done: "bg-sky-500/70",
  failed: "bg-red-500",
  stalled: "bg-red-500",
  idle: "bg-neutral-500",
};

const toneText: Record<WorkerTone, string> = {
  working: "text-emerald-400",
  waiting: "text-amber-400",
  attention: "text-amber-400",
  warm: "text-sky-400",
  done: "text-neutral-500",
  failed: "text-red-400",
  stalled: "text-red-400",
  idle: "text-neutral-500",
};

function WorkerIcon({ tone }: { tone: WorkerTone }) {
  if (tone === "working") return <LoaderCircle size={11} className="animate-spin text-emerald-400" aria-hidden="true" />;
  if (tone === "failed" || tone === "stalled") return <AlertTriangle size={11} className="text-red-400" aria-hidden="true" />;
  if (tone === "done") return <Check size={11} className="text-sky-500/70" aria-hidden="true" />;
  if (tone === "waiting" || tone === "attention") return <Clock3 size={11} className="text-amber-400" aria-hidden="true" />;
  return <span className={cn("h-1.5 w-1.5 rounded-full", toneDot[tone])} />;
}

function relativeUpdate(value: string | undefined, now: number): string | undefined {
  if (!value) return undefined;
  const elapsed = Math.max(0, now - Date.parse(value));
  if (!Number.isFinite(elapsed)) return undefined;
  if (elapsed < 10_000) return "now";
  if (elapsed < 60_000) return `${Math.floor(elapsed / 1000)}s ago`;
  if (elapsed < 3_600_000) return `${Math.floor(elapsed / 60_000)}m ago`;
  return `${Math.floor(elapsed / 3_600_000)}h ago`;
}

export function SidebarWorkerPanel({
  workers,
  runtimes,
  reasons,
  collapsed,
  now,
}: {
  workers: Session[];
  runtimes: WorkerRuntimeRecord[];
  reasons: BridgeEvent[];
  collapsed: boolean;
  now?: number;
}) {
  const [liveNow, setLiveNow] = useState(Date.now);
  useEffect(() => {
    if (now !== undefined) return;
    const timer = window.setInterval(() => setLiveNow(Date.now()), 1_000);
    return () => window.clearInterval(timer);
  }, [now]);
  if (!workers.length) return null;
  const effectiveNow = now ?? liveNow;
  const rows = workers.map(worker => {
    const runtime = runtimes.find(item => item.sessionId === worker.id);
    const status = workerStatus(worker, runtime);
    const activity = status.detail ?? reasons
      .filter(reason => reason.entityId === worker.id)
      .sort((a, b) => b.id - a.id)[0]?.body;
    return { worker, runtime, status, activity };
  });
  const running = rows.filter(row => isRunning(row.status.tone)).length;
  const waiting = rows.filter(row => isWaiting(row.status.tone)).length;
  const broken = rows.filter(row => isBroken(row.status.tone)).length;

  if (collapsed) {
    const tone = broken ? "text-red-400" : running ? "text-emerald-400" : "text-neutral-500";
    return (
      <div className="mb-3 flex justify-center" title={`${workers.length} worker${workers.length === 1 ? "" : "s"}: ${running} running, ${waiting} waiting, ${broken} failed`}>
        <div className={cn("relative flex h-10 w-10 items-center justify-center rounded-xl border border-white/[0.06] bg-white/[0.035]", tone)}>
          <Bot size={16} strokeWidth={1.6} aria-hidden="true" />
          {running > 0 && <span className="absolute right-1.5 top-1.5 h-1.5 w-1.5 animate-pulse rounded-full bg-emerald-400" />}
          <span className="absolute bottom-1 right-1 rounded-full bg-[#17171c] px-1 font-mono text-[8px] text-neutral-300">{workers.length}</span>
        </div>
      </div>
    );
  }

  return (
    <section className="mb-4 shrink-0 overflow-hidden rounded-xl border border-white/[0.065] bg-white/[0.025]" aria-label="Live workers">
      <div className="flex h-8 items-center gap-2 border-b border-white/[0.05] px-2.5">
        <Bot size={12} className="text-neutral-500" strokeWidth={1.6} aria-hidden="true" />
        <span className="text-[10px] font-semibold uppercase tracking-[0.12em] text-neutral-500">Live workers</span>
        <span className="font-mono text-[9px] text-neutral-600">{workers.length}</span>
        <span className="ml-auto flex items-center gap-1.5">
          {running > 0 && <span className="inline-flex items-center gap-1 text-[9px] text-emerald-400"><span className="h-1.5 w-1.5 animate-pulse rounded-full bg-emerald-400" />{running} active</span>}
          {waiting > 0 && <span className="text-[9px] text-amber-400">{waiting} waiting</span>}
          {broken > 0 && <span className="text-[9px] text-red-400">{broken} failed</span>}
        </span>
      </div>
      <div className="max-h-48 overflow-y-auto">
        {rows.map(({ worker, runtime, status, activity }) => (
          <div key={worker.id} className={cn("border-b border-white/[0.04] px-2.5 py-2 last:border-0", isBroken(status.tone) && "border-l-2 border-l-red-500/70 bg-red-500/[0.05]")}>
            <div className="flex min-w-0 items-center gap-2">
              <span className="flex w-3 shrink-0 justify-center"><WorkerIcon tone={status.tone} /></span>
              <span className="min-w-0 flex-1 truncate text-[11px] font-medium text-neutral-200">{worker.label}</span>
              <span className={cn("shrink-0 text-[8px] font-semibold tracking-[0.06em]", toneText[status.tone])}>{status.label}</span>
            </div>
            <div className="ml-5 mt-0.5 flex items-center gap-1.5 text-[9px] text-neutral-600">
              <span className="truncate">{runtime?.taskFamily ?? worker.harness}</span>
              {runtime?.retryCount ? <span className="inline-flex items-center gap-0.5"><RefreshCw size={8} aria-hidden="true" />retry {runtime.retryCount}</span> : null}
              <span className="ml-auto shrink-0">{relativeUpdate(runtime?.lastActivityAt ?? runtime?.updatedAt, effectiveNow)}</span>
            </div>
            {activity && <p className="ml-5 mt-1 line-clamp-2 text-[9.5px] leading-3.5 text-neutral-500">{activity}</p>}
          </div>
        ))}
      </div>
    </section>
  );
}
