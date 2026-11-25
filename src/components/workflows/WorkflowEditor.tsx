import { useEffect, useMemo, useState } from "react";
import { LoaderCircle, Plus, RefreshCw, ShieldCheck, X } from "lucide-react";
import { bridgeApi } from "../../api";
import type { Project, Workflow, WorkflowDefinition, WorkflowEventKind, WorkflowEventSample, WorkflowFiles, WorkflowGrant, WorkflowOptions, WorkflowStep, WorkflowTrigger, WorkflowTriggerPreview } from "../../protocol/generated/protocol";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogPanel, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { ErrorNote } from "./parts";
import { EVENT_APPS, EVENT_OPTIONS, eventFilters, localTimezone, presetCron, presetFor, presetForCron, WEEKDAY_OPTIONS, type SchedulePreset } from "./format";

const FIELD = "block text-caption font-medium text-muted-foreground";
const SELECT = "mt-1 h-8 w-full rounded-lg border border-input bg-background dark:bg-input/32 px-2 text-ui text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring";

export type EditorSubmit = { definition: WorkflowDefinition; approve: boolean; dryRun: boolean; event?: WorkflowEventSample | null };

/** The event trigger the editor's fields describe. */
export function composeEventTrigger(fields: { event: WorkflowEventKind; repo: string; channel: string; from: string; collect: boolean; batchCron: string | null; timezone: string }): WorkflowTrigger {
  const filters = eventFilters(fields.event);
  const text = (value: string) => value.trim() || null;
  return {
    kind: "event",
    event: fields.event,
    repo: filters.repo ? text(fields.repo) : null,
    channel: filters.channel ? text(fields.channel) : null,
    from: filters.from ? text(fields.from) : null,
    batch: fields.collect && fields.batchCron ? { cron: fields.batchCron, timezone: fields.timezone } : null,
  };
}

/** "Would have run 4 times in the last 7 days." In batch mode the count is of
 * events, which are collected into one run per slot, not of runs. */
export function previewSentence(preview: WorkflowTriggerPreview, collected = false): string {
  if (!preview.supported) return preview.reason ?? "Bridge cannot read this yet.";
  const atLeast = preview.note?.startsWith("At least") ? "at least " : "";
  const days = `in the last ${preview.windowDays} days`;
  if (collected) {
    if (preview.matches === 0) return `No matching events ${days}.`;
    const count = `${atLeast ? "At least " : ""}${preview.matches} matching event${preview.matches === 1 ? "" : "s"}`;
    return `${count} ${days}, collected into one run per scheduled time.`;
  }
  const times = preview.matches === 1 ? "once" : `${atLeast}${preview.matches} times`;
  return preview.matches === 0 ? `Would not have run ${days}.` : `Would have run ${times} ${days}.`;
}

/** The batch schedules the editor offers; a saved one keeps its own. */
type BatchPreset = "daily" | "weekdays" | "weekly" | "hourly" | "custom";
const BATCH_OPTIONS: Array<{ value: BatchPreset; label: string }> = [
  { value: "daily", label: "every day" },
  { value: "weekdays", label: "on weekdays" },
  { value: "weekly", label: "once a week" },
  { value: "hourly", label: "every hour" },
  { value: "custom", label: "on a cron schedule" },
];
function batchPresetFor(cron: string | undefined): BatchPreset {
  const preset = presetForCron(cron ?? "0 9 * * *").preset;
  return preset === "daily" || preset === "weekdays" || preset === "weekly" || preset === "hourly" ? preset : "custom";
}

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
  // Core refuses worktree jobs by the same gate as connectors: a worktree run
  // must still be kept from every connector. Its reason says why.
  if (definition.files === "worktree" && !harness.worktreeAllowed) return harness.connectorsReason ?? "cannot run worktree jobs";
  return null;
}

/** The most steps one workflow runs; the daemon holds the same limit. */
export const MAX_STEPS = 5;

