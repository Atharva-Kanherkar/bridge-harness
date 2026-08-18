import { useEffect, useMemo, useState } from "react";
import { AlertTriangle, Bot, Check, Clock3, CornerDownRight, GitBranch, LayoutGrid, LoaderCircle, RefreshCw } from "lucide-react";
import type { AgentEvent, BridgeEvent, Session, WorkerRuntimeRecord } from "../types";
import { cn } from "@/lib/utils";
import { formatElapsed, harnessLabel } from "../utils";
import { isBroken, isRunning, isWaiting, workerStatus, type WorkerTone } from "./workerStatus";

// Mission Control renders every live agent at once as its own window, instead of
// the single-session view. It reuses the sidebar's tone vocabulary so a tile and
// its sidebar row read the same at a glance.
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

// The border/glow a tile wears. Working tiles get a live accent; anything that
// needs a human (waiting, blocked, failed) is loud enough to pull the eye.
const toneFrame: Record<WorkerTone, string> = {
  working: "border-emerald-400/25",
  waiting: "border-amber-400/40",
  attention: "border-amber-400/40",
  warm: "border-sky-400/25",
  done: "border-white/[0.07]",
  failed: "border-red-500/45",
  stalled: "border-red-500/45",
  idle: "border-white/[0.07]",
};

// Sort so the tiles that demand a human land first, the satisfying live ones
// next, and finished work settles to the end.
const tonePriority: Record<WorkerTone, number> = {
  attention: 0,
  waiting: 1,
  failed: 2,
  stalled: 2,
  working: 3,
  warm: 4,
  done: 5,
  idle: 6,
};

function TileIcon({ tone }: { tone: WorkerTone }) {
  if (tone === "working") return <LoaderCircle size={13} className="animate-spin text-emerald-400" aria-hidden="true" />;
  if (tone === "failed" || tone === "stalled") return <AlertTriangle size={13} className="text-red-400" aria-hidden="true" />;
  if (tone === "done") return <Check size={13} className="text-sky-500/70" aria-hidden="true" />;
  if (tone === "waiting" || tone === "attention") return <Clock3 size={13} className="text-amber-400" aria-hidden="true" />;
  return <span className={cn("h-2 w-2 rounded-full", toneDot[tone])} />;
}

// The last few legible things this agent said or did, newest last. Deltas and
// bare lifecycle events carry no text, so filtering on text keeps the ticker to
// what a human can actually read.
function recentLines(events: AgentEvent[], sessionId: string): { id: number; text: string }[] {
  const out: { id: number; text: string }[] = [];
  for (const event of events) {
    if (event.sessionId !== sessionId) continue;
    const text = (event.text ?? "").trim() || (event.title ?? "").trim();
    if (!text) continue;
    const last = out[out.length - 1];
    if (last && last.text === text) { last.id = event.id; continue; }
    out.push({ id: event.id, text });
  }
  return out.slice(-4);
}

