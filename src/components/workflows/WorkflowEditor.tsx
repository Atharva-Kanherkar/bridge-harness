import { useMemo, useState } from "react";
import { LoaderCircle, ShieldCheck } from "lucide-react";
import type { Project, Workflow, WorkflowDefinition, WorkflowGrant, WorkflowOptions } from "../../protocol/generated/protocol";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogPanel, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { ErrorNote } from "./parts";
import { localTimezone, presetCron, presetFor, WEEKDAY_OPTIONS, type SchedulePreset } from "./format";

const FIELD = "block text-caption font-medium text-muted-foreground";
const SELECT = "mt-1 h-8 w-full rounded-lg border border-input bg-background dark:bg-input/32 px-2 text-[13px] text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring";

export type EditorSubmit = { definition: WorkflowDefinition; approve: boolean; dryRun: boolean };

function emptyDefinition(options: WorkflowOptions | null): WorkflowDefinition {
  const claude = options?.harnesses.find(harness => harness.id === "claude" && harness.connectorsAllowed) ?? options?.harnesses.find(harness => harness.connectorsAllowed);
  return {
    name: "",
    projectId: null,
    steps: [{ harness: claude?.id ?? "claude", model: claude?.defaultModel ?? claude?.models[0]?.id ?? "", instructions: "" }],
    trigger: { kind: "manual" },
    grants: [],
    alwaysAllow: [],
    files: "none",
    budget: { maxWallSeconds: 600, maxTurns: 24, maxToolCalls: 64, maxOutputTokens: null, costCeilingMicrousd: null },
    expectedResult: "",
    notify: { onSuccess: false },
  };
}

/** Why a harness cannot run this shape of workflow, or null. */
export function harnessRefusal(options: WorkflowOptions | null, harnessId: string, definition: Pick<WorkflowDefinition, "files" | "grants">): string | null {
  const harness = options?.harnesses.find(candidate => candidate.id === harnessId);
  if (!harness) return "not registered";
  const needsConnectorAuthority = (definition.grants ?? []).length > 0 || definition.files === "none";
  if (needsConnectorAuthority && !harness.connectorsAllowed) return harness.connectorsReason ?? "cannot limit a run to single connector tools";
  if (definition.files === "worktree" && !harness.worktreeAllowed) return "cannot run worktree jobs";
  return null;
}

