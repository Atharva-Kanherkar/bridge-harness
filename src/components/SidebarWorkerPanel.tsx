import { useEffect, useState } from "react";
import { AlertTriangle, Bot, Check, Clock3, LoaderCircle, RefreshCw } from "lucide-react";
import type { BridgeEvent, Session, WorkerRuntimeRecord } from "../types";
import { cn } from "@/lib/utils";
import { isBroken, isRunning, isWaiting, workerStatus, type WorkerTone } from "./workerStatus";

// Chrome stays achromatic; only the status ink carries hue, so a worker row and
// its Mission Control tile read the same at a glance.
const toneDot: Record<WorkerTone, string> = {
  working: "bg-success",
  waiting: "bg-warning",
  attention: "bg-warning",
  warm: "bg-info",
  done: "bg-muted-foreground",
  failed: "bg-destructive",
  stalled: "bg-destructive",
  idle: "bg-muted-foreground",
};

const toneText: Record<WorkerTone, string> = {
  working: "text-success",
  waiting: "text-warning",
  attention: "text-warning",
  warm: "text-info",
  done: "text-muted-foreground",
  failed: "text-destructive",
  stalled: "text-destructive",
  idle: "text-muted-foreground",
};

function WorkerIcon({ tone }: { tone: WorkerTone }) {
  if (tone === "working") return <LoaderCircle size={11} className="animate-spin text-success" aria-hidden="true" />;
  if (tone === "failed" || tone === "stalled") return <AlertTriangle size={11} className="text-destructive" aria-hidden="true" />;
  if (tone === "done") return <Check size={11} className="text-muted-foreground" aria-hidden="true" />;
  if (tone === "waiting" || tone === "attention") return <Clock3 size={11} className="text-warning" aria-hidden="true" />;
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
    const tone = broken ? "text-destructive" : running ? "text-success" : "text-muted-foreground";
    return (
      <div className="mb-3 flex justify-center" title={`${workers.length} worker${workers.length === 1 ? "" : "s"}: ${running} running, ${waiting} waiting, ${broken} failed`}>
        <div className={cn("relative flex h-10 w-10 items-center justify-center rounded-xl border border-border bg-card", tone)}>
          <Bot size={16} strokeWidth={1.6} aria-hidden="true" />
          {running > 0 && <span className="absolute right-1.5 top-1.5 h-1.5 w-1.5 animate-pulse rounded-full bg-success" />}
          <span className="absolute bottom-1 right-1 rounded-full bg-card px-1 font-mono text-[8px] text-foreground">{workers.length}</span>
        </div>
      </div>
    );
  }

  return (
    <section className="mb-4 shrink-0 overflow-hidden rounded-lg border border-border bg-card" aria-label="Live workers">
      <div className="flex min-h-8 flex-wrap items-center gap-x-2 gap-y-0.5 border-b border-border px-2.5 py-1">
        <Bot size={12} className="shrink-0 text-muted-foreground" strokeWidth={1.6} aria-hidden="true" />
        <span className="text-[10px] font-semibold uppercase tracking-[0.12em] text-muted-foreground">Live workers</span>
        <span className="font-mono text-[9px] text-muted-foreground/70">{workers.length}</span>
        <span className="ml-auto flex flex-wrap items-center gap-1.5">
          {running > 0 && <span className="inline-flex items-center gap-1 text-[9px] text-success"><span className="h-1.5 w-1.5 animate-pulse rounded-full bg-success" />{running} active</span>}
          {waiting > 0 && <span className="text-[9px] text-warning">{waiting} waiting</span>}
          {broken > 0 && <span className="text-[9px] text-destructive">{broken} failed</span>}
        </span>
      </div>
      <div className="max-h-48 overflow-y-auto">
        {rows.map(({ worker, runtime, status, activity }) => (
          // A failed row stays neutral and carries its failure in the left tick
          // and the status ink, so a dense list never turns into a wall of red.
          <div key={worker.id} className={cn("border-b border-border px-2.5 py-2 last:border-0", isBroken(status.tone) && "border-l-2 border-l-destructive")}>
            <div className="flex min-w-0 items-center gap-2">
              <span className="flex w-3 shrink-0 justify-center"><WorkerIcon tone={status.tone} /></span>
              <span className="min-w-0 flex-1 truncate text-[11px] font-medium text-foreground">{worker.label}</span>
              <span className={cn("shrink-0 text-[8px] font-semibold tracking-[0.06em]", toneText[status.tone])}>{status.label}</span>
            </div>
            <div className="ml-5 mt-0.5 flex items-center gap-1.5 text-[9px] text-muted-foreground/70">
              <span className="truncate">{runtime?.taskFamily ?? worker.harness}</span>
              {runtime?.retryCount ? <span className="inline-flex items-center gap-0.5"><RefreshCw size={8} aria-hidden="true" />retry {runtime.retryCount}</span> : null}
              <span className="ml-auto shrink-0 font-mono">{relativeUpdate(runtime?.lastActivityAt ?? runtime?.updatedAt, effectiveNow)}</span>
            </div>
            {activity && <p className="ml-5 mt-1 line-clamp-2 text-[9.5px] leading-3.5 text-muted-foreground">{activity}</p>}
          </div>
        ))}
      </div>
    </section>
  );
}
