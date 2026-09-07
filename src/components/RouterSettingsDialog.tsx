import { useEffect, useMemo, useRef, useState } from "react";
import { useForm } from "@tanstack/react-form";
import { BrainCircuit, CalendarClock, CircleCheck, Copy, ExternalLink, LoaderCircle, Play, RotateCcw, ShieldCheck, Undo2, X } from "lucide-react";
import { bridgeApi } from "../api";
import { openExternalUrl } from "../externalLinks";
import type { AdapterDescriptor, ExternalLearningTriggerKind, LearningReport, LearningSchedule, LearningState, ModelProfileDraft, ModelSetupState, RouterMode, RouterPreferences } from "../types";
import { ModelProfileEditor } from "./ModelProfileEditor";
import { modelProfilesChanged, profileDraftsFromSetup } from "../modelProfiles";

const defaults: RouterPreferences = {
  mode: "shadow",
  minimumPassBps: 6500,
  pinnedHarness: null,
  pinnedModel: null,
  excludedHarnesses: [],
  excludedModels: [],
};

const defaultSchedule: LearningSchedule = {
  jobId: "", enabled: false, cadenceMinutes: 1440, nextRunAt: null, runBudgetMicrousd: 0, runBudgetTokens: 0, mode: "manual",
};

interface RouterFormValues {
  mode: RouterMode;
  passFloorPercent: string;
  pinnedHarness: string;
  pinnedModel: string;
  excludedHarnesses: string;
  excludedModels: string;
  schedule: LearningSchedule;
}

const defaultFormValues: RouterFormValues = {
  mode: defaults.mode,
  passFloorPercent: String(Math.round(defaults.minimumPassBps / 100)),
  pinnedHarness: "",
  pinnedModel: "",
  excludedHarnesses: "",
  excludedModels: "",
  schedule: defaultSchedule,
};

function parseList(value: string): string[] {
  return [...new Set(value.split(",").map(item => item.trim().toLowerCase()).filter(Boolean))];
}

function validatePercent(value: string): string | undefined {
  const trimmed = value.trim();
  if (trimmed === "") return undefined;
  const parsed = Number(trimmed);
  if (Number.isNaN(parsed) || parsed < 0 || parsed > 100) return "Enter a percentage between 0 and 100.";
  return undefined;
}

// The schedule crosses to Rust as an i64; a fractional value must never reach
// canSubmit, or Save fails downstream with a raw deserialization error instead
// of this message.
function validateCadence(value: number): string | undefined {
  if (!Number.isInteger(value)) return "Cadence must be a whole number of minutes.";
  if (value < 15) return "Cadence must be at least 15 minutes.";
  return undefined;
}

function validateNonNegativeInteger(value: number): string | undefined {
  if (!Number.isInteger(value)) return "Enter a whole number.";
  if (value < 0) return "Must be zero or greater.";
  return undefined;
}

/** Cadence, mode, and enabled — not nextRunAt or the unused evaluator ceilings. */
export function scheduleUserFieldsChanged(saved: LearningSchedule, draft: LearningSchedule): boolean {
  return saved.enabled !== draft.enabled || saved.cadenceMinutes !== draft.cadenceMinutes || saved.mode !== draft.mode
    || saved.runBudgetMicrousd !== draft.runBudgetMicrousd || saved.runBudgetTokens !== draft.runBudgetTokens;
}

/** Exhaustive over the contract so a new execution state is a compile error, not a stale label. */
export function evaluatorExecutionLabel(execution: LearningReport["evaluationExecution"]): string {
  switch (execution) {
    case "deterministic_only":
      return "deterministic only — no model evaluation requested";
    case "reused_existing_evidence":
      return "reused existing evidence";
    case "queued":
      return "queued — bounded model evaluation has not run yet";
    case "executed":
      return "bounded model evaluation executed";
    case "evaluation_failed":
      return "bounded model evaluation reached no verdict";
    // Reports persisted before the executor existed still say "deferred";
    // stored history renders honestly rather than as an empty label.
    case "deferred":
    case "not_run":
      return "not run";
  }
}

