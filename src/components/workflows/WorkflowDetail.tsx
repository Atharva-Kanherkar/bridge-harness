import { ArrowLeft, Pause, Pencil, Play, ShieldCheck, Trash2 } from "lucide-react";
import type { Project, Workflow, WorkflowRun } from "../../protocol/generated/protocol";
import { Button } from "@/components/ui/button";
import { ScreenHeading } from "../ui/screen";
import { RUN_STATUS_LABEL, relativeTime, runTone, triggerSummary } from "./format";
import { CHIP, Card, Dot, Section, StatusChip } from "./parts";

function duration(run: WorkflowRun): string {
  if (!run.startedAt || !run.finishedAt) return "";
  const seconds = Math.max(0, Math.round((Date.parse(run.finishedAt) - Date.parse(run.startedAt)) / 1000));
  return seconds < 60 ? `${seconds}s` : `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
}

export function WorkflowDetail({ workflow, runs, projects, nextRunAt, busy, onBack, onRun, onToggle, onApprove, onEdit, onDelete, onOpenRun }: {
  workflow: Workflow;
  runs: WorkflowRun[];
  projects: Project[];
  nextRunAt: string | null;
  busy: boolean;
  onBack: () => void;
  onRun: (dryRun: boolean) => void;
  onToggle: () => void;
  onApprove: () => void;
  onEdit: () => void;
  onDelete: () => void;
  onOpenRun: (run: WorkflowRun) => void;
}) {
  const definition = workflow.definition;
  const step = definition.steps[0];
  const project = definition.projectId ? projects.find(candidate => candidate.id === definition.projectId)?.name ?? "missing project" : "No project";
  const now = Date.now();
  const status = !workflow.approved ? { label: "needs approval", tone: "warn" as const }
    : !workflow.enabled ? { label: workflow.disabledReason === "project_removed" ? "project gone" : "paused", tone: "warn" as const }
    : { label: "active", tone: "ok" as const };
  const next = relativeTime(nextRunAt, now);
  const budget = definition.budget;

  return <div>
    <button type="button" onClick={onBack} className="mb-3 inline-flex items-center gap-1 text-caption text-muted-foreground hover:text-foreground"><ArrowLeft size={12}/>Workflows</button>
    <ScreenHeading title={definition.name}
      description={<span className="inline-flex flex-wrap items-center gap-2"><StatusChip label={status.label} tone={status.tone}/><span>{[project, `${step.harness} · ${step.model}`, next ? `next run ${next}` : null].filter(Boolean).join(" · ")}</span></span>}
      action={<>
        {!workflow.approved && <Button size="xs" disabled={busy} onClick={onApprove}><ShieldCheck size={11}/>Approve</Button>}
        <Button size="xs" variant={workflow.approved ? "default" : "secondary"} disabled={busy || !workflow.approved || !workflow.enabled} onClick={() => onRun(false)}><Play size={11}/>Run now</Button>
        <Button size="xs" variant="secondary" disabled={busy || !workflow.enabled} onClick={() => onRun(true)}>Dry run</Button>
        <Button size="xs" variant="secondary" disabled={busy} onClick={onToggle}>{workflow.enabled ? <><Pause size={11}/>Pause</> : <><Play size={11}/>Resume</>}</Button>
        <Button size="xs" variant="ghost" disabled={busy} onClick={onEdit}><Pencil size={11}/>Edit</Button>
        <Button size="xs" variant="ghost" disabled={busy} onClick={onDelete} aria-label="Delete workflow"><Trash2 size={11}/></Button>
      </>}/>
    <div className="grid gap-5 lg:grid-cols-[1fr_300px]">
      <Card>
        <Section title="Instructions"><p className="whitespace-pre-wrap text-[13px] leading-relaxed text-foreground">{step.instructions}</p></Section>
        {definition.expectedResult && <Section title="Expected result"><p className="text-[12px] text-muted-foreground">{definition.expectedResult}</p></Section>}
        <Section title="Last runs">
          {runs.length === 0 && <p className="text-caption text-muted-foreground">No runs yet.</p>}
          <div className="divide-y divide-border">{runs.map(run => <button type="button" key={run.id} onClick={() => onOpenRun(run)} className="flex w-full items-center gap-3 py-2 text-left text-[12px] hover:bg-accent">
            <Dot tone={runTone(run.status)}/>
            <span className="w-32 text-foreground">{RUN_STATUS_LABEL[run.status]}</span>
            <span className="w-24 text-muted-foreground">{relativeTime(run.createdAt, now)}</span>
            <span className="flex-1 text-muted-foreground">{run.triggerKind === "catch_up" ? "catch-up" : run.triggerKind === "manual" ? (run.dryRun ? "dry run" : "Run now") : "schedule"}</span>
            <span className="w-14 text-right text-muted-foreground">{duration(run)}</span>
            <span className="w-20 text-right font-mono text-muted-foreground">{run.tokens ? `${run.tokens.toLocaleString()} tok` : ""}</span>
          </button>)}</div>
        </Section>
      </Card>
      <div className="space-y-4">
        <Card><Section title="Trigger">
          <p className="text-[13px] text-foreground">{triggerSummary(definition.trigger)}</p>
          {definition.trigger.kind === "schedule" && <p className="mt-0.5 font-mono text-caption text-muted-foreground">{definition.trigger.cron} · {definition.trigger.timezone}</p>}
        </Section></Card>
        <Card><Section title="Can use">
          {definition.files === "worktree" && <p className="text-[12px] text-foreground">Its own worktree of {project}</p>}
          {(definition.grants ?? []).length === 0 && definition.files === "none" && <p className="text-caption text-muted-foreground">No connectors. It can only answer from what it knows.</p>}
          {(definition.grants ?? []).map(grant => <div key={grant.server} className="flex flex-wrap items-center gap-2 py-1 text-[12px]">
            <span className="font-mono text-foreground">{grant.server}</span>
            {grant.read && <span className={CHIP}>read</span>}
            {grant.write === "ask" && <span className={CHIP}>write · asks</span>}
          </div>)}
          {(definition.alwaysAllow ?? []).length > 0 && <p className="mt-2 text-caption text-muted-foreground">Always allowed: {(definition.alwaysAllow ?? []).map(identity => identity.split("__").pop()).join(", ")}</p>}
        </Section></Card>
        <Card><Section title="Budget">
          <p className="text-[12px] text-muted-foreground">{Math.round(budget.maxWallSeconds / 60)} min · {budget.maxToolCalls} tool calls per run</p>
        </Section></Card>
      </div>
    </div>
  </div>;
}
