import type { AdapterDescriptor, ModelProfileDraft, ProfilePurpose, ReasoningEffort } from "../types";
import { availableModelOptions, profileLabels, profilePurposes } from "../modelProfiles";

const fieldClass = "h-9 w-full min-w-0 rounded-xl border border-input bg-card px-3 text-xs text-foreground transition-colors disabled:opacity-45";

export function ModelProfileEditor({ profiles, adapters, disabled, onChange }: {
  profiles: ModelProfileDraft[];
  adapters: AdapterDescriptor[];
  disabled?: boolean;
  onChange: (profiles: ModelProfileDraft[]) => void;
}) {
  const options = availableModelOptions(adapters);
  const update = (purpose: ProfilePurpose, patch: Partial<ModelProfileDraft>) => onChange(profiles.map(profile => profile.purpose === purpose ? { ...profile, ...patch } : profile));
  return <div className="space-y-3">
    {adapters.filter(adapter => adapter.modelCatalog?.stale || adapter.modelCatalog?.lastError).map(adapter => <div key={adapter.id} className="rounded-xl border border-warning/30 bg-warning/10 px-3 py-2 text-[10px] text-muted-foreground"><span className="font-medium text-foreground">{adapter.label} catalog:</span> {adapter.modelCatalog?.source === "last_known_good" ? "using last-known-good models" : "using curated fallback"}{adapter.modelCatalog?.lastError ? ` · ${adapter.modelCatalog.lastError}` : ""}</div>)}
    {profiles.map(profile => {
      const selectionMode = profile.selectionMode ?? (profile.pinned ? "pinned" : "track_standard");
      return <section key={profile.purpose} className="rounded-2xl border border-border bg-muted/50 p-3.5">
      <div className="mb-3 flex flex-wrap items-center justify-between gap-x-3 gap-y-1.5">
        <div className="min-w-0"><h3 className="text-sm font-medium text-foreground">{profileLabels[profile.purpose]}</h3><p className="mt-0.5 text-[10px] text-muted-foreground/70">{["reviewer", "evaluator"].includes(profile.purpose) ? "Verification role with a specialized rubric" : "Canonical role profile"}</p></div>
        <label className="flex items-center gap-2 text-[11px] text-muted-foreground"><input type="checkbox" checked={profile.learningEnabled} disabled={disabled || selectionMode === "pinned"} onChange={event => update(profile.purpose, { learningEnabled: event.target.checked })} />Allow learning</label>
      </div>
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
        <label className="space-y-1.5 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">Selection behavior
          <select className={fieldClass} value={selectionMode} disabled={disabled} onChange={event => { const mode = event.target.value as "track_standard" | "pinned"; update(profile.purpose, { selectionMode: mode, pinned: mode === "pinned", learningEnabled: mode === "pinned" ? false : profile.learningEnabled }); }}><option value="track_standard">Track standard</option><option value="pinned">Pinned model</option></select>
        </label>
        <label className="space-y-1.5 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">Provider & model
          <select className={fieldClass} value={`${profile.provider}:${profile.model}`} disabled={disabled || selectionMode === "track_standard"} onChange={event => { const selected = options.find(option => option.value === event.target.value); if (selected) update(profile.purpose, { provider: selected.adapter.id, model: selected.model.id }); }}>
            {options.map(option => <option key={option.value} value={option.value}>{option.adapter.label} · {option.model.label}</option>)}
          </select>
        </label>
        <label className="space-y-1.5 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">Reasoning effort
          <select className={fieldClass} value={profile.effort} disabled={disabled} onChange={event => update(profile.purpose, { effort: event.target.value as ReasoningEffort })}>
            {(["low", "medium", "high", "xhigh"] as ReasoningEffort[]).map(effort => <option key={effort} value={effort}>{effort}</option>)}
          </select>
        </label>
        <label className="space-y-1.5 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">Fallback profile
          <select className={fieldClass} value={profile.fallbackPurpose ?? ""} disabled={disabled} onChange={event => update(profile.purpose, { fallbackPurpose: (event.target.value || null) as ProfilePurpose | null })}>
            <option value="">Catalog default</option>{profilePurposes.filter(purpose => purpose !== profile.purpose).map(purpose => <option key={purpose} value={purpose}>{profileLabels[purpose]}</option>)}
          </select>
        </label>
        <label className="space-y-1.5 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">Budget preference
          <select className={fieldClass} value={profile.budgetPreference ?? ""} disabled={disabled} onChange={event => update(profile.purpose, { budgetPreference: event.target.value || null })}><option value="">Balanced</option><option value="economy">Economy</option><option value="quality">Quality first</option></select>
        </label>
        <label className="space-y-1.5 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">Latency preference
          <select className={fieldClass} value={profile.latencyPreference ?? ""} disabled={disabled} onChange={event => update(profile.purpose, { latencyPreference: event.target.value || null })}><option value="">Balanced</option><option value="fast">Low latency</option><option value="patient">Patient</option></select>
        </label>
      </div>
    </section>;})}
  </div>;
}
