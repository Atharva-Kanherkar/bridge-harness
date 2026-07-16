import { useEffect, useMemo, useState } from "react";
import { BrainCircuit, CalendarClock, CircleCheck, Copy, ExternalLink, LoaderCircle, Play, RotateCcw, ShieldCheck, Undo2, X } from "lucide-react";
import { bridgeApi } from "../api";
import type { AdapterDescriptor, LearningState, ModelProfileDraft, ModelSetupState, RouterMode, RouterPreferences } from "../types";
import { ModelProfileEditor } from "./ModelProfileEditor";

const defaults: RouterPreferences = {
  mode: "shadow",
  minimumPassBps: 6500,
  pinnedHarness: null,
  pinnedModel: null,
  excludedHarnesses: [],
  excludedModels: [],
};

function parseList(value: string): string[] {
  return [...new Set(value.split(",").map(item => item.trim().toLowerCase()).filter(Boolean))];
}

export function LearningRunSummary({ learning, running = false, onApprove, onCancel, onRollback }: {
  learning: LearningState;
  running?: boolean;
  onApprove?: () => void;
  onCancel?: () => void;
  onRollback?: () => void;
}) {
  const run = learning.latestRun;
  if (!run) return null;
  const report = run.report;
  return <div className="mt-4 rounded-2xl border border-white/[0.07] bg-white/[0.025] p-4 text-xs text-neutral-400">
    <div className="flex flex-wrap items-center gap-2"><span className="rounded-full bg-white/[0.07] px-2 py-1 text-[10px] uppercase tracking-wider text-neutral-300">{run.duplicate ? "duplicate · no-op" : run.status}</span><span>{report?.reason ?? "Learning run queued"}</span></div>
    {report && <><dl className="mt-3 grid gap-2 text-[11px] sm:grid-cols-3 lg:grid-cols-6"><div><dt className="text-neutral-600">Evidence</dt><dd>{report.evidenceCount} runs · #{report.evidenceBoundary}</dd></div><div><dt className="text-neutral-600">Quality</dt><dd>{report.qualityBps == null ? "Unknown" : `${(report.qualityBps / 100).toFixed(0)}%`}</dd></div><div><dt className="text-neutral-600">Cost / success</dt><dd>{report.averageCostMicrousd == null ? "Unknown" : `$${(report.averageCostMicrousd / 1_000_000).toFixed(4)}`}</dd></div><div><dt className="text-neutral-600">Confidence</dt><dd>{report.averageConfidenceBps == null ? "Unknown" : `${(report.averageConfidenceBps / 100).toFixed(0)}%`}</dd></div><div><dt className="text-neutral-600">Replay</dt><dd>{report.replayPassed == null ? "Not run" : report.replayPassed ? "Passed" : "Blocked"}</dd></div><div><dt className="text-neutral-600">Policy</dt><dd>v{report.basePolicyVersion} → {report.candidatePolicyVersion ? `v${report.candidatePolicyVersion} · ${run.promotionStatus.replaceAll("_", " ")}` : "unchanged"}</dd></div></dl>{!report.costComplete && <p className="mt-2 text-[10px] text-amber-300/75">Cost comparison is unknown because at least one provider did not report cost.</p>}<div className="mt-3 flex flex-wrap gap-2">{run.promotionStatus === "awaiting_approval" && <button type="button" disabled={running} onClick={onApprove} className="inline-flex items-center gap-1.5 rounded-lg bg-emerald-300 px-2.5 py-1.5 text-[11px] font-medium text-emerald-950"><CircleCheck size={12} aria-hidden="true" />Approve replayed policy</button>}{["recommended", "awaiting_approval"].includes(run.promotionStatus) && <button type="button" disabled={running} onClick={onCancel} className="rounded-lg border border-white/[0.08] px-2.5 py-1.5 text-[11px] text-neutral-400">Cancel candidate</button>}{learning.activePolicyVersion !== report.basePolicyVersion && <button type="button" disabled={running} onClick={onRollback} className="inline-flex items-center gap-1.5 rounded-lg border border-amber-300/20 px-2.5 py-1.5 text-[11px] text-amber-200"><Undo2 size={12} aria-hidden="true" />Roll back to v{report.basePolicyVersion}</button>}</div></>}
  </div>;
}

export function RouterSettingsDialog({
  open,
  workspaceId,
  adapters,
  databasePath,
  onClose,
  onError,
}: {
  open: boolean;
  workspaceId?: string;
  adapters: AdapterDescriptor[];
  databasePath?: string;
  onClose: () => void;
  onError: (message: string) => void;
}) {
  const [preferences, setPreferences] = useState<RouterPreferences>(defaults);
  const [excludedHarnesses, setExcludedHarnesses] = useState("");
  const [excludedModels, setExcludedModels] = useState("");
  const [modelSetup, setModelSetup] = useState<ModelSetupState>();
  const [profiles, setProfiles] = useState<ModelProfileDraft[]>([]);
  const [learning, setLearning] = useState<LearningState>();
  const [busy, setBusy] = useState(false);
  const [running, setRunning] = useState(false);
  const models = useMemo(() => adapters.flatMap(adapter => adapter.models.map(model => ({ ...model, harness: adapter.id, harnessLabel: adapter.label }))), [adapters]);

  useEffect(() => {
    if (!open || !workspaceId) return;
    let active = true;
    setBusy(true);
    Promise.all([bridgeApi.routerPreferences(workspaceId), bridgeApi.modelSetup(), bridgeApi.learningState()]).then(([value, setup, learningState]) => {
      if (!active) return;
      setPreferences(value);
      setExcludedHarnesses(value.excludedHarnesses.join(", "));
      setExcludedModels(value.excludedModels.join(", "));
      setModelSetup(setup);
      setProfiles(setup.profiles.map(({ purpose, provider, model, effort, fallbackPurpose, pinned, learningEnabled, budgetPreference, latencyPreference }) => ({ purpose, provider, model, effort, fallbackPurpose, pinned, learningEnabled, budgetPreference, latencyPreference })));
      setLearning(learningState);
    }).catch(error => { if (active) onError(String(error)); }).finally(() => { if (active) setBusy(false); });
    return () => { active = false; };
  }, [onError, open, workspaceId]);

  if (!open || !workspaceId) return null;
  const fieldClass = "h-10 w-full rounded-xl border border-white/[0.09] bg-white/[0.04] px-3 text-sm text-neutral-200 outline-none transition-colors focus:border-white/[0.18] disabled:opacity-45";
  const save = async () => {
    setBusy(true);
    try {
      const saved = await bridgeApi.updateRouterPreferences(workspaceId, {
        ...preferences,
        excludedHarnesses: parseList(excludedHarnesses),
        excludedModels: parseList(excludedModels),
      });
      setPreferences(saved);
      if (profiles.length) setModelSetup(await bridgeApi.saveModelProfiles(profiles));
      if (learning) setLearning({ ...learning, schedule: await bridgeApi.updateLearningSchedule(learning.schedule) });
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
      await bridgeApi.runLearning("manual");
      setLearning(await bridgeApi.learningState());
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
      setProfiles(setup.profiles.map(({ purpose, provider, model, effort, fallbackPurpose, pinned, learningEnabled, budgetPreference, latencyPreference }) => ({ purpose, provider, model, effort, fallbackPurpose, pinned, learningEnabled, budgetPreference, latencyPreference })));
    } catch (error) { onError(error instanceof Error ? error.message : String(error)); }
    finally { setBusy(false); }
  };

  const copyInstruction = (text: string) => { void navigator.clipboard?.writeText(text); };
  const approveCandidate = async () => {
    if (!learning?.latestRun) return;
    setRunning(true);
    try {
      await bridgeApi.approveLearningRun(learning.latestRun.id);
      setLearning(await bridgeApi.learningState());
    } catch (error) { onError(error instanceof Error ? error.message : String(error)); }
    finally { setRunning(false); }
  };
  const cancelCandidate = async () => {
    if (!learning?.latestRun) return;
    setRunning(true);
    try {
      await bridgeApi.cancelLearningRun(learning.latestRun.id);
      setLearning(await bridgeApi.learningState());
    } catch (error) { onError(error instanceof Error ? error.message : String(error)); }
    finally { setRunning(false); }
  };
  const rollbackPolicy = async () => {
    const targetVersion = learning?.latestRun?.basePolicyVersion;
    if (!targetVersion) return;
    setRunning(true);
    try {
      setLearning(await bridgeApi.rollbackRoutingPolicy(targetVersion, "User requested rollback from adaptive-learning settings"));
    } catch (error) { onError(error instanceof Error ? error.message : String(error)); }
    finally { setRunning(false); }
  };
  const registerAndCopy = async (kind: "codex" | "claude") => {
    const registrationId = kind === "codex" ? "codex-scheduled" : "claude-desktop";
    setRunning(true);
    try {
      await bridgeApi.registerLearningTrigger(kind, registrationId, null, null);
      await bridgeApi.enableLearningTrigger(kind, registrationId);
      copyInstruction(await bridgeApi.learningTriggerInstructions(kind, databasePath ?? "$BRIDGE_DB", registrationId));
    } catch (error) { onError(error instanceof Error ? error.message : String(error)); }
    finally { setRunning(false); }
  };

  return <div className="fixed inset-0 z-50 flex items-start justify-center bg-black/70 p-4 pt-[7vh] backdrop-blur-md" role="dialog" aria-modal="true" aria-labelledby="router-settings-title" onMouseDown={event => { if (event.target === event.currentTarget) onClose(); }}>
    <div className="animate-page-enter w-full max-w-4xl overflow-hidden rounded-3xl border border-white/[0.09] bg-[#121214]/95 shadow-2xl shadow-black/50">
      <header className="flex items-start gap-3 border-b border-white/[0.08] px-5 py-4">
        <span className="mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-violet-400/[0.09] text-violet-300"><BrainCircuit size={18} aria-hidden="true" /></span>
        <div className="min-w-0 flex-1"><h2 id="router-settings-title" className="font-display text-base font-semibold text-white">Learning router</h2><p className="mt-1 text-[13px] leading-relaxed text-neutral-500">Choose the least expensive route that preserves your measured quality floor.</p></div>
        <button type="button" className="rounded-xl p-2 text-neutral-500 hover:bg-white/[0.08] hover:text-neutral-200" onClick={onClose} aria-label="Close"><X size={16} aria-hidden="true" /></button>
      </header>
      <div className="max-h-[68vh] space-y-5 overflow-y-auto p-5">
        <div className="grid gap-4 sm:grid-cols-2">
          <label className="space-y-2 text-[11px] font-semibold uppercase tracking-wider text-neutral-500">Mode
            <select className={fieldClass} value={preferences.mode} disabled={busy} onChange={event => setPreferences(current => ({ ...current, mode: event.target.value as RouterMode }))}>
              <option value="disabled">Disabled</option><option value="shadow">Shadow</option><option value="autonomous">Autonomous</option>
            </select>
          </label>
          <label className="space-y-2 text-[11px] font-semibold uppercase tracking-wider text-neutral-500">Minimum pass probability
            <div className="relative"><input className={fieldClass} type="number" min={0} max={100} step={1} value={Math.round(preferences.minimumPassBps / 100)} disabled={busy} onChange={event => setPreferences(current => ({ ...current, minimumPassBps: Math.max(0, Math.min(10000, Number(event.target.value) * 100)) }))} /><span className="pointer-events-none absolute right-3 top-2.5 text-sm text-neutral-500">%</span></div>
          </label>
          <label className="space-y-2 text-[11px] font-semibold uppercase tracking-wider text-neutral-500">Pin harness
            <select className={fieldClass} value={preferences.pinnedHarness ?? ""} disabled={busy} onChange={event => setPreferences(current => ({ ...current, pinnedHarness: event.target.value || null, pinnedModel: null }))}>
              <option value="">Automatic</option>{adapters.map(adapter => <option key={adapter.id} value={adapter.id}>{adapter.label}{adapter.available ? "" : " (unavailable)"}</option>)}
            </select>
          </label>
          <label className="space-y-2 text-[11px] font-semibold uppercase tracking-wider text-neutral-500">Pin model
            <select className={fieldClass} value={preferences.pinnedModel ?? ""} disabled={busy} onChange={event => setPreferences(current => ({ ...current, pinnedModel: event.target.value || null }))}>
              <option value="">Automatic</option>{models.filter(model => !preferences.pinnedHarness || model.harness === preferences.pinnedHarness).map(model => <option key={`${model.harness}:${model.id}`} value={model.id}>{model.harnessLabel} · {model.label}</option>)}
            </select>
          </label>
        </div>
        <label className="block space-y-2 text-[11px] font-semibold uppercase tracking-wider text-neutral-500">Exclude harnesses <span className="normal-case tracking-normal text-neutral-600">comma-separated IDs</span><input className={fieldClass} value={excludedHarnesses} disabled={busy} placeholder="e.g. claude" onChange={event => setExcludedHarnesses(event.target.value)} /></label>
        <label className="block space-y-2 text-[11px] font-semibold uppercase tracking-wider text-neutral-500">Exclude models <span className="normal-case tracking-normal text-neutral-600">comma-separated IDs</span><input className={fieldClass} value={excludedModels} disabled={busy} placeholder="e.g. opus, gpt-5.3-codex" onChange={event => setExcludedModels(event.target.value)} /></label>
        <div className="flex gap-3 rounded-2xl border border-emerald-300/[0.09] bg-emerald-300/[0.04] p-4"><ShieldCheck className="mt-0.5 shrink-0 text-emerald-300" size={17} aria-hidden="true" /><p className="text-[12px] leading-relaxed text-neutral-400">Shadow mode measures recommendations without changing execution. Autonomous mode unlocks only after 20 completed shadow outcomes with fewer than 5% manual or no-route decisions. Pins never bypass permissions or budgets.</p></div>

        <section className="border-t border-white/[0.07] pt-5">
          <div className="mb-4 flex items-start justify-between gap-3"><div><h3 className="font-display text-base font-semibold text-white">Role model profiles</h3><p className="mt-1 text-xs text-neutral-500">Version {modelSetup?.activeVersion ?? "—"}. Edits create a new immutable version.</p></div><button type="button" disabled={busy} onClick={() => void resetProfiles()} className="inline-flex items-center gap-1.5 rounded-xl px-3 py-2 text-xs text-neutral-500 hover:bg-white/[0.05] hover:text-neutral-300"><RotateCcw size={13} aria-hidden="true" />Reset defaults</button></div>
          <ModelProfileEditor profiles={profiles} adapters={adapters} disabled={busy} onChange={setProfiles} />
        </section>

        <section className="border-t border-white/[0.07] pt-5">
          <div className="flex items-start justify-between gap-4"><div><h3 className="font-display text-base font-semibold text-white">Adaptive learning</h3><p className="mt-1 text-xs leading-relaxed text-neutral-500">One local runner freezes typed evidence, replays held-out outcomes, and manages immutable policy versions. Active policy v{learning?.activePolicyVersion ?? "—"}{learning?.canaryPolicyVersion ? ` · canary v${learning.canaryPolicyVersion}` : ""}.</p></div><button type="button" disabled={running || busy} onClick={() => void runNow()} className="inline-flex shrink-0 items-center gap-2 rounded-xl bg-violet-300 px-3.5 py-2 text-xs font-medium text-violet-950 disabled:opacity-40">{running ? <LoaderCircle className="animate-spin" size={14} aria-hidden="true" /> : <Play size={14} aria-hidden="true" />}Run learning now</button></div>
          {learning && <LearningRunSummary learning={learning} running={running} onApprove={() => void approveCandidate()} onCancel={() => void cancelCandidate()} onRollback={() => void rollbackPolicy()} />}

          {learning && <div className="mt-4 grid gap-3 rounded-2xl border border-white/[0.07] p-4 sm:grid-cols-2 lg:grid-cols-5"><label className="flex items-center gap-2 text-xs text-neutral-400"><input type="checkbox" checked={learning.schedule.enabled} onChange={event => setLearning(current => current ? { ...current, schedule: { ...current.schedule, enabled: event.target.checked, nextRunAt: event.target.checked && !current.schedule.nextRunAt ? new Date(Date.now() + 86_400_000).toISOString() : current.schedule.nextRunAt } } : current)} /><CalendarClock size={14} aria-hidden="true" />In-app schedule</label><label className="space-y-1 text-[10px] uppercase tracking-wider text-neutral-600">Learning mode<select aria-label="Learning mode" className={fieldClass} value={learning.schedule.mode} onChange={event => setLearning(current => current ? { ...current, schedule: { ...current.schedule, mode: event.target.value as LearningState["schedule"]["mode"] } } : current)}><option value="manual">Manual · recommend</option><option value="ask">Ask · approval required</option><option value="automatic">Automatic · guarded canary</option></select></label><label className="space-y-1 text-[10px] uppercase tracking-wider text-neutral-600">Cadence<input className={fieldClass} type="number" min={15} value={learning.schedule.cadenceMinutes} onChange={event => setLearning(current => current ? { ...current, schedule: { ...current.schedule, cadenceMinutes: Math.max(15, Number(event.target.value)) } } : current)} /></label><label className="space-y-1 text-[10px] uppercase tracking-wider text-neutral-600">Spend ceiling (µUSD)<input className={fieldClass} type="number" min={0} value={learning.schedule.runBudgetMicrousd} onChange={event => setLearning(current => current ? { ...current, schedule: { ...current.schedule, runBudgetMicrousd: Math.max(0, Number(event.target.value)) } } : current)} /></label><label className="space-y-1 text-[10px] uppercase tracking-wider text-neutral-600">Token ceiling<input className={fieldClass} type="number" min={0} value={learning.schedule.runBudgetTokens} onChange={event => setLearning(current => current ? { ...current, schedule: { ...current.schedule, runBudgetTokens: Math.max(0, Number(event.target.value)) } } : current)} /></label>{learning.schedule.mode === "automatic" && <p className="text-[10px] leading-relaxed text-amber-300/75 sm:col-span-2 lg:col-span-5">Automatic mode is opt-in. It promotes only replay-approved candidates to a canary and creates an immutable rollback version on regression.</p>}</div>}

          <div className="mt-4 grid gap-3 sm:grid-cols-2">
            <article className="rounded-2xl border border-white/[0.07] p-4"><h4 className="text-xs font-medium text-neutral-300">Codex Scheduled <span className="font-normal text-neutral-600">· optional</span></h4><p className="mt-2 text-[11px] leading-relaxed text-neutral-500">Managed in Codex/ChatGPT. Bridge cannot create or enumerate schedules. Registration copies a tested narrow local wake-up task; it never grants promotion authority.</p><div className="mt-3 flex flex-wrap gap-3"><button type="button" disabled={running} onClick={() => void registerAndCopy("codex")} className="inline-flex items-center gap-1.5 text-[11px] text-violet-300 hover:text-violet-200"><Copy size={12} aria-hidden="true" />Register + copy task</button><button type="button" onClick={() => window.open("https://chatgpt.com/codex", "_blank", "noopener,noreferrer")} className="inline-flex items-center gap-1.5 text-[11px] text-neutral-500 hover:text-neutral-300"><ExternalLink size={12} aria-hidden="true" />Open Codex Scheduled setup</button></div></article>
            <article className="rounded-2xl border border-white/[0.07] p-4"><h4 className="text-xs font-medium text-neutral-300">Claude Desktop schedule <span className="font-normal text-neutral-600">· optional</span></h4><p className="mt-2 text-[11px] leading-relaxed text-neutral-500">Prefer a local Desktop task for local evidence. Cloud Routines remain experimental and require a future Bridge-owned authenticated endpoint; local SQLite is never uploaded.</p><button type="button" disabled={running} onClick={() => void registerAndCopy("claude")} className="mt-3 inline-flex items-center gap-1.5 text-[11px] text-violet-300 hover:text-violet-200"><Copy size={12} aria-hidden="true" />Register + copy local task</button></article>
          </div>
        </section>
      </div>
      <footer className="flex justify-end gap-2 border-t border-white/[0.08] px-5 py-4"><button type="button" className="rounded-xl px-4 py-2 text-sm text-neutral-500 hover:text-neutral-200" disabled={busy} onClick={onClose}>Cancel</button><button type="button" className="inline-flex min-w-24 items-center justify-center gap-2 rounded-xl bg-white px-4 py-2 text-sm font-medium text-neutral-900 disabled:opacity-40" disabled={busy} onClick={() => void save()}>{busy && <LoaderCircle className="animate-spin" size={14} aria-hidden="true" />}Save</button></footer>
    </div>
  </div>;
}