export function WorkflowEditor({ open, workflow, initial, options, projects, busy, error, onCancel, onSubmit }: {
  open: boolean;
  /** The workflow being edited, or null to create one. */
  workflow: Workflow | null;
  /** A draft to start from (P1.5), when not editing. */
  initial?: WorkflowDefinition | null;
  options: WorkflowOptions | null;
  projects: Project[];
  busy: boolean;
  error: string | null;
  onCancel: () => void;
  onSubmit: (submit: EditorSubmit) => void;
}) {
  const start = workflow?.definition ?? initial ?? emptyDefinition(options);
  const [definition, setDefinition] = useState<WorkflowDefinition>(start);
  const schedule = presetFor(start.trigger);
  const [preset, setPreset] = useState<SchedulePreset>(schedule.preset);
  const [time, setTime] = useState(schedule.time);
  const [weekday, setWeekday] = useState(schedule.weekday);
  const [custom, setCustom] = useState(schedule.custom);
  const [timezone, setTimezone] = useState(start.trigger.kind === "schedule" ? start.trigger.timezone : localTimezone());
  const step = definition.steps[0];
  const grants = definition.grants ?? [];
  const harness = options?.harnesses.find(candidate => candidate.id === step.harness);
  const update = (change: Partial<WorkflowDefinition>) => setDefinition(current => ({ ...current, ...change }));
  const updateStep = (change: Partial<typeof step>) => setDefinition(current => ({ ...current, steps: [{ ...current.steps[0], ...change }] }));

  const composed = useMemo<WorkflowDefinition>(() => {
    const cron = presetCron(preset, time, weekday, custom);
    return { ...definition, name: definition.name.trim(), trigger: cron === null ? { kind: "manual" } : { kind: "schedule", cron, timezone } };
  }, [definition, preset, time, weekday, custom, timezone]);

  const grantFor = (server: string): WorkflowGrant | undefined => grants.find(grant => grant.server === server);
  const setGrant = (server: string, next: WorkflowGrant | null) => {
    const rest = grants.filter(grant => grant.server !== server);
    update({ grants: next ? [...rest, next] : rest });
  };
  const refusal = harnessRefusal(options, step.harness, composed);
  const ready = composed.name.length > 0 && step.instructions.trim().length > 0 && step.model.trim().length > 0 && !refusal;

  return <Dialog open={open} onOpenChange={next => { if (!next && !busy) onCancel(); }}>
    {open && <DialogContent className="sm:max-w-2xl" showCloseButton={!busy}>
      <DialogHeader>
        <DialogTitle>{workflow ? "Edit workflow" : "New workflow"}</DialogTitle>
        <DialogDescription>Check what it may use before you approve it. A change that lets it touch more asks for approval again.</DialogDescription>
      </DialogHeader>
      <DialogPanel className="min-w-0 space-y-3">
        <label className={FIELD}>Name<Input aria-label="Workflow name" className="mt-1" value={definition.name} onChange={event => update({ name: event.target.value })}/></label>

        <div className="grid gap-3 sm:grid-cols-2">
          <label className={FIELD}>Project
            <select aria-label="Workflow project" className={SELECT} value={definition.projectId ?? ""} onChange={event => {
              const projectId = event.target.value || null;
              update({ projectId, files: projectId ? definition.files : "none" });
            }}>
              <option value="">No project (runs in a private folder)</option>
              {projects.map(project => <option key={project.id} value={project.id}>{project.name}</option>)}
            </select>
          </label>
          <label className={FIELD}>When
            <select aria-label="Workflow schedule" className={SELECT} value={preset} onChange={event => setPreset(event.target.value as SchedulePreset)}>
              <option value="manual">Only when I press Run now</option>
              <option value="daily">Every day</option>
              <option value="weekdays">Weekdays</option>
              <option value="weekly">Once a week</option>
              <option value="hourly">Every hour</option>
              <option value="custom">Custom (cron)</option>
            </select>
          </label>
        </div>
        {preset !== "manual" && <div className="grid gap-3 sm:grid-cols-3">
          {preset === "custom"
            ? <label className={`${FIELD} sm:col-span-2`}>Cron (minute hour day month weekday)<Input aria-label="Cron expression" className="mt-1 font-mono" value={custom} onChange={event => setCustom(event.target.value)}/></label>
            : <>
              {preset === "weekly" && <label className={FIELD}>Day<select aria-label="Weekday" className={SELECT} value={weekday} onChange={event => setWeekday(Number(event.target.value))}>{WEEKDAY_OPTIONS.map(day => <option key={day.value} value={day.value}>{day.label}</option>)}</select></label>}
              <label className={FIELD}>{preset === "hourly" ? "Minute past the hour" : "Time"}<Input aria-label="Schedule time" type="time" className="mt-1" value={time} onChange={event => setTime(event.target.value)}/></label>
            </>}
          <label className={FIELD}>Timezone<Input aria-label="Timezone" className="mt-1" value={timezone} onChange={event => setTimezone(event.target.value)}/></label>
        </div>}

        <div className={FIELD}>Agent
          <div className="mt-1 divide-y divide-border overflow-hidden rounded-lg border border-border">
            {(options?.harnesses ?? []).map(candidate => {
              const reason = harnessRefusal(options, candidate.id, composed);
              const selected = candidate.id === step.harness;
              return <label key={candidate.id} title={reason ?? undefined} className={`flex min-w-0 items-center gap-2 px-2.5 py-1.5 text-[13px] ${selected ? "bg-accent text-foreground" : reason ? "text-muted-foreground opacity-70" : "text-foreground"}`}>
                <input type="radio" name="workflow-harness" aria-label={candidate.label} disabled={!!reason || !candidate.available} checked={selected} onChange={() => updateStep({ harness: candidate.id, model: candidate.defaultModel ?? candidate.models[0]?.id ?? "" })} className="accent-foreground"/>
                <span className="shrink-0">{candidate.label}</span>
                <span className="ml-auto min-w-0 line-clamp-2 text-right text-caption text-muted-foreground">{reason ?? (candidate.available ? "" : "not installed")}</span>
              </label>;
            })}
          </div>
        </div>
        <label className={FIELD}>Model
          {harness && harness.models.length > 0
            ? <select aria-label="Workflow model" className={SELECT} value={step.model} onChange={event => updateStep({ model: event.target.value })}>{harness.models.map(model => <option key={model.id} value={model.id}>{model.label}</option>)}</select>
            : <Input aria-label="Workflow model" className="mt-1" value={step.model} onChange={event => updateStep({ model: event.target.value })}/>}
        </label>

        <label className={FIELD}>Instructions<Textarea aria-label="Workflow instructions" className="mt-1" value={step.instructions} onChange={event => updateStep({ instructions: event.target.value })}/></label>
        <label className={FIELD}>What a finished run looks like<Input aria-label="Expected result" className="mt-1" value={definition.expectedResult ?? ""} onChange={event => update({ expectedResult: event.target.value })}/></label>

        {definition.projectId && <label className="flex items-center gap-2 text-caption text-muted-foreground">
          <input type="checkbox" aria-label="Work in a worktree" checked={definition.files === "worktree"} onChange={event => update({ files: event.target.checked ? "worktree" : "none", grants: event.target.checked ? [] : grants })} className="size-3.5 accent-foreground"/>
          Work in its own copy of the project (a worktree). Connectors are off for worktree jobs for now.
        </label>}

        {definition.files === "none" && <div className={FIELD}>Can use
          <div className="mt-1 divide-y divide-border rounded-lg border border-border">
            {(options?.servers ?? []).length === 0 && <p className="px-2.5 py-2 text-caption text-muted-foreground">No connectors found in Claude's configuration.</p>}
            {(options?.servers ?? []).map(server => {
              const grant = grantFor(server.id);
              return <div key={server.id} className="flex min-w-0 flex-wrap items-center gap-3 px-2.5 py-1.5 text-[12px] text-foreground">
                <span className="min-w-0 flex-1 break-all font-mono">{server.id}{server.connected === false && <span className="ml-2 font-sans text-caption text-muted-foreground">not connected</span>}</span>
                <label className="flex items-center gap-1.5 text-muted-foreground"><input type="checkbox" aria-label={`Read ${server.id}`} checked={grant?.read ?? false} onChange={event => {
                  const read = event.target.checked;
                  const write = grant?.write ?? "never";
                  setGrant(server.id, read || write === "ask" ? { server: server.id, read, write } : null);
                }} className="size-3.5 accent-foreground"/>read</label>
                <label className="flex items-center gap-1.5 text-muted-foreground"><input type="checkbox" aria-label={`Write ${server.id}`} checked={grant?.write === "ask"} onChange={event => {
                  const write = event.target.checked ? "ask" : "never";
                  const read = grant?.read ?? false;
                  setGrant(server.id, read || write === "ask" ? { server: server.id, read, write } : null);
                }} className="size-3.5 accent-foreground"/>write, asking every time</label>
              </div>;
            })}
          </div>
        </div>}

        <div className="grid gap-3 sm:grid-cols-3">
          <label className={FIELD}>Time limit (minutes)<Input aria-label="Time limit" type="number" min={1} max={60} className="mt-1" value={Math.round(definition.budget.maxWallSeconds / 60)} onChange={event => update({ budget: { ...definition.budget, maxWallSeconds: Math.max(1, Number(event.target.value) || 1) * 60 } })}/></label>
          <label className={FIELD}>Tool calls<Input aria-label="Tool call limit" type="number" min={1} max={256} className="mt-1" value={definition.budget.maxToolCalls} onChange={event => update({ budget: { ...definition.budget, maxToolCalls: Number(event.target.value) || 1 } })}/></label>
          <label className="flex items-end gap-2 pb-1.5 text-caption text-muted-foreground"><input type="checkbox" aria-label="Notify on success" checked={definition.notify?.onSuccess ?? false} onChange={event => update({ notify: { onSuccess: event.target.checked } })} className="size-3.5 accent-foreground"/>Notify me on success too</label>
        </div>
        {refusal && <p className="text-caption text-muted-foreground">This agent cannot run this workflow: {refusal}.</p>}
        <ErrorNote message={error}/>
      </DialogPanel>
      <DialogFooter>
        <Button variant="ghost" disabled={busy} onClick={onCancel}>Cancel</Button>
        <Button variant="secondary" disabled={busy || !ready} onClick={() => onSubmit({ definition: composed, approve: false, dryRun: true })}>Dry run first</Button>
        <Button disabled={busy || !ready} onClick={() => onSubmit({ definition: composed, approve: true, dryRun: false })}>{busy ? <LoaderCircle className="animate-spin" size={12}/> : <ShieldCheck size={12}/>}Approve and save</Button>
      </DialogFooter>
    </DialogContent>}
  </Dialog>;
}
