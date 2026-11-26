import type { AdapterDescriptor, ModelProfileDraft, ProfilePurpose, ReasoningEffort } from "../types";
import { Checkbox } from "@/components/ui/checkbox";
import { SelectField } from "@/components/ui/select";
import { advertisedEfforts, availableModelOptions, isOrchestratorPurpose, normalizedEffort, profileLabels, profilePurposes } from "../modelProfiles";


export function ModelProfileEditor({ profiles, adapters, disabled, onChange }: {
  profiles: ModelProfileDraft[];
  adapters: AdapterDescriptor[];
  disabled?: boolean;
  onChange: (profiles: ModelProfileDraft[]) => void;
}) {
  const options = availableModelOptions(adapters);
  const modelChoices = options.map(option => ({ value: option.value, label: `${option.adapter.label} · ${option.model.label}` }));
  const update = (purpose: ProfilePurpose, patch: Partial<ModelProfileDraft>) => onChange(profiles.map(profile => profile.purpose === purpose ? { ...profile, ...patch } : profile));
  const orchestrators = profiles.filter(profile => isOrchestratorPurpose(profile.purpose));
  const workers = profiles.filter(profile => !isOrchestratorPurpose(profile.purpose));
  return <div className="@container/profiles space-y-6">
    {adapters.filter(adapter => adapter.modelCatalog?.stale || adapter.modelCatalog?.lastError).map(adapter => <div key={adapter.id} className="rounded-xl border border-warning/30 bg-warning/10 px-3 py-2 text-meta text-muted-foreground"><span className="font-medium text-foreground">{adapter.label} catalog:</span> {adapter.modelCatalog?.source === "last_known_good" ? "using last-known-good models" : "using curated fallback"}{adapter.modelCatalog?.lastError ? ` · ${adapter.modelCatalog.lastError}` : ""}</div>)}

    {/* Orchestrator: the model the user talks to. A direct choice from whatever
        the provider exposes — no fast/standard/strong tier in sight. */}
    <section className="space-y-3">
      <div>
        <h3 className="text-sm font-medium text-foreground">Orchestrator model</h3>
      </div>
      {orchestrators.map(profile => {
        const selected = options.find(option => option.value === `${profile.provider}:${profile.model}`);
        // A model with no advertised effort levels has no thinking knob (e.g.
        // Claude Haiku): show the levels it does advertise, or hide the control.
        const efforts = advertisedEfforts(selected?.model);
        return <section key={profile.purpose} className="rounded-xl border border-border bg-card p-4">
          <h4 className="mb-3 text-sm font-medium text-foreground">{profileLabels[profile.purpose]}</h4>
          <div className="grid gap-3 @min-[420px]/profiles:grid-cols-2">
            <label className="space-y-1.5 block text-meta font-medium text-muted-foreground">Model
              <SelectField label="Model" value={`${profile.provider}:${profile.model}`} disabled={disabled} onChange={value => { const option = options.find(candidate => candidate.value === value); if (option) update(profile.purpose, { provider: option.adapter.id, model: option.model.id, effort: normalizedEffort(profile.effort, option.model), selectionMode: "pinned", pinned: true, learningEnabled: false }); }} options={modelChoices} />
            </label>
            {efforts.length > 0 && <label className="space-y-1.5 block text-meta font-medium text-muted-foreground">Thinking
              <SelectField label="Thinking" value={profile.effort} disabled={disabled} onChange={value => update(profile.purpose, { effort: value as ReasoningEffort })} options={efforts.map(effort => ({ value: effort, label: effort }))} />
            </label>}
          </div>
        </section>;
      })}
    </section>

    {/* Worker roles: delegated agents, routed by capability tier. */}
    <section className="space-y-3">
      <div>
        <h3 className="text-sm font-medium text-foreground">Worker roles</h3>
        <p className="mt-0.5 text-ui leading-relaxed text-muted-foreground">Choose automatically to let Bridge pick a model suited to each worker. Choosing a provider and model below switches that worker to Use a specific model.</p>
      </div>
      {workers.map(profile => {
      const selectionMode = profile.selectionMode ?? (profile.pinned ? "pinned" : "track_standard");
      return <section key={profile.purpose} className="rounded-xl border border-border bg-card p-4">
      <div className="mb-3 flex flex-wrap items-center justify-between gap-x-3 gap-y-1.5">
        <div className="min-w-0"><h3 className="text-sm font-medium text-foreground">{profileLabels[profile.purpose]}</h3><p className="mt-0.5 text-meta text-muted-foreground">{["reviewer", "evaluator"].includes(profile.purpose) ? "Verification role with a specialized rubric" : "Canonical role profile"}</p></div>
        <label className="flex items-center gap-2 text-ui text-muted-foreground"><Checkbox checked={profile.learningEnabled} disabled={disabled || selectionMode === "pinned"} onCheckedChange={checked => update(profile.purpose, { learningEnabled: checked === true })} />Allow learning</label>
      </div>
      <div className="grid gap-3 @min-[420px]/profiles:grid-cols-2 @min-[720px]/profiles:grid-cols-3">
        <label className="space-y-1.5 block text-meta font-medium text-muted-foreground">How Bridge chooses a model
          <SelectField label="How Bridge chooses a model" value={selectionMode} disabled={disabled} onChange={value => { const mode = value as "track_standard" | "pinned"; update(profile.purpose, { selectionMode: mode, pinned: mode === "pinned", learningEnabled: mode === "pinned" ? false : profile.learningEnabled }); }} options={[{ value: "track_standard", label: "Choose automatically" }, { value: "pinned", label: "Use a specific model" }]} />
        </label>
        <label className="space-y-1.5 block text-meta font-medium text-muted-foreground">Provider & model
          <SelectField label="Provider & model" value={`${profile.provider}:${profile.model}`} disabled={disabled} options={modelChoices} onChange={value => {
            const selected = options.find(option => option.value === value);
            if (selected) update(profile.purpose, {
              provider: selected.adapter.id, model: selected.model.id,
              effort: normalizedEffort(profile.effort, selected.model),
              selectionMode: "pinned", pinned: true, learningEnabled: false,
            });
          }} />
        </label>
        <label className="space-y-1.5 block text-meta font-medium text-muted-foreground">Reasoning effort
          <SelectField label="Reasoning effort" value={profile.effort} disabled={disabled} onChange={value => update(profile.purpose, { effort: value as ReasoningEffort })} options={(["low", "medium", "high", "xhigh"] as ReasoningEffort[]).map(effort => ({ value: effort, label: effort }))} />
        </label>
        <label className="space-y-1.5 block text-meta font-medium text-muted-foreground">Fallback profile
          <SelectField label="Fallback profile" value={profile.fallbackPurpose ?? ""} disabled={disabled} onChange={value => update(profile.purpose, { fallbackPurpose: (value || null) as ProfilePurpose | null })} options={[{ value: "", label: "Catalog default" }, ...profilePurposes.filter(purpose => purpose !== profile.purpose).map(purpose => ({ value: purpose, label: profileLabels[purpose] }))]} />
        </label>
        <label className="space-y-1.5 block text-meta font-medium text-muted-foreground">Budget preference
          <SelectField label="Budget preference" value={profile.budgetPreference ?? ""} disabled={disabled} onChange={value => update(profile.purpose, { budgetPreference: value || null })} options={[{ value: "", label: "Balanced" }, { value: "economy", label: "Economy" }, { value: "quality", label: "Quality first" }]} />
        </label>
        <label className="space-y-1.5 block text-meta font-medium text-muted-foreground">Latency preference
          <SelectField label="Latency preference" value={profile.latencyPreference ?? ""} disabled={disabled} onChange={value => update(profile.purpose, { latencyPreference: value || null })} options={[{ value: "", label: "Balanced" }, { value: "fast", label: "Low latency" }, { value: "patient", label: "Patient" }]} />
        </label>
      </div>
    </section>;})}
    </section>
  </div>;
}
