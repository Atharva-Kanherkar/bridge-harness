import type { AdapterDescriptor, ModelProfileDraft, ProfilePurpose, ReasoningEffort } from "../types";
import { advertisedEfforts, availableModelOptions, isOrchestratorPurpose, normalizedEffort, profileLabels, profilePurposes } from "../modelProfiles";

const fieldClass = "h-9 w-full min-w-0 rounded-lg border border-input bg-card px-3 text-[13px] text-foreground transition-colors disabled:opacity-45";

export function ModelProfileEditor({ profiles, adapters, disabled, onChange }: {
  profiles: ModelProfileDraft[];
  adapters: AdapterDescriptor[];
  disabled?: boolean;
  onChange: (profiles: ModelProfileDraft[]) => void;
}) {
  const options = availableModelOptions(adapters);
  const update = (purpose: ProfilePurpose, patch: Partial<ModelProfileDraft>) => onChange(profiles.map(profile => profile.purpose === purpose ? { ...profile, ...patch } : profile));
  const orchestrators = profiles.filter(profile => isOrchestratorPurpose(profile.purpose));
  const workers = profiles.filter(profile => !isOrchestratorPurpose(profile.purpose));
  return <div className="@container/profiles space-y-6">
    {adapters.filter(adapter => adapter.modelCatalog?.stale || adapter.modelCatalog?.lastError).map(adapter => <div key={adapter.id} className="rounded-xl border border-warning/30 bg-warning/10 px-3 py-2 text-[12px] text-muted-foreground"><span className="font-medium text-foreground">{adapter.label} catalog:</span> {adapter.modelCatalog?.source === "last_known_good" ? "using last-known-good models" : "using curated fallback"}{adapter.modelCatalog?.lastError ? ` · ${adapter.modelCatalog.lastError}` : ""}</div>)}

    {/* Orchestrator: the model the user talks to. A direct choice from whatever
        the provider exposes — no fast/standard/strong tier in sight. */}
    <section className="space-y-3">
      <div>
        <h3 className="text-sm font-medium text-foreground">Orchestrator model</h3>
        <p className="mt-0.5 text-[13px] leading-relaxed text-muted-foreground">The model you talk to. Pick any model your providers expose and set how hard it thinks. Bridge still routes the workers it delegates to by capability tier below.</p>
      </div>
      {orchestrators.map(profile => {
        const selected = options.find(option => option.value === `${profile.provider}:${profile.model}`);
        // A model with no advertised effort levels has no thinking knob (e.g.
        // Claude Haiku): show the levels it does advertise, or hide the control.
        const efforts = advertisedEfforts(selected?.model);
        return <section key={profile.purpose} className="rounded-xl border border-border bg-card p-4">
          <h4 className="mb-3 text-sm font-medium text-foreground">{profileLabels[profile.purpose]}</h4>
          <div className="grid gap-3 @min-[420px]/profiles:grid-cols-2">
            <label className="space-y-1.5 block text-[12px] font-medium text-muted-foreground">Model
              <select className={fieldClass} value={`${profile.provider}:${profile.model}`} disabled={disabled} onChange={event => { const option = options.find(candidate => candidate.value === event.target.value); if (option) update(profile.purpose, { provider: option.adapter.id, model: option.model.id, effort: normalizedEffort(profile.effort, option.model), selectionMode: "pinned", pinned: true, learningEnabled: false }); }}>
                {options.map(option => <option key={option.value} value={option.value}>{option.adapter.label} · {option.model.label}</option>)}
              </select>
            </label>
            {efforts.length > 0 && <label className="space-y-1.5 block text-[12px] font-medium text-muted-foreground">Thinking
              <select className={fieldClass} value={profile.effort} disabled={disabled} onChange={event => update(profile.purpose, { effort: event.target.value as ReasoningEffort })}>
                {efforts.map(effort => <option key={effort} value={effort}>{effort}</option>)}
              </select>
            </label>}
          </div>
        </section>;
      })}
    </section>

    {/* Worker roles: delegated agents, routed by capability tier. */}
    <section className="space-y-3">
      <div>
        <h3 className="text-sm font-medium text-foreground">Worker roles</h3>
        <p className="mt-0.5 text-[13px] leading-relaxed text-muted-foreground">Agents the orchestrator delegates to. Each tracks the standard model for its capability tier, or you can pin a specific one.</p>
      </div>
      {workers.map(profile => {
      const selectionMode = profile.selectionMode ?? (profile.pinned ? "pinned" : "track_standard");
      return <section key={profile.purpose} className="rounded-xl border border-border bg-card p-4">
      <div className="mb-3 flex flex-wrap items-center justify-between gap-x-3 gap-y-1.5">
        <div className="min-w-0"><h3 className="text-sm font-medium text-foreground">{profileLabels[profile.purpose]}</h3><p className="mt-0.5 text-[12px] text-muted-foreground">{["reviewer", "evaluator"].includes(profile.purpose) ? "Verification role with a specialized rubric" : "Canonical role profile"}</p></div>
        <label className="flex items-center gap-2 text-[13px] text-muted-foreground"><input type="checkbox" checked={profile.learningEnabled} disabled={disabled || selectionMode === "pinned"} onChange={event => update(profile.purpose, { learningEnabled: event.target.checked })} />Allow learning</label>
      </div>
      <div className="grid gap-3 @min-[420px]/profiles:grid-cols-2 @min-[720px]/profiles:grid-cols-3">
        <label className="space-y-1.5 block text-[12px] font-medium text-muted-foreground">Selection behavior
          <select className={fieldClass} value={selectionMode} disabled={disabled} onChange={event => { const mode = event.target.value as "track_standard" | "pinned"; update(profile.purpose, { selectionMode: mode, pinned: mode === "pinned", learningEnabled: mode === "pinned" ? false : profile.learningEnabled }); }}><option value="track_standard">Track standard</option><option value="pinned">Pinned model</option></select>
        </label>
        <label className="space-y-1.5 block text-[12px] font-medium text-muted-foreground">Provider & model
          <select className={fieldClass} value={`${profile.provider}:${profile.model}`} disabled={disabled || selectionMode === "track_standard"} onChange={event => { const selected = options.find(option => option.value === event.target.value); if (selected) update(profile.purpose, { provider: selected.adapter.id, model: selected.model.id }); }}>
            {options.map(option => <option key={option.value} value={option.value}>{option.adapter.label} · {option.model.label}</option>)}
          </select>
        </label>
        <label className="space-y-1.5 block text-[12px] font-medium text-muted-foreground">Reasoning effort
          <select className={fieldClass} value={profile.effort} disabled={disabled} onChange={event => update(profile.purpose, { effort: event.target.value as ReasoningEffort })}>
            {(["low", "medium", "high", "xhigh"] as ReasoningEffort[]).map(effort => <option key={effort} value={effort}>{effort}</option>)}
          </select>
        </label>
        <label className="space-y-1.5 block text-[12px] font-medium text-muted-foreground">Fallback profile
          <select className={fieldClass} value={profile.fallbackPurpose ?? ""} disabled={disabled} onChange={event => update(profile.purpose, { fallbackPurpose: (event.target.value || null) as ProfilePurpose | null })}>
            <option value="">Catalog default</option>{profilePurposes.filter(purpose => purpose !== profile.purpose).map(purpose => <option key={purpose} value={purpose}>{profileLabels[purpose]}</option>)}
          </select>
        </label>
        <label className="space-y-1.5 block text-[12px] font-medium text-muted-foreground">Budget preference
          <select className={fieldClass} value={profile.budgetPreference ?? ""} disabled={disabled} onChange={event => update(profile.purpose, { budgetPreference: event.target.value || null })}><option value="">Balanced</option><option value="economy">Economy</option><option value="quality">Quality first</option></select>
        </label>
        <label className="space-y-1.5 block text-[12px] font-medium text-muted-foreground">Latency preference
          <select className={fieldClass} value={profile.latencyPreference ?? ""} disabled={disabled} onChange={event => update(profile.purpose, { latencyPreference: event.target.value || null })}><option value="">Balanced</option><option value="fast">Low latency</option><option value="patient">Patient</option></select>
        </label>
      </div>
    </section>;})}
    </section>
  </div>;
}