// Workers get the lifecycle/result-driven resolver. A top-level session's own
// status is authoritative and its "ready"/"warm" states are live — resolving both
// the same way would mislabel a ready orchestrator or chat as DONE.
function agentStatus(session: Session, runtime?: WorkerRuntimeRecord): { tone: WorkerTone; label: string; detail?: string } {
  if (session.parentSessionId) return workerStatus(session, runtime);
  switch (session.status) {
    case "failed": return { tone: "failed", label: "FAILED" };
    case "cancelled": return { tone: "failed", label: "CANCELLED" };
    case "working": return { tone: "working", label: "WORKING" };
    case "waiting": return { tone: "waiting", label: "NEEDS YOU" };
    case "ready": return { tone: "warm", label: "READY" };
    case "warm": return { tone: "warm", label: "WARM" };
    case "checkpointing": return { tone: "warm", label: "CHECKPOINTING" };
    case "starting": case "resuming": case "restored": return { tone: "working", label: session.status.toUpperCase() };
    case "completed": case "stopped": return { tone: "done", label: "DONE" };
    default: return { tone: "idle", label: (session.status ?? "idle").toUpperCase() };
  }
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

type Agent = {
  session: Session;
  runtime?: WorkerRuntimeRecord;
  tone: WorkerTone;
  label: string;
  detail?: string;
  lines: { id: number; text: string }[];
  activity?: string;
};

function AgentTile({ agent, active, now, onFocus }: { agent: Agent; active: boolean; now: number; onFocus: () => void }) {
  const { session, runtime, tone, label, detail, lines, activity } = agent;
  const isWorker = !!session.parentSessionId;
  const needsYou = isWaiting(tone);
  const stream = lines.length ? lines : detail ? [{ id: -1, text: detail }] : activity ? [{ id: -2, text: activity }] : [];
  return (
    <button
      type="button"
      onClick={onFocus}
      aria-label={`Focus ${session.title || session.label}`}
      className={cn(
        "group relative flex h-56 min-w-0 flex-col overflow-hidden rounded-2xl border text-left transition-all u-glass",
        toneFrame[tone],
        "hover:-translate-y-0.5 hover:border-white/20 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-white/40",
        active && "ring-1 ring-white/40",
      )}
    >
      {isRunning(tone) && <span className="mission-live-accent pointer-events-none absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent via-emerald-400/80 to-transparent" />}
      {needsYou && <span className="pointer-events-none absolute inset-0 bg-amber-400/[0.04]" />}
      {isBroken(tone) && <span className="pointer-events-none absolute inset-0 bg-red-500/[0.05]" />}

      <div className="flex shrink-0 items-center gap-2 border-b border-white/[0.05] px-3.5 py-2.5">
        <span className="flex w-4 shrink-0 justify-center">{isWorker ? <CornerDownRight size={13} className="text-neutral-500" aria-hidden="true" /> : <Bot size={14} className="text-neutral-400" strokeWidth={1.7} aria-hidden="true" />}</span>
        <span className="min-w-0 flex-1 truncate text-[12.5px] font-semibold text-neutral-100">{session.title || session.label}</span>
        <span className="flex shrink-0 items-center gap-1.5">
          <TileIcon tone={tone} />
          <span className={cn("text-[8.5px] font-semibold tracking-[0.07em]", toneText[tone])}>{label}</span>
        </span>
      </div>

      <div className="flex shrink-0 items-center gap-1.5 px-3.5 pt-2 font-mono text-[9px] text-neutral-600">
        <span className="truncate">{runtime?.taskFamily ?? (session.kind === "orchestrator" ? "orchestrator" : harnessLabel(session.harness))}</span>
        {runtime?.retryCount ? <span className="inline-flex items-center gap-0.5"><RefreshCw size={8} aria-hidden="true" />retry {runtime.retryCount}</span> : null}
        <GitBranch size={9} aria-hidden="true" className="ml-auto shrink-0" />
        <span className="shrink-0">{formatElapsed(session.startedAt, now)}</span>
        <span className="shrink-0 text-neutral-700">{relativeUpdate(runtime?.lastActivityAt ?? runtime?.updatedAt, now)}</span>
      </div>

      {needsYou && (
        <div className="mx-3.5 mt-2 flex shrink-0 items-center gap-1.5 rounded-md border border-amber-400/30 bg-amber-400/10 px-2 py-1 text-[10px] font-medium text-amber-300">
          <AlertTriangle size={11} aria-hidden="true" />
          <span className="truncate">Needs your approval — click to open</span>
        </div>
      )}

      <div className="relative mt-2 min-h-0 flex-1 overflow-hidden px-3.5 pb-3">
        {stream.length ? (
          <div className="flex flex-col gap-1">
            {stream.map((line, index) => (
              <p key={line.id} className={cn("line-clamp-2 font-mono text-[10px] leading-[1.5]", index === stream.length - 1 ? "text-neutral-300" : "text-neutral-600")}>{line.text}</p>
            ))}
          </div>
        ) : isRunning(tone) ? (
          <div className="thinking-shimmer h-[2px] w-16 rounded-full" />
        ) : (
          <p className="font-mono text-[10px] text-neutral-600">No recent activity.</p>
        )}
        <span className="pointer-events-none absolute inset-x-0 bottom-0 h-8 bg-gradient-to-t from-[color-mix(in_srgb,var(--color-card)_60%,transparent)] to-transparent" />
      </div>
    </button>
  );
}

export function MissionControl({
  sessions,
  runtimes,
  reasons,
  events,
  activeSessionId,
  now,
  onFocusSession,
}: {
  sessions: Session[];
  runtimes: WorkerRuntimeRecord[];
  reasons: BridgeEvent[];
  events: AgentEvent[];
  activeSessionId?: string;
  now?: number;
  onFocusSession: (sessionId: string) => void;
}) {
  const [liveNow, setLiveNow] = useState(Date.now);
  useEffect(() => {
    if (now !== undefined) return;
    const timer = window.setInterval(() => setLiveNow(Date.now()), 1_000);
    return () => window.clearInterval(timer);
  }, [now]);
  const effectiveNow = now ?? liveNow;

  const agents = useMemo(() => {
    const list: Agent[] = [];
    for (const session of sessions) {
      if (session.harness === "shell") continue;
      const runtime = runtimes.find(item => item.sessionId === session.id);
      const isActive = session.id === activeSessionId;
      // Runtimes and reasons are scoped to the loaded forest. A worker we have no
      // runtime for belongs to a forest we did not load, so its status cannot be
      // trusted (a failed one would read as DONE); leave it out rather than lie.
      if (session.parentSessionId && !runtime && !isActive) continue;
      const status = agentStatus(session, runtime);
      // Only live agents belong on the grid. Idle chats and finished sessions are
      // dropped (keeping the active one so returning to the grid never blanks),
      // which also stops completed history from growing the grid without bound.
      if ((status.tone === "idle" || status.tone === "done") && !isActive) continue;
      const activity = reasons
        .filter(reason => reason.entityId === session.id)
        .sort((a, b) => b.id - a.id)[0]?.body;
      list.push({ session, runtime, tone: status.tone, label: status.label, detail: status.detail, lines: recentLines(events, session.id), activity });
    }
    list.sort((a, b) => {
      const priority = tonePriority[a.tone] - tonePriority[b.tone];
      if (priority !== 0) return priority;
      return (a.session.title || a.session.label).localeCompare(b.session.title || b.session.label);
    });
    return list;
  }, [sessions, runtimes, reasons, events, activeSessionId]);

  const running = agents.filter(agent => isRunning(agent.tone)).length;
  const waiting = agents.filter(agent => isWaiting(agent.tone)).length;
  const broken = agents.filter(agent => isBroken(agent.tone)).length;

  return (
    <div className="flex min-h-0 flex-1 flex-col animate-page-mount">
      <div className="flex shrink-0 items-center gap-2.5 border-b border-white/[0.04] px-4 py-3 sm:px-6">
        <LayoutGrid size={15} className="text-neutral-400" aria-hidden="true" />
        <h1 className="m-0 font-display text-sm font-semibold tracking-tight text-white">Mission Control</h1>
        <span className="font-mono text-[10px] text-neutral-600">{agents.length} agent{agents.length === 1 ? "" : "s"}</span>
        <span className="ml-auto flex items-center gap-3 text-[10px]">
          {running > 0 && <span className="inline-flex items-center gap-1.5 text-emerald-400"><span className="h-1.5 w-1.5 animate-pulse rounded-full bg-emerald-400" />{running} active</span>}
          {waiting > 0 && <span className="text-amber-400">{waiting} need you</span>}
          {broken > 0 && <span className="text-red-400">{broken} failed</span>}
        </span>
      </div>
      {agents.length ? (
        <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain p-4 sm:p-6">
          <div className="grid gap-3 [grid-template-columns:repeat(auto-fill,minmax(300px,1fr))]">
            {agents.map(agent => (
              <AgentTile
                key={agent.session.id}
                agent={agent}
                active={agent.session.id === activeSessionId}
                now={effectiveNow}
                onFocus={() => onFocusSession(agent.session.id)}
              />
            ))}
          </div>
        </div>
      ) : (
        <div className="grid min-h-0 flex-1 place-items-center px-6 text-center">
          <div className="max-w-sm">
            <div className="mx-auto mb-3 grid h-12 w-12 place-items-center rounded-2xl border border-white/[0.06] bg-white/[0.03] text-neutral-500"><Bot size={20} strokeWidth={1.5} aria-hidden="true" /></div>
            <p className="text-[13px] font-medium text-neutral-300">No agents running yet</p>
            <p className="mt-1 text-[11.5px] leading-relaxed text-neutral-600">Start a chat or delegate work, and every live agent will appear here as its own window.</p>
          </div>
        </div>
      )}
    </div>
  );
}
