import { useCallback, useEffect, useState } from "react";
import { Check, Clock, ShieldCheck, X } from "lucide-react";
import { bridgeApi } from "../../api";
import type { WorkflowRun } from "../../protocol/generated/protocol";
import { Button } from "@/components/ui/button";
import { RUN_STATUS_LABEL, relativeTime, runTone } from "./format";
import { runSteps, toolLabel } from "./WorkflowRunView";
import { Dot, ErrorNote, errorText } from "./parts";
import { RowButton } from "@/components/ui/row";
import { Skeleton, SkeletonRegion } from "@/components/ui/skeleton";

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
        <p className="text-ui font-semibold text-foreground">{workflowName} wants to change something</p>
        {pending ? <p className="mt-0.5 text-caption text-muted-foreground">{toolLabel(pending.identity)}</p> : <SkeletonRegion label="Loading the request" className="mt-1.5"><Skeleton className="h-2.5 w-40" /></SkeletonRegion>}
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
    <p className="mb-2 text-caption font-medium text-muted-foreground">Needs you</p>
    {waiting.length === 0 && <p className="mb-4 text-caption text-muted-foreground">Nothing is waiting for you.</p>}
    <div className="space-y-2">{waiting.map(run => run.status === "waiting_approval"
      ? <PendingCard key={run.id} run={run} workflowName={name(run)} onAnswered={onChanged}/>
      : <RowButton size="bare" tone="choice" shape="card" key={run.id} onClick={() => onOpenRun(run)}><span className="flex w-full items-center gap-2.5 p-3.5">
        <Clock size={14} className="shrink-0 text-muted-foreground"/>
        <span className="text-ui text-foreground">{name(run)} is waiting for quota</span>
        <span className="ml-auto text-caption text-muted-foreground">{relativeTime(run.createdAt, now)}</span>
      </span></RowButton>)}</div>
    <p className="mb-2 mt-6 text-caption font-medium text-muted-foreground">Results</p>
    {results.length === 0 && <p className="text-caption text-muted-foreground">No runs yet.</p>}
    {results.length > 0 && <div className="divide-y divide-border overflow-hidden rounded-xl border border-border bg-card">{results.map(run =>
      <RowButton size="bare" tone="default" shape="card" key={run.id} onClick={() => onOpenRun(run)}><span className="flex w-full items-start gap-3 px-4 py-3.5">
        <span className="mt-[5px] flex shrink-0"><Dot tone={runTone(run.status)}/></span>
        <div className="min-w-0 flex-1">
          <p className={`text-ui ${run.unread ? "font-semibold" : "font-medium"} text-foreground`}>{name(run)} · {RUN_STATUS_LABEL[run.status]}{run.unread && <span className="ml-2 rounded-full bg-muted px-1.5 py-px text-caption font-medium text-muted-foreground">New</span>}</p>
          <p className="mt-0.5 text-caption text-muted-foreground">{[run.triggerKind === "catch_up" ? `catch-up for ${run.missedSlots} missed slot${run.missedSlots === 1 ? "" : "s"}` : run.triggerKind === "manual" ? (run.dryRun ? "dry run" : "Run now") : run.triggerKind === "event" ? ((run.eventCount ?? 0) > 1 ? `${run.eventCount} events` : run.event?.title ?? "event") : "schedule", relativeTime(run.finishedAt ?? run.createdAt, now), run.tokens ? `${run.tokens.toLocaleString()} tokens` : null].filter(Boolean).join(" · ")}</p>
          {run.summary && <p className="mt-1.5 line-clamp-2 text-meta leading-relaxed text-muted-foreground">{run.summary}</p>}
        </div>
      </span></RowButton>)}</div>}
  </div>;
}

/** Mission Control's strip of active workflow runs. */
export const activeWorkflowRuns = (runs: WorkflowRun[]) => runs.filter(run => ["queued", "running", "waiting_approval", "waiting_quota"].includes(run.status));

/** `chips` sits inline in a toolbar; `card` is its own bordered row. */
export function WorkflowRunsStrip({ runs, names, onOpenRun, variant = "card" }: { runs: WorkflowRun[]; names: Map<string, string>; onOpenRun: (run: WorkflowRun) => void; variant?: "card" | "chips" }) {
  const active = activeWorkflowRuns(runs);
  if (active.length === 0) return null;
  return <div role="group" aria-label="Workflows running" className={variant === "chips"
    ? "flex min-w-0 items-center gap-1.5 overflow-hidden"
    : "mb-3 flex flex-wrap items-center gap-2 rounded-xl border border-border bg-card px-3 py-2"}>
    <span className={variant === "chips" ? "sr-only" : "text-caption font-medium text-muted-foreground"}>Workflows</span>
    {active.map(run => <Button variant="outline" size="2xs" className="max-w-48 shrink-0" key={run.id} onClick={() => onOpenRun(run)}>
      <Dot tone={runTone(run.status)}/>{names.get(run.workflowId) ?? "Workflow"} · {RUN_STATUS_LABEL[run.status]}
    </Button>)}
  </div>;
}