/** A draft the user is actively editing wins over a background refresh, unless nothing user-facing changed. */
function mergeLearningState(fresh: LearningState, saved: LearningSchedule | null, draftSchedule: LearningSchedule): LearningState {
  if (!saved || !scheduleUserFieldsChanged(saved, draftSchedule)) return fresh;
  return { ...fresh, schedule: draftSchedule };
}

export function LearningRunSummary({ learning, running = false, onApprove, onCancel, onRollback }: {
  learning: LearningState;
  running?: boolean;
  onApprove?: () => void;
  onCancel?: () => void;
  onRollback?: () => void;
}) {
  const run = learning.latestRun;
  if (!run) {
    return <div className="mt-4 rounded-2xl border border-dashed border-border p-4 text-xs text-muted-foreground">
      No learning runs yet. Run learning now replays the routing evidence this workspace has collected; until then the router uses its conservative priors.
    </div>;
  }
  const report = run.report;
  const rollbackTarget = learning.rollbackTargetVersion;
  return <div className="mt-4 rounded-2xl border border-border bg-muted/50 p-4 text-xs text-muted-foreground">
    <div className="flex flex-wrap items-center gap-2"><span className="rounded-full bg-accent px-2 py-1 text-[10px] uppercase tracking-wider text-foreground">{run.duplicate ? "duplicate · no-op" : run.status}</span><span>{report?.reason ?? "Learning run queued"}</span></div>
    {report && <><dl className="mt-3 grid gap-2 text-[11px] sm:grid-cols-3 lg:grid-cols-6"><div><dt className="text-muted-foreground/70">Evidence</dt><dd>{report.evidenceCount} runs · #{report.evidenceBoundary}</dd></div><div><dt className="text-muted-foreground/70">Quality</dt><dd>{report.qualityBps == null ? "Unknown" : `${(report.qualityBps / 100).toFixed(0)}%`}</dd></div><div><dt className="text-muted-foreground/70">Cost / success</dt><dd>{report.averageCostMicrousd == null ? "Unknown" : `$${(report.averageCostMicrousd / 1_000_000).toFixed(4)}`}</dd></div><div><dt className="text-muted-foreground/70">Confidence</dt><dd>{report.averageConfidenceBps == null ? "Unknown" : `${(report.averageConfidenceBps / 100).toFixed(0)}%`}</dd></div><div><dt className="text-muted-foreground/70">Replay</dt><dd>{report.replayPassed == null ? "Not run" : report.replayPassed ? "Passed" : "Blocked"}</dd></div><div><dt className="text-muted-foreground/70">Policy</dt><dd>v{report.basePolicyVersion} → {report.candidatePolicyVersion ? `v${report.candidatePolicyVersion} · ${run.promotionStatus.replaceAll("_", " ")}` : "unchanged"}</dd></div></dl><p className="mt-3 rounded-xl border border-border bg-card px-3 py-2 text-[12px] leading-relaxed text-foreground">Evaluator: <span className="font-medium">{evaluatorExecutionLabel(report.evaluationExecution)}</span>. Observed evaluator spend {`$${(report.evaluatedSpendMicrousd / 1_000_000).toFixed(4)}`} over {report.evaluatedTokens} tokens. A verdict scores confidence; it never restates what happened.</p>{!report.costComplete && <p className="mt-2 text-[10px] text-warning">Cost comparison is unknown because at least one provider did not report cost.</p>}<div className="mt-3 flex flex-wrap gap-2">{run.promotionStatus === "awaiting_approval" && <button type="button" disabled={running} onClick={onApprove} className="inline-flex items-center gap-1.5 rounded-lg bg-success px-2.5 py-1.5 text-[11px] font-medium text-success-foreground transition-colors hover:bg-success/90 disabled:opacity-40"><CircleCheck size={12} aria-hidden="true" />Approve replayed policy</button>}{["recommended", "awaiting_approval"].includes(run.promotionStatus) && <button type="button" disabled={running} onClick={onCancel} className="rounded-lg border border-border px-2.5 py-1.5 text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-40">Cancel candidate</button>}{rollbackTarget != null && rollbackTarget > 0 && <button type="button" disabled={running} onClick={onRollback} className="inline-flex items-center gap-1.5 rounded-lg border border-warning/30 px-2.5 py-1.5 text-[11px] text-warning transition-colors hover:bg-warning/10 disabled:opacity-40"><Undo2 size={12} aria-hidden="true" />Roll back to v{rollbackTarget}</button>}</div></>}
  </div>;
}

export function RouterSettingsDialog({
  open,
  workspaceId,
  adapters,
  databasePath,
  onModelSetupChange,
  onClose,
  onError,
}: {
  open: boolean;
  workspaceId?: string;
  adapters: AdapterDescriptor[];
  databasePath?: string;
  onModelSetupChange?: (setup: ModelSetupState) => void;
  onClose: () => void;
  onError: (message: string) => void;
}) {
  const form = useForm({
    defaultValues: defaultFormValues,
    onSubmit: async ({ value }) => { await save(value); },
  });
  // The last percentage a blur committed. An emptied input reverts to this on
  // blur rather than committing a 0% quality floor on the way to typing a number.
  const lastValidPassPercentRef = useRef(defaultFormValues.passFloorPercent);
  const [modelSetup, setModelSetup] = useState<ModelSetupState>();
  const [profiles, setProfiles] = useState<ModelProfileDraft[]>([]);
  const [learning, setLearning] = useState<LearningState>();
  const [busy, setBusy] = useState(false);
  const [running, setRunning] = useState(false);
  const savedScheduleRef = useRef<LearningSchedule | null>(null);
  // Which learning-state read is the newest. The notify handler, the four action
  // buttons, and the initial load all read, so a slow earlier call can land after
  // a fast later one; without this it would write its own result over the newer
  // state. Same shape as App.tsx's workReadGeneration.
  const learningReadGeneration = useRef(0);
  const models = useMemo(() => adapters.flatMap(adapter => adapter.models.map(model => ({ ...model, harness: adapter.id, harnessLabel: adapter.label }))), [adapters]);

  useEffect(() => {
    if (open) return;
    // A closed dialog keeps no state: unsaved schedule edits must not greet
    // the next open, and an in-flight read must not land on it.
    savedScheduleRef.current = null;
    learningReadGeneration.current += 1;
    setLearning(undefined);
  }, [open]);

  useEffect(() => {
    if (!open || !workspaceId) return;
    let active = true;
    let off: (() => void) | undefined;
    setBusy(true);
    const initialGeneration = ++learningReadGeneration.current;
    Promise.all([bridgeApi.routerPreferences(workspaceId), bridgeApi.modelSetup(), bridgeApi.learningState(workspaceId)]).then(([value, setup, learningState]) => {
      if (!active) return;
      const passPercent = String(Math.round(value.minimumPassBps / 100));
      lastValidPassPercentRef.current = passPercent;
      form.setFieldValue("mode", value.mode);
      form.setFieldValue("passFloorPercent", passPercent);
      form.setFieldValue("pinnedHarness", value.pinnedHarness ?? "");
      form.setFieldValue("pinnedModel", value.pinnedModel ?? "");
      form.setFieldValue("excludedHarnesses", (value.excludedHarnesses ?? []).join(", "));
      form.setFieldValue("excludedModels", (value.excludedModels ?? []).join(", "));
      setModelSetup(setup);
      setProfiles(profileDraftsFromSetup(setup));
      if (initialGeneration === learningReadGeneration.current) {
        savedScheduleRef.current = learningState.schedule;
        setLearning(learningState);
        form.setFieldValue("schedule", learningState.schedule);
      }
    }).catch(error => { if (active) onError(String(error)); }).finally(() => { if (active) setBusy(false); });
    void bridgeApi.onLearningJobChanged(() => {
      const generation = ++learningReadGeneration.current;
      void bridgeApi.learningState(workspaceId).then(fresh => {
        if (!active || generation !== learningReadGeneration.current) return;
        applyLearning(generation, fresh);
      }).catch(error => { if (active) onError(String(error)); });
    }).then(fn => {
      if (!active) { fn(); return; }
      off = fn;
    }).catch(error => { if (active) onError(String(error)); });
    return () => { active = false; off?.(); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [onError, open, workspaceId]);

  if (!open || !workspaceId) return null;
  const fieldClass = "h-10 w-full min-w-0 rounded-xl border border-input bg-card px-3 text-sm text-foreground transition-colors disabled:opacity-45";
  const errorClass = "text-[11px] font-normal normal-case tracking-normal text-danger";
  const applyLearning = (generation: number, fresh: LearningState) => {
    if (generation !== learningReadGeneration.current) return;
    const draftSchedule = form.getFieldValue("schedule");
    const next = mergeLearningState(fresh, savedScheduleRef.current, draftSchedule);
    if (next.schedule === fresh.schedule) savedScheduleRef.current = fresh.schedule;
    setLearning(next);
    form.setFieldValue("schedule", next.schedule);
  };
  const save = async (value: RouterFormValues) => {
    setBusy(true);
    try {
      const minimumPassBps = Math.round(Number(value.passFloorPercent)) * 100;
      const saved = await bridgeApi.updateRouterPreferences(workspaceId, {
        mode: value.mode,
        minimumPassBps,
        pinnedHarness: value.pinnedHarness || null,
        pinnedModel: value.pinnedModel || null,
        excludedHarnesses: parseList(value.excludedHarnesses),
        excludedModels: parseList(value.excludedModels),
      });
      const passPercent = String(Math.round(saved.minimumPassBps / 100));
      lastValidPassPercentRef.current = passPercent;
      form.setFieldValue("mode", saved.mode);
      form.setFieldValue("passFloorPercent", passPercent);
      form.setFieldValue("pinnedHarness", saved.pinnedHarness ?? "");
      form.setFieldValue("pinnedModel", saved.pinnedModel ?? "");
      form.setFieldValue("excludedHarnesses", (saved.excludedHarnesses ?? []).join(", "));
      form.setFieldValue("excludedModels", (saved.excludedModels ?? []).join(", "));
      if (profiles.length && modelProfilesChanged(profiles, modelSetup)) {
        const setup = await bridgeApi.saveModelProfiles(profiles);
        setModelSetup(setup);
        onModelSetupChange?.(setup);
      }
      if (savedScheduleRef.current && scheduleUserFieldsChanged(savedScheduleRef.current, value.schedule)) {
        const schedule = await bridgeApi.updateLearningSchedule(value.schedule);
        learningReadGeneration.current += 1;
        savedScheduleRef.current = schedule;
        setLearning(current => current ? { ...current, schedule } : current);
        form.setFieldValue("schedule", schedule);
      }
      onClose();
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };

  const runNow = async () => {
    setRunning(true);
    try {
      await bridgeApi.runLearning("manual", workspaceId);
      const generation = ++learningReadGeneration.current;
      applyLearning(generation, await bridgeApi.learningState(workspaceId));
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    } finally {
      setRunning(false);
    }
  };

  const resetProfiles = async () => {
    setBusy(true);
    try {
      const setup = await bridgeApi.resetModelProfiles();
      setModelSetup(setup);
      onModelSetupChange?.(setup);
      setProfiles(profileDraftsFromSetup(setup));
    } catch (error) { onError(error instanceof Error ? error.message : String(error)); }
    finally { setBusy(false); }
  };

  const copyInstruction = (text: string) => { void navigator.clipboard?.writeText(text); };
  const approveCandidate = async () => {
    if (!learning?.latestRun) return;
    setRunning(true);
    try {
      await bridgeApi.approveLearningRun(learning.latestRun.id);
      const generation = ++learningReadGeneration.current;
      applyLearning(generation, await bridgeApi.learningState(workspaceId));
    } catch (error) { onError(error instanceof Error ? error.message : String(error)); }
    finally { setRunning(false); }
  };
  const cancelCandidate = async () => {
    if (!learning?.latestRun) return;
    setRunning(true);
    try {
      await bridgeApi.cancelLearningRun(learning.latestRun.id);
      const generation = ++learningReadGeneration.current;
      applyLearning(generation, await bridgeApi.learningState(workspaceId));
    } catch (error) { onError(error instanceof Error ? error.message : String(error)); }
    finally { setRunning(false); }
  };
  const rollbackPolicy = async () => {
    const targetVersion = learning?.rollbackTargetVersion;
    if (targetVersion == null || targetVersion <= 0) return;
    setRunning(true);
    try {
      const generation = ++learningReadGeneration.current;
      applyLearning(generation, await bridgeApi.rollbackRoutingPolicy(workspaceId, targetVersion, "User requested rollback from adaptive-learning settings"));
    } catch (error) { onError(error instanceof Error ? error.message : String(error)); }
    finally { setRunning(false); }
  };
  const registerAndCopy = async (kind: ExternalLearningTriggerKind) => {
    const registrationId = kind === "codex" ? "codex-scheduled" : kind === "claude" ? "claude-desktop" : "opencode-scheduled";
    setRunning(true);
    try {
      await bridgeApi.registerLearningTrigger(kind, registrationId, null, null);
      await bridgeApi.enableLearningTrigger(kind, registrationId);
      copyInstruction(await bridgeApi.learningTriggerInstructions(kind, databasePath ?? "$BRIDGE_DB", registrationId));
    } catch (error) { onError(error instanceof Error ? error.message : String(error)); }
    finally { setRunning(false); }
  };

  return <div className="fixed inset-0 z-50 flex items-start justify-center overflow-y-auto bg-scrim p-4 pt-[6vh] backdrop-blur-md" role="dialog" aria-modal="true" aria-labelledby="router-settings-title" onMouseDown={event => { if (event.target === event.currentTarget) onClose(); }}>
    <div className="u-overlay-strong animate-page-enter flex max-h-[90dvh] w-full max-w-4xl flex-col overflow-hidden rounded-3xl">
      <header className="flex shrink-0 items-start gap-3 border-b border-border px-5 py-4">
        <span className="mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-muted text-foreground"><BrainCircuit size={18} aria-hidden="true" /></span>
        <div className="min-w-0 flex-1"><h2 id="router-settings-title" className="font-display text-base font-semibold text-foreground">Learning router</h2><p className="mt-1 text-[13px] leading-relaxed text-muted-foreground">Choose the least expensive route that preserves your measured quality floor.</p><p className="mt-2 text-[12px] leading-relaxed text-muted-foreground">This panel is the helper picker. It is not Bridge's memory engine. Provider <span className="font-mono text-[11px]">/memory</span> and <span className="font-mono text-[11px]">/memories</span> stay on that provider.</p></div>
        <button type="button" className="shrink-0 rounded-xl p-2 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground" onClick={onClose} aria-label="Close"><X size={16} aria-hidden="true" /></button>
      </header>
      <div className="min-h-0 flex-1 space-y-5 overflow-y-auto p-5">
        <div className="grid gap-4 sm:grid-cols-2">
          <form.Field name="mode">{field => (
            <label className="space-y-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">Mode
              <select className={fieldClass} value={field.state.value} disabled={busy} onChange={event => field.handleChange(event.target.value as RouterMode)}>
                <option value="disabled">Disabled</option><option value="shadow">Shadow</option><option value="autonomous">Autonomous</option>
              </select>
            </label>
          )}</form.Field>
          <form.Field name="passFloorPercent" validators={{ onChange: ({ value }) => validatePercent(value) }} listeners={{
            onBlur: ({ value, fieldApi }) => {
              const trimmed = value.trim();
              const parsed = Number(trimmed);
              if (trimmed === "" || Number.isNaN(parsed)) {
                fieldApi.setValue(lastValidPassPercentRef.current);
                return;
              }
              if (parsed >= 0 && parsed <= 100) {
                const normalized = String(Math.round(parsed));
                lastValidPassPercentRef.current = normalized;
                fieldApi.setValue(normalized);
              }
            },
          }}>{field => (
            <label className="space-y-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">Minimum pass probability
              <div className="relative"><input className={fieldClass} type="number" min={0} max={100} step={1} value={field.state.value} disabled={busy} onChange={event => field.handleChange(event.target.value)} onBlur={field.handleBlur} /><span className="pointer-events-none absolute right-3 top-2.5 text-sm text-muted-foreground">%</span></div>
              {field.state.meta.errors.length > 0 && <p className={errorClass}>{String(field.state.meta.errors[0])}</p>}
            </label>
          )}</form.Field>
          <form.Field name="pinnedHarness" listeners={{
            onChange: ({ fieldApi }) => fieldApi.form.setFieldValue("pinnedModel", ""),
          }}>{field => (
            <label className="space-y-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">Pin harness
              <select className={fieldClass} value={field.state.value} disabled={busy} onChange={event => field.handleChange(event.target.value)}>
                <option value="">Automatic</option>{adapters.map(adapter => <option key={adapter.id} value={adapter.id}>{adapter.label}{adapter.available ? "" : " (unavailable)"}</option>)}
              </select>
            </label>
          )}</form.Field>
          <form.Subscribe selector={state => state.values.pinnedHarness}>{pinnedHarness => (
            <form.Field name="pinnedModel">{field => (
              <label className="space-y-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">Pin model
                <select className={fieldClass} value={field.state.value} disabled={busy} onChange={event => field.handleChange(event.target.value)}>
                  <option value="">Automatic</option>{models.filter(model => !pinnedHarness || model.harness === pinnedHarness).map(model => <option key={`${model.harness}:${model.id}`} value={model.id}>{model.harnessLabel} · {model.label}</option>)}
                </select>
              </label>
            )}</form.Field>
          )}</form.Subscribe>
        </div>
        <form.Field name="excludedHarnesses">{field => (
          <label className="block space-y-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">Exclude harnesses <span className="normal-case tracking-normal text-muted-foreground/70">comma-separated IDs</span><input className={fieldClass} value={field.state.value} disabled={busy} placeholder="e.g. claude" onChange={event => field.handleChange(event.target.value)} /></label>
        )}</form.Field>
        <form.Field name="excludedModels">{field => (
          <label className="block space-y-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">Exclude models <span className="normal-case tracking-normal text-muted-foreground/70">comma-separated IDs</span><input className={fieldClass} value={field.state.value} disabled={busy} placeholder="e.g. opus, gpt-5.3-codex" onChange={event => field.handleChange(event.target.value)} /></label>
        )}</form.Field>
        <div className="flex gap-3 rounded-2xl border border-success/20 bg-success/10 p-4"><ShieldCheck className="mt-0.5 shrink-0 text-success" size={17} aria-hidden="true" /><p className="text-[12px] leading-relaxed text-muted-foreground">Shadow mode measures recommendations without changing execution. Autonomous mode unlocks only after 20 completed shadow outcomes with fewer than 5% manual or no-route decisions. Pins never bypass permissions or budgets.</p></div>

        <section className="border-t border-border pt-5">
          <div className="mb-4 flex flex-wrap items-start justify-between gap-3"><div><h3 className="font-display text-base font-semibold text-foreground">Role model profiles</h3><p className="mt-1 text-xs text-muted-foreground">Version {modelSetup?.activeVersion ?? "—"}. Edits create a new immutable version.</p></div><button type="button" disabled={busy} onClick={() => void resetProfiles()} className="inline-flex items-center gap-1.5 rounded-xl px-3 py-2 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"><RotateCcw size={13} aria-hidden="true" />Reset defaults</button></div>
          <ModelProfileEditor profiles={profiles} adapters={adapters} disabled={busy} onChange={setProfiles} />
        </section>

        <section className="border-t border-border pt-5">
          <div className="flex flex-wrap items-start justify-between gap-4"><div><h3 className="font-display text-base font-semibold text-foreground">Adaptive learning</h3><p className="mt-1 text-xs leading-relaxed text-muted-foreground">One local runner freezes typed evidence, replays held-out outcomes, and manages immutable policy versions. Active policy v{learning?.activePolicyVersion ?? "—"}{learning?.canaryPolicyVersion ? ` · canary v${learning.canaryPolicyVersion}` : ""}.</p></div><button type="button" disabled={running || busy} onClick={() => void runNow()} className="inline-flex shrink-0 items-center gap-2 rounded-xl bg-primary px-3.5 py-2 text-xs font-medium text-primary-foreground transition-colors hover:bg-primary/90 disabled:opacity-40">{running ? <LoaderCircle className="animate-spin" size={14} aria-hidden="true" /> : <Play size={14} aria-hidden="true" />}Run learning now</button></div>
          {learning && <LearningRunSummary learning={learning} running={running} onApprove={() => void approveCandidate()} onCancel={() => void cancelCandidate()} onRollback={() => void rollbackPolicy()} />}

          {learning && <div className="mt-4 grid gap-3 rounded-2xl border border-border p-4 sm:grid-cols-2 lg:grid-cols-5">
            <form.Field name="schedule.enabled" listeners={{
              onChange: ({ value, fieldApi }) => {
                if (value && !fieldApi.form.getFieldValue("schedule.nextRunAt")) {
                  fieldApi.form.setFieldValue("schedule.nextRunAt", new Date(Date.now() + 86_400_000).toISOString());
                }
              },
            }}>{field => (
              <label className="flex items-center gap-2 text-xs text-muted-foreground"><input type="checkbox" checked={field.state.value} onChange={event => field.handleChange(event.target.checked)} /><CalendarClock size={14} aria-hidden="true" />In-app schedule</label>
            )}</form.Field>
            <form.Field name="schedule.mode">{field => (
              <label className="space-y-1 text-[10px] uppercase tracking-wider text-muted-foreground/70">Learning mode<select aria-label="Learning mode" className={fieldClass} value={field.state.value} onChange={event => field.handleChange(event.target.value)}><option value="manual">Manual · recommend</option><option value="ask">Ask · approval required</option><option value="automatic">Automatic · guarded canary</option></select></label>
            )}</form.Field>
            <form.Field name="schedule.cadenceMinutes" validators={{ onChange: ({ value }) => validateCadence(value) }}>{field => (
              <label className="space-y-1 text-[10px] uppercase tracking-wider text-muted-foreground/70">Cadence<input className={fieldClass} type="number" min={15} value={field.state.value} onChange={event => field.handleChange(Number(event.target.value))} onBlur={field.handleBlur} />{field.state.meta.errors.length > 0 && <p className={errorClass}>{String(field.state.meta.errors[0])}</p>}</label>
            )}</form.Field>
            <form.Field name="schedule.runBudgetMicrousd" validators={{ onChange: ({ value }) => validateNonNegativeInteger(value) }}>{field => (
              <label className="space-y-1 text-[10px] uppercase tracking-wider text-muted-foreground/70">Spend ceiling (µUSD)<input className={fieldClass} type="number" min={0} value={field.state.value} onChange={event => field.handleChange(Number(event.target.value))} onBlur={field.handleBlur} />{field.state.meta.errors.length > 0 && <p className={errorClass}>{String(field.state.meta.errors[0])}</p>}</label>
            )}</form.Field>
            <form.Field name="schedule.runBudgetTokens" validators={{ onChange: ({ value }) => validateNonNegativeInteger(value) }}>{field => (
              <label className="space-y-1 text-[10px] uppercase tracking-wider text-muted-foreground/70">Token ceiling<input className={fieldClass} type="number" min={0} value={field.state.value} onChange={event => field.handleChange(Number(event.target.value))} onBlur={field.handleBlur} />{field.state.meta.errors.length > 0 && <p className={errorClass}>{String(field.state.meta.errors[0])}</p>}</label>
            )}</form.Field>
            <form.Subscribe selector={state => state.values.schedule.mode}>{mode => mode === "automatic" && <p className="text-[10px] leading-relaxed text-warning sm:col-span-2 lg:col-span-5">Automatic mode is opt-in. It promotes only replay-approved candidates to a canary and creates an immutable rollback version on regression.</p>}</form.Subscribe>
            <p className="text-[10px] leading-relaxed text-muted-foreground sm:col-span-2 lg:col-span-5">Spend and token ceilings cap what one learning run's bounded evaluations may observe; once a run reaches either, its remaining evaluations are skipped. A zero ceiling makes the whole learning run an auditable no-op.</p>
          </div>}

          <div className="mt-4 grid gap-3 sm:grid-cols-2">
            <article className="rounded-2xl border border-border p-4"><h4 className="text-xs font-medium text-foreground">Codex Scheduled <span className="font-normal text-muted-foreground/70">· optional</span></h4><p className="mt-2 text-[11px] leading-relaxed text-muted-foreground">Managed in Codex/ChatGPT. Bridge cannot create or enumerate schedules. Registration copies a tested narrow local wake-up task; it never grants promotion authority.</p><div className="mt-3 flex flex-wrap gap-3"><button type="button" disabled={running} onClick={() => void registerAndCopy("codex")} className="inline-flex items-center gap-1.5 text-[11px] text-info transition-colors hover:text-info/80"><Copy size={12} aria-hidden="true" />Register + copy task</button><button type="button" onClick={() => void openExternalUrl("https://chatgpt.com/codex")} className="inline-flex items-center gap-1.5 text-[11px] text-muted-foreground transition-colors hover:text-foreground"><ExternalLink size={12} aria-hidden="true" />Open Codex Scheduled setup</button></div></article>
            <article className="rounded-2xl border border-border p-4"><h4 className="text-xs font-medium text-foreground">Claude Desktop schedule <span className="font-normal text-muted-foreground/70">· optional</span></h4><p className="mt-2 text-[11px] leading-relaxed text-muted-foreground">Prefer a local Desktop task for local evidence. Cloud Routines remain experimental and require a future Bridge-owned authenticated endpoint; local SQLite is never uploaded.</p><button type="button" disabled={running} onClick={() => void registerAndCopy("claude")} className="mt-3 inline-flex items-center gap-1.5 text-[11px] text-info transition-colors hover:text-info/80"><Copy size={12} aria-hidden="true" />Register + copy local task</button></article>
            <article className="rounded-2xl border border-border p-4"><h4 className="text-xs font-medium text-foreground">OpenCode schedule <span className="font-normal text-muted-foreground/70">· optional</span></h4><p className="mt-2 text-[11px] leading-relaxed text-muted-foreground">Register a narrow local OpenCode wake-up command. Bridge retains replay, approval, promotion, and rollback authority.</p><button type="button" disabled={running} onClick={() => void registerAndCopy("open_code")} className="mt-3 inline-flex items-center gap-1.5 text-[11px] text-info transition-colors hover:text-info/80"><Copy size={12} aria-hidden="true" />Register + copy local task</button></article>
          </div>
        </section>
      </div>
      <footer className="flex shrink-0 flex-wrap justify-end gap-2 border-t border-border px-5 py-4"><button type="button" className="rounded-xl px-4 py-2 text-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground" disabled={busy} onClick={onClose}>Cancel</button><form.Subscribe selector={state => state.canSubmit}>{canSubmit => (
        <button type="button" className="inline-flex min-w-24 items-center justify-center gap-2 rounded-xl bg-primary px-4 py-2 text-sm font-medium text-primary-foreground transition-colors hover:bg-primary/90 disabled:opacity-40" disabled={busy || !canSubmit} onClick={() => void form.handleSubmit()}>{busy && <LoaderCircle className="animate-spin" size={14} aria-hidden="true" />}Save</button>
      )}</form.Subscribe></footer>
    </div>
  </div>;
}