/** The granted servers step `index` uses: its own list, or every grant. */
export function stepServers(definition: WorkflowDefinition, index: number): string[] {
  return definition.steps[index]?.servers ?? (definition.grants ?? []).map(grant => grant.server);
}

/** Which files step `index` works on: its own choice, or the workflow's. */
export function stepFiles(definition: WorkflowDefinition, index: number): WorkflowFiles {
  return definition.steps[index]?.files ?? definition.files;
}

/** Why `harnessId` cannot run step `index` as it is set up, or null. */
export function stepRefusal(options: WorkflowOptions | null, definition: WorkflowDefinition, index: number, harnessId = definition.steps[index]?.harness ?? ""): string | null {
  const servers = stepServers(definition, index);
  return harnessRefusal(options, harnessId, { files: stepFiles(definition, index), grants: (definition.grants ?? []).filter(grant => servers.includes(grant.server)) });
}

/** What the steps' shares of connectors and files leave wrong, or null. One
 * step is checked too, as the daemon checks it. */
export function stepsProblem(definition: WorkflowDefinition): string | null {
  const indexes = definition.steps.map((_, index) => index);
  const used = new Set(indexes.flatMap(index => stepServers(definition, index)));
  const unused = (definition.grants ?? []).find(grant => !used.has(grant.server));
  if (unused) return `No step uses ${unused.server}. Give it to a step, or remove it.`;
  const both = indexes.find(index => stepFiles(definition, index) === "worktree" && stepServers(definition, index).length > 0);
  if (both !== undefined) return `Step ${both + 1} works in the worktree, so it cannot use connectors.`;
  if (definition.files === "worktree" && !indexes.some(index => stepFiles(definition, index) === "worktree")) return "No step works in the worktree. Choose one, or turn the worktree off.";
  return null;
}

/** A connector's name inside `mcp__<server>__<tool>`. */
const wireServerName = (server: string) => server.replace(/[^A-Za-z0-9_-]/g, "_");

/**
 * The workflow left with its first step alone. That step keeps exactly what
 * it had, with no settings of its own, since one step has no controls for
 * them: in the worktree, the worktree and no connectors; otherwise the grants
 * it used and their always-allowed tools.
 */
export function asOneStep(definition: WorkflowDefinition): WorkflowDefinition {
  const files = stepFiles(definition, 0);
  const servers = files === "worktree" ? [] : stepServers(definition, 0);
  const grants = (definition.grants ?? []).filter(grant => servers.includes(grant.server));
  const wire = grants.map(grant => wireServerName(grant.server));
  const alwaysAllow = (definition.alwaysAllow ?? []).filter(identity => wire.some(name => identity.startsWith(`mcp__${name}__`)));
  return { ...definition, files, grants, alwaysAllow, steps: [{ ...definition.steps[0], servers: null, files: null }] };
}

/** A new step: Claude when it may run one, with nothing granted yet. */
function newStep(options: WorkflowOptions | null): WorkflowStep {
  const agent = options?.harnesses.find(harness => harness.id === "claude" && harness.connectorsAllowed) ?? options?.harnesses.find(harness => harness.connectorsAllowed);
  return { harness: agent?.id ?? "claude", model: agent?.defaultModel ?? agent?.models[0]?.id ?? "", instructions: "", servers: [], files: "none" };
}

