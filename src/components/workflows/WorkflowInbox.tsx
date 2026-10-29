import { useCallback, useEffect, useState } from "react";
import { Check, Clock, ShieldCheck, X } from "lucide-react";
import { bridgeApi } from "../../api";
import type { WorkflowRun } from "../../protocol/generated/protocol";
import { Button } from "@/components/ui/button";
import { RUN_STATUS_LABEL, relativeTime, runTone } from "./format";
import { runSteps, toolLabel } from "./WorkflowRunView";
import { Dot, ErrorNote, errorText } from "./parts";

/** Results newest first; among runs of the same moment, unread first. */
export function orderResults(runs: WorkflowRun[]): WorkflowRun[] {
  return [...runs].sort((a, b) => b.createdAt.localeCompare(a.createdAt) || Number(b.unread) - Number(a.unread));
}

/** The pending write card of a run waiting for approval, answerable in place. */
function PendingCard({ run, workflowName, onAnswered }: { run: WorkflowRun; workflowName: string; onAnswered: () => void }) {
  const [pending, setPending] = useState<{ sequence: number; identity: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (!run.sessionId) return;
    bridgeApi.replaySessionEvents(run.sessionId, 0, 200, true).then(events => {
      const step = runSteps(events).reverse().find(candidate => candidate.pending);
      setPending(step?.pending ? { sequence: step.pending.sequence, identity: step.identity ?? step.title } : null);
    }).catch(reason => setError(errorText(reason)));
  }, [run.sessionId, run.status]);
  const answer = async (work: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await work();
      onAnswered();
    } catch (reason) {
      setError(errorText(reason));
    } finally {
      setBusy(false);
    }
  };
  return <div className="rounded-xl border border-border bg-card p-3.5">
    <div className="flex items-start gap-2.5">
      <ShieldCheck size={15} className="mt-0.5 shrink-0 text-foreground"/>
      <div className="min-w-0 flex-1">
        <p className="text-[13px] font-semibold text-foreground">{workflowName} wants to change something</p>
        <p className="mt-0.5 text-caption text-muted-foreground">{pending ? toolLabel(pending.identity) : "Loading the request…"}</p>
        {pending && run.sessionId && <div className="mt-3 flex flex-wrap gap-1.5">
          <Button size="xs" disabled={busy} onClick={() => void answer(() => bridgeApi.resolveApproval(run.sessionId!, pending.sequence, "accept"))}><Check size={10}/>Allow once</Button>
          <Button size="xs" variant="secondary" disabled={busy} onClick={() => void answer(() => bridgeApi.alwaysAllowWorkflowTool(run.workflowId, run.sessionId!, pending.sequence, pending.identity))}>Always allow for this workflow</Button>
          <Button size="xs" variant="ghost" disabled={busy} onClick={() => void answer(() => bridgeApi.resolveApproval(run.sessionId!, pending.sequence, "decline"))}><X size={10}/>Decline</Button>
        </div>}
        <ErrorNote message={error}/>
      </div>
    </div>
  </div>;
}

export function WorkflowInbox({ runs, names, onOpenRun, onChanged }: {
  runs: WorkflowRun[];
  /** Workflow id to name. */
  names: Map<string, string>;
  onOpenRun: (run: WorkflowRun) => void;
  onChanged: () => void;
}) {
  const now = Date.now();
  const name = useCallback((run: WorkflowRun) => names.get(run.workflowId) ?? "A workflow", [names]);
  const waiting = runs.filter(run => run.status === "waiting_approval" || run.status === "waiting_quota");
  const results = orderResults(runs.filter(run => !["waiting_approval", "waiting_quota", "queued", "running"].includes(run.status)));
  return <div className="mt-4">
    <p className="mb-2 text-caption font-medium uppercase tracking-[0.06em] text-muted-foreground">Needs you</p>
    {waiting.length === 0 && <p className="mb-4 text-caption text-muted-foreground">Nothing is waiting for you.</p>}
    <div className="space-y-2">{waiting.map(run => run.status === "waiting_approval"
      ? <PendingCard key={run.id} run={run} workflowName={name(run)} onAnswered={onChanged}/>
      : <button type="button" key={run.id} onClick={() => onOpenRun(run)} className="flex w-full items-center gap-2.5 rounded-xl border border-border bg-card p-3.5 text-left hover:bg-accent">
        <Clock size={14} className="shrink-0 text-muted-foreground"/>
        <span className="text-[13px] text-foreground">{name(run)} is waiting for quota</span>
        <span className="ml-auto text-caption text-muted-foreground">{relativeTime(run.createdAt, now)}</span>
      </button>)}</div>
    <p className="mb-2 mt-6 text-caption font-medium uppercase tracking-[0.06em] text-muted-foreground">Results</p>
    {results.length === 0 && <p className="text-caption text-muted-foreground">No runs yet.</p>}
    {results.length > 0 && <div className="divide-y divide-border overflow-hidden rounded-xl border border-border bg-card">{results.map(run =>
      <button type="button" key={run.id} onClick={() => onOpenRun(run)} className="flex w-full items-start gap-3 px-4 py-3.5 text-left hover:bg-accent">
        <span className="mt-[5px] flex shrink-0"><Dot tone={runTone(run.status)}/></span>
        <div className="min-w-0 flex-1">
          <p className={`text-[13px] ${run.unread ? "font-semibold" : "font-medium"} text-foreground`}>{name(run)} · {RUN_STATUS_LABEL[run.status]}{run.unread && <span className="ml-2 rounded-full bg-muted px-1.5 py-px text-[10px] font-medium uppercase tracking-[0.06em] text-muted-foreground">New</span>}</p>
          <p className="mt-0.5 text-caption text-muted-foreground">{[run.triggerKind === "catch_up" ? `catch-up for ${run.missedSlots} missed slot${run.missedSlots === 1 ? "" : "s"}` : run.triggerKind === "manual" ? (run.dryRun ? "dry run" : "Run now") : "schedule", relativeTime(run.finishedAt ?? run.createdAt, now), run.tokens ? `${run.tokens.toLocaleString()} tokens` : null].filter(Boolean).join(" · ")}</p>
          {run.summary && <p className="mt-1.5 line-clamp-2 text-[12px] leading-relaxed text-muted-foreground">{run.summary}</p>}
        </div>
      </button>)}</div>}
  </div>;
}

/** Mission Control's strip of active workflow runs. */
export function WorkflowRunsStrip({ runs, names, onOpenRun }: { runs: WorkflowRun[]; names: Map<string, string>; onOpenRun: (run: WorkflowRun) => void }) {
  const active = runs.filter(run => ["queued", "running", "waiting_approval", "waiting_quota"].includes(run.status));
  if (active.length === 0) return null;
  return <div aria-label="Workflows running" className="mb-3 flex flex-wrap items-center gap-2 rounded-xl border border-border bg-card px-3 py-2">
    <span className="text-caption font-medium uppercase tracking-[0.06em] text-muted-foreground">Workflows</span>
    {active.map(run => <button type="button" key={run.id} onClick={() => onOpenRun(run)} className="inline-flex items-center gap-1.5 rounded-full border border-border bg-muted px-2 py-0.5 text-caption text-foreground hover:bg-accent">
      <Dot tone={runTone(run.status)}/>{names.get(run.workflowId) ?? "Workflow"} · {RUN_STATUS_LABEL[run.status]}
    </button>)}
  </div>;
}