/** Which of the granted connectors a step uses, and whether it works in the worktree. */
function StepShare({ definition, index, onChange }: { definition: WorkflowDefinition; index: number; onChange: (change: Partial<WorkflowStep>) => void }) {
  const label = `Step ${index + 1}`;
  const servers = stepServers(definition, index);
  const worktree = stepFiles(definition, index) === "worktree";
  const grants = definition.grants ?? [];
  if (grants.length === 0 && definition.files !== "worktree") return null;
  return <div className="flex flex-wrap items-center gap-x-4 gap-y-1.5 text-caption text-muted-foreground">
    {grants.length > 0 && <span>Uses</span>}
    {grants.map(grant => <label key={grant.server} className="flex min-w-0 items-center gap-1.5">
      <input type="checkbox" aria-label={`${label} uses ${grant.server}`} disabled={worktree} checked={!worktree && servers.includes(grant.server)} onChange={event => {
        const next = event.target.checked ? [...servers.filter(server => server !== grant.server), grant.server] : servers.filter(server => server !== grant.server);
        onChange({ servers: next });
      }} className="size-3.5 accent-foreground"/><span className="break-all font-mono">{grant.server}</span>
    </label>)}
    {definition.files === "worktree" && <label className="flex items-center gap-1.5">
      <input type="checkbox" aria-label={`${label} works in the worktree`} checked={worktree} onChange={event => onChange(event.target.checked ? { files: "worktree", servers: [] } : { files: "none" })} className="size-3.5 accent-foreground"/>Works in the worktree
    </label>}
  </div>;
}

/** Step 2 and later: its own agent, model, instructions and share. */
function StepCard({ definition, index, options, onChange, onRemove }: { definition: WorkflowDefinition; index: number; options: WorkflowOptions | null; onChange: (change: Partial<WorkflowStep>) => void; onRemove: () => void }) {
  const step = definition.steps[index];
  const label = `Step ${index + 1}`;
  const agent = options?.harnesses.find(candidate => candidate.id === step.harness);
  const refusal = stepRefusal(options, definition, index);
  return <section aria-label={label} className="min-w-0 space-y-2.5 rounded-xl border border-border px-3 py-2.5">
    <div className="flex items-center justify-between"><span className="text-caption font-medium text-foreground">{label}</span>
      <button type="button" aria-label={`Remove step ${index + 1}`} onClick={onRemove} className="inline-flex items-center gap-1 text-caption text-muted-foreground hover:text-foreground"><X size={11}/>Remove</button></div>
    <div className="grid gap-3 sm:grid-cols-2">
      <label className={FIELD}>Agent
        <select aria-label={`${label} agent`} className={SELECT} value={step.harness} onChange={event => {
          const next = options?.harnesses.find(candidate => candidate.id === event.target.value);
          onChange({ harness: event.target.value, model: next?.defaultModel ?? next?.models[0]?.id ?? "" });
        }}>
          {(options?.harnesses ?? []).map(candidate => {
            const reason = stepRefusal(options, definition, index, candidate.id) ?? (candidate.available ? null : "not installed");
            return <option key={candidate.id} value={candidate.id} disabled={Boolean(reason) && candidate.id !== step.harness} title={reason ?? undefined}>{candidate.label}{reason ? ` (${reason})` : ""}</option>;
          })}
        </select>
      </label>
      <label className={FIELD}>Model
        {agent && agent.models.length > 0
          ? <select aria-label={`${label} model`} className={SELECT} value={step.model} onChange={event => onChange({ model: event.target.value })}>{!agent.models.some(model => model.id === step.model) && <option value={step.model} disabled>{step.model || "Choose a model"}</option>}{agent.models.map(model => <option key={model.id} value={model.id}>{model.label}</option>)}</select>
          : <Input aria-label={`${label} model`} className="mt-1" value={step.model} onChange={event => onChange({ model: event.target.value })}/>}
      </label>
    </div>
    <label className={FIELD}>Instructions<Textarea aria-label={`${label} instructions`} className="mt-1" value={step.instructions} onChange={event => onChange({ instructions: event.target.value })}/></label>
    <StepShare definition={definition} index={index} onChange={onChange}/>
    {refusal && <p className="text-caption text-muted-foreground">{agent?.label ?? step.harness} cannot run this step: {refusal}.</p>}
  </section>;
}

export function WorkflowEditor({ open, workflow, initial, notes = [], options, projects, busy, error, onCancel, onSubmit, onRefreshConnectors, refreshing = false }: {
  open: boolean;
  /** The workflow being edited, or null to create one. */
  workflow: Workflow | null;
  /** A draft to start from (P1.5), when not editing. */
  initial?: WorkflowDefinition | null;
  /** What drafting could not do, shown above the form. */
  notes?: string[];
  options: WorkflowOptions | null;
  projects: Project[];
  busy: boolean;
  error: string | null;
  onCancel: () => void;
  onSubmit: (submit: EditorSubmit) => void;
  /** Ask Claude afresh which connectors it has. */
  onRefreshConnectors?: () => void;
  refreshing?: boolean;
}) {
  const start = workflow?.definition ?? initial ?? emptyDefinition(options);
  const [definition, setDefinition] = useState<WorkflowDefinition>(start);
  const schedule = presetFor(start.trigger);
  const [preset, setPreset] = useState<SchedulePreset>(schedule.preset);
  const [time, setTime] = useState(schedule.time);
  const [weekday, setWeekday] = useState(schedule.weekday);
  const [custom, setCustom] = useState(schedule.custom);
  const [timezone, setTimezone] = useState(start.trigger.kind === "schedule" ? start.trigger.timezone : start.trigger.kind === "event" && start.trigger.batch ? start.trigger.batch.timezone : localTimezone());
  const startEvent = start.trigger.kind === "event" ? start.trigger : null;
  const [eventKind, setEventKind] = useState<WorkflowEventKind>(startEvent?.event ?? "slack_mention");
  const [repo, setRepo] = useState(startEvent?.repo ?? "");
  const [channel, setChannel] = useState(startEvent?.channel ?? "");
  const [from, setFrom] = useState(startEvent?.from ?? "");
  const [collect, setCollect] = useState(Boolean(startEvent?.batch));
  // A saved batch keeps its own schedule until the person changes it (R10).
  const [batchPreset, setBatchPreset] = useState<BatchPreset>(batchPresetFor(startEvent?.batch?.cron));
  // A preview belongs to the trigger it answered (R11).
  const [preview, setPreview] = useState<{ key: string; result: WorkflowTriggerPreview } | null>(null);
  const [previewing, setPreviewing] = useState(false);
  const step = definition.steps[0];
  const grants = definition.grants ?? [];
  const multi = definition.steps.length > 1;
  const harness = options?.harnesses.find(candidate => candidate.id === step.harness);
  const update = (change: Partial<WorkflowDefinition>) => setDefinition(current => ({ ...current, ...change }));
  const updateStepAt = (index: number, change: Partial<WorkflowStep>) => setDefinition(current => ({ ...current, steps: current.steps.map((existing, at) => at === index ? { ...existing, ...change } : existing) }));
  const updateStep = (change: Partial<WorkflowStep>) => updateStepAt(0, change);
  const addStep = () => setDefinition(current => current.steps.length >= MAX_STEPS ? current : { ...current, steps: [...current.steps, newStep(options)] });
  const removeStep = (index: number) => setDefinition(current => {
    const next = { ...current, steps: current.steps.filter((_, at) => at !== index) };
    return next.steps.length === 1 ? asOneStep(next) : next;
  });

  const composed = useMemo<WorkflowDefinition>(() => {
    if (preset === "event") {
      const trigger = composeEventTrigger({ event: eventKind, repo, channel, from, collect, batchCron: presetCron(batchPreset, time, weekday, custom), timezone });
      return { ...definition, name: definition.name.trim(), trigger };
    }
    const cron = presetCron(preset, time, weekday, custom);
    return { ...definition, name: definition.name.trim(), trigger: cron === null ? { kind: "manual" } : { kind: "schedule", cron, timezone } };
  }, [definition, preset, time, weekday, custom, timezone, eventKind, repo, channel, from, collect, batchPreset]);

  // What this trigger would have done last week, refreshed as it is edited.
  const triggerKey = JSON.stringify(composed.trigger);
  useEffect(() => {
    if (!open || composed.trigger.kind !== "event") { setPreview(null); return; }
    let current = true;
    setPreviewing(true);
    const key = triggerKey;
    const timer = window.setTimeout(() => {
      bridgeApi.previewWorkflowTrigger(composed.trigger)
        .then(result => { if (current) setPreview({ key, result }); })
        .catch(reason => { if (current) setPreview({ key, result: { supported: false, reason: String(reason?.message ?? reason), windowDays: 7, matches: 0, samples: [], note: null } }); })
        .finally(() => { if (current) setPreviewing(false); });
    }, 400);
    return () => { current = false; window.clearTimeout(timer); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, triggerKey]);

  // Options can arrive after the editor opened (a cold start): take their
  // default model instead of keeping an empty one the select would hide.
  useEffect(() => {
    if (!options || step.model.trim()) return;
    const candidate = options.harnesses.find(entry => entry.id === step.harness);
    const model = candidate?.defaultModel ?? candidate?.models[0]?.id;
    if (model) updateStepAt(0, { model });
  }, [options, step.harness, step.model]);

  const grantFor = (server: string): WorkflowGrant | undefined => grants.find(grant => grant.server === server);
  const setGrant = (server: string, next: WorkflowGrant | null) => {
    const rest = grants.filter(grant => grant.server !== server);
    // A connector that is no longer granted leaves every step that used it.
    setDefinition(current => ({
      ...current,
      grants: next ? [...rest, next] : rest,
      steps: next ? current.steps : current.steps.map(existing => existing.servers ? { ...existing, servers: existing.servers.filter(name => name !== server) } : existing),
    }));
  };
  const refusal = stepRefusal(options, composed, 0);
  const anyRefused = composed.steps.some((_, index) => stepRefusal(options, composed, index) !== null);
  const problem = stepsProblem(composed);
  // A filter the event cannot run without: a GitHub repository, a Notion database.
  const filters = eventFilters(eventKind);
  const missingRepo = composed.trigger.kind === "event" && Boolean(filters.repo?.required) && !composed.trigger.repo;
  const missingChannel = composed.trigger.kind === "event" && Boolean(filters.channel?.required) && !composed.trigger.channel;
  const stepsFilled = composed.steps.every(entry => entry.instructions.trim().length > 0 && entry.model.trim().length > 0);
  const ready = composed.name.length > 0 && stepsFilled && !anyRefused && !problem && !missingRepo && !missingChannel;
  // Only the preview of the trigger as it is now: an older one's sample may be
  // another event entirely.
  const shown = preview?.key === triggerKey ? preview.result : null;
  const previewPending = composed.trigger.kind === "event" && !shown;
  // An event workflow's dry run is tried on the newest real event, when there is one.
  const tryOn = composed.trigger.kind === "event" ? shown?.samples[0] ?? null : null;

  return <Dialog open={open} onOpenChange={next => { if (!next && !busy) onCancel(); }}>
    {open && <DialogContent className="sm:max-w-2xl" showCloseButton={!busy}>
      <DialogHeader>
        <DialogTitle>{workflow ? "Edit workflow" : initial ? "Drafted workflow" : "New workflow"}</DialogTitle>
        <DialogDescription>Check what it may use before you approve it. A change that lets it touch more asks for approval again.</DialogDescription>
      </DialogHeader>
      <DialogPanel className="min-w-0 space-y-3">
        {notes.length > 0 && <ul aria-label="Draft notes" className="space-y-1 rounded-xl border border-border bg-muted px-3 py-2 text-caption text-muted-foreground">{notes.map(note => <li key={note}>{note}</li>)}</ul>}
        <label className={FIELD}>Name<Input aria-label="Workflow name" className="mt-1" value={definition.name} onChange={event => update({ name: event.target.value })}/></label>

        <div className="grid gap-3 sm:grid-cols-2">
          <label className={FIELD}>Project
            <select aria-label="Workflow project" className={SELECT} value={definition.projectId ?? ""} onChange={event => {
              const projectId = event.target.value || null;
              // No project, no worktree: every step goes back to the workflow's files.
              setDefinition(current => projectId ? { ...current, projectId } : { ...current, projectId, files: "none", steps: current.steps.map(existing => ({ ...existing, files: null })) });
            }}>
              <option value="">No project (runs in a private folder)</option>
              {projects.map(project => <option key={project.id} value={project.id}>{project.name}</option>)}
            </select>
          </label>
          <label className={FIELD}>When
            <select aria-label="Workflow schedule" className={SELECT} value={preset} onChange={event => setPreset(event.target.value as SchedulePreset)}>
              <option value="manual">Only when I press Run now</option>
              <option value="event">When something happens</option>
              <option value="daily">Every day</option>
              <option value="weekdays">Weekdays</option>
              <option value="weekly">Once a week</option>
              <option value="hourly">Every hour</option>
              <option value="custom">Custom (cron)</option>
            </select>
          </label>
        </div>
        {preset === "event" && <div className="space-y-3 rounded-xl border border-border px-3 py-2.5">
          <div className="grid gap-3 sm:grid-cols-2">
            <label className={FIELD}>When
              <select aria-label="Event" className={SELECT} value={eventKind} onChange={event => setEventKind(event.target.value as WorkflowEventKind)}>
                {EVENT_APPS.map(app => <optgroup key={app} label={app}>{EVENT_OPTIONS.filter(option => option.app === app).map(option => <option key={option.value} value={option.value}>{option.label}</option>)}</optgroup>)}
              </select>
            </label>
            {filters.repo && <label className={FIELD}>{filters.repo.label}{filters.repo.required ? "" : " (optional)"}<Input aria-label="Repository" className="mt-1 font-mono" placeholder={filters.repo.placeholder} value={repo} onChange={event => setRepo(event.target.value)}/></label>}
            {filters.channel && <label className={FIELD}>{filters.channel.label}{filters.channel.required ? "" : " (optional)"}<Input aria-label="Channel filter" className="mt-1" placeholder={filters.channel.placeholder} value={channel} onChange={event => setChannel(event.target.value)}/></label>}
          </div>
          {filters.from && <label className={FIELD}>{filters.from.label} (optional)<Input aria-label="Sender filter" className="mt-1" placeholder={filters.from.placeholder} value={from} onChange={event => setFrom(event.target.value)}/></label>}
          <div role="radiogroup" aria-label="How often" className="flex flex-wrap items-center gap-x-4 gap-y-2 text-caption text-muted-foreground">
            <label className="flex items-center gap-1.5"><input type="radio" name="event-mode" aria-label="Run for each event" checked={!collect} onChange={() => setCollect(false)} className="accent-foreground"/>Run for each event</label>
            <label className="flex items-center gap-1.5"><input type="radio" name="event-mode" aria-label="Collect them and run at a set time" checked={collect} onChange={() => setCollect(true)} className="accent-foreground"/>Collect them and run</label>
            {collect && <>
              <select aria-label="Collect on" className="h-7 rounded-md border border-input bg-background px-1.5 text-caption text-foreground dark:bg-input/32" value={batchPreset} onChange={event => setBatchPreset(event.target.value as BatchPreset)}>{BATCH_OPTIONS.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}</select>
              {batchPreset === "weekly" && <select aria-label="Collect weekday" className="h-7 rounded-md border border-input bg-background px-1.5 text-caption text-foreground dark:bg-input/32" value={weekday} onChange={event => setWeekday(Number(event.target.value))}>{WEEKDAY_OPTIONS.map(day => <option key={day.value} value={day.value}>on {day.label}</option>)}</select>}
              {batchPreset === "custom"
                ? <Input aria-label="Collect cron" className="h-7 w-40 font-mono" placeholder="minute hour day month weekday" value={custom} onChange={event => setCustom(event.target.value)}/>
                : <><span>{batchPreset === "hourly" ? "at minute" : "at"}</span>
                  <Input aria-label="Collect at" type="time" className="h-7 w-28" value={time} onChange={event => setTime(event.target.value)}/></>}
            </>}
          </div>
          <div aria-live="polite" className="rounded-lg bg-muted px-2.5 py-2 text-caption text-muted-foreground">
            {!shown ? (previewing ? <span className="inline-flex items-center gap-1.5"><LoaderCircle className="animate-spin" size={11}/>Checking the last week…</span> : null)
              : <>
                <p className={shown.supported ? "text-foreground" : ""}>{previewSentence(shown, collect)}</p>
                {shown.note && <p className="mt-0.5">{shown.note}</p>}
                {shown.samples.length > 0 && <ul aria-label="Recent events" className="mt-1.5 space-y-0.5 contain-inline-size">{shown.samples.slice(0, 3).map(sample => <li key={sample.key} className="truncate">{sample.title}</li>)}</ul>}
              </>}
          </div>
        </div>}
        {preset !== "manual" && (preset !== "event" || collect) && <div className="grid gap-3 sm:grid-cols-3">
          {preset === "event" ? null : preset === "custom"
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
              const reason = stepRefusal(options, composed, 0, candidate.id);
              const selected = candidate.id === step.harness;
              return <label key={candidate.id} title={reason ?? undefined} className={`flex min-w-0 items-center gap-2 px-2.5 py-1.5 text-ui ${selected ? "bg-accent text-foreground" : reason ? "text-muted-foreground opacity-70" : "text-foreground"}`}>
                <input type="radio" name="workflow-harness" aria-label={candidate.label} disabled={!!reason || !candidate.available} checked={selected} onChange={() => updateStep({ harness: candidate.id, model: candidate.defaultModel ?? candidate.models[0]?.id ?? "" })} className="accent-foreground"/>
                <span className="shrink-0">{candidate.label}</span>
                <span className="ml-auto min-w-0 line-clamp-2 text-right text-caption text-muted-foreground">{reason ?? (candidate.available ? "" : "not installed")}</span>
              </label>;
            })}
          </div>
        </div>
        <label className={FIELD}>Model
          {harness && harness.models.length > 0
            ? <select aria-label="Workflow model" className={SELECT} value={step.model} onChange={event => updateStep({ model: event.target.value })}>{!harness.models.some(model => model.id === step.model) && <option value={step.model} disabled>{step.model || "Choose a model"}</option>}{harness.models.map(model => <option key={model.id} value={model.id}>{model.label}</option>)}</select>
            : <Input aria-label="Workflow model" className="mt-1" value={step.model} onChange={event => updateStep({ model: event.target.value })}/>}
        </label>

        <label className={FIELD}>{multi ? "Step 1 instructions" : "Instructions"}<Textarea aria-label="Workflow instructions" className="mt-1" value={step.instructions} onChange={event => updateStep({ instructions: event.target.value })}/></label>
        {multi && <StepShare definition={definition} index={0} onChange={updateStep}/>}
        {definition.steps.map((_, index) => index === 0 ? null : <StepCard key={index} definition={definition} index={index} options={options} onChange={change => updateStepAt(index, change)} onRemove={() => removeStep(index)}/>)}
        {definition.steps.length < MAX_STEPS && <button type="button" aria-label="Add a step" onClick={addStep} className="inline-flex items-center gap-1 text-caption text-muted-foreground hover:text-foreground"><Plus size={11}/>Add a step, with its own agent</button>}
        <label className={FIELD}>What a finished run looks like<Input aria-label="Expected result" className="mt-1" value={definition.expectedResult ?? ""} onChange={event => update({ expectedResult: event.target.value })}/></label>

        {definition.projectId && <label className="flex items-center gap-2 text-caption text-muted-foreground">
          <input type="checkbox" aria-label="Work in a worktree" checked={definition.files === "worktree"} onChange={event => {
            const on = event.target.checked;
            // One step: the worktree replaces connectors. Several: each step
            // chooses, and the ones in the worktree use no connectors.
            if (!multi) update({ files: on ? "worktree" : "none", grants: on ? [] : grants });
            else setDefinition(current => ({ ...current, files: on ? "worktree" : "none", steps: on ? current.steps : current.steps.map(existing => ({ ...existing, files: null })) }));
          }} className="size-3.5 accent-foreground"/>
          {multi ? "Work in its own copy of the project (a worktree). Steps that work there use no connectors." : "Work in its own copy of the project (a worktree). Connectors are off for worktree jobs for now."}
        </label>}

        {(definition.files === "none" || multi) && <div className={FIELD}><span className="flex items-center justify-between">Can use
          {onRefreshConnectors && <button type="button" onClick={onRefreshConnectors} disabled={refreshing} className="inline-flex items-center gap-1 font-normal text-muted-foreground hover:text-foreground disabled:opacity-60"><RefreshCw size={11} className={refreshing ? "animate-spin" : ""}/>Refresh</button>}</span>
          <div className="mt-1 divide-y divide-border rounded-lg border border-border">
            {options?.connectorsError
              ? <p className="px-2.5 py-2 text-caption text-muted-foreground">Bridge could not list Claude's connectors just now ({options.connectorsError}). Use Refresh to ask again.</p>
              : (options?.servers ?? []).length === 0 && <p className="px-2.5 py-2 text-caption text-muted-foreground">No connectors found in Claude's configuration.</p>}
            {(options?.servers ?? []).map(server => {
              const grant = grantFor(server.id);
              return <div key={server.id} className="flex min-w-0 flex-wrap items-center gap-3 px-2.5 py-1.5 text-meta text-foreground">
                <span className="min-w-0 flex-1 break-all font-mono">{server.id}{server.connected === false && <span className="ml-2 font-sans text-caption text-muted-foreground">needs sign-in in Claude</span>}</span>
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
        {refusal && <p className="text-caption text-muted-foreground">{multi ? "Step 1's agent cannot run this step" : "This agent cannot run this workflow"}: {refusal}.</p>}
        {problem && <p className="text-caption text-muted-foreground">{problem}</p>}
        {!options && <p className="inline-flex items-center gap-1.5 text-caption text-muted-foreground"><LoaderCircle className="animate-spin" size={11}/>Loading agents and connectors…</p>}
        {missingRepo && <p className="text-caption text-muted-foreground">Choose the repository to watch, as owner/name.</p>}
        {missingChannel && <p className="text-caption text-muted-foreground">Choose the Notion database to watch, by name or link.</p>}
        {options && !step.model.trim() && <p className="text-caption text-muted-foreground">Choose a model.</p>}
        <ErrorNote message={error}/>
      </DialogPanel>
      <DialogFooter>
        <Button variant="ghost" disabled={busy} onClick={onCancel}>Cancel</Button>
        <Button variant="secondary" disabled={busy || !ready || previewPending} title={tryOn ? `On: ${tryOn.title}` : undefined} onClick={() => onSubmit({ definition: composed, approve: false, dryRun: true, event: tryOn })}>{tryOn ? "Dry run on the latest event" : "Dry run first"}</Button>
        <Button disabled={busy || !ready} onClick={() => onSubmit({ definition: composed, approve: true, dryRun: false })}>{busy ? <LoaderCircle className="animate-spin" size={12}/> : <ShieldCheck size={12}/>}Approve and save</Button>
      </DialogFooter>
    </DialogContent>}
  </Dialog>;
}
