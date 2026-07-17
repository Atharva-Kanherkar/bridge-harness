import type { AdapterDescriptor, ModelProfileDraft, ProfilePurpose, ReasoningEffort } from "../types";
import { availableModelOptions, profileLabels, profilePurposes } from "../modelProfiles";

const fieldClass = "h-9 w-full rounded-xl border border-white/[0.09] bg-white/[0.04] px-3 text-xs text-neutral-200 outline-none transition-colors focus:border-white/[0.18] disabled:opacity-45";

export function ModelProfileEditor({ profiles, adapters, disabled, onChange }: {
  profiles: ModelProfileDraft[];
  adapters: AdapterDescriptor[];
  disabled?: boolean;
  onChange: (profiles: ModelProfileDraft[]) => void;
}) {
  const options = availableModelOptions(adapters);
  const update = (purpose: ProfilePurpose, patch: Partial<ModelProfileDraft>) => onChange(profiles.map(profile => profile.purpose === purpose ? { ...profile, ...patch } : profile));
  return <div className="space-y-3">
    {profiles.map(profile => <section key={profile.purpose} className="rounded-2xl border border-white/[0.07] bg-white/[0.025] p-3.5">
      <div className="mb-3 flex items-center justify-between gap-3">
        <div><h3 className="text-sm font-medium text-neutral-200">{profileLabels[profile.purpose]}</h3><p className="mt-0.5 text-[10px] text-neutral-600">{["reviewer", "evaluator"].includes(profile.purpose) ? "Verification role with a specialized rubric" : "Canonical role profile"}</p></div>
        <label className="flex items-center gap-2 text-[11px] text-neutral-500"><input type="checkbox" checked={profile.learningEnabled} disabled={disabled || profile.pinned} onChange={event => update(profile.purpose, { learningEnabled: event.target.checked })} />Allow learning</label>
      </div>
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
        <label className="space-y-1.5 text-[10px] font-semibold uppercase tracking-wider text-neutral-600">Provider & model
          <select className={fieldClass} value={`${profile.provider}:${profile.model}`} disabled={disabled} onChange={event => { const selected = options.find(option => option.value === event.target.value); if (selected) update(profile.purpose, { provider: selected.adapter.id, model: selected.model.id }); }}>
            {options.map(option => <option key={option.value} value={option.value}>{option.adapter.label} · {option.model.label}</option>)}
          </select>
        </label>
        <label className="space-y-1.5 text-[10px] font-semibold uppercase tracking-wider text-neutral-600">Reasoning effort
          <select className={fieldClass} value={profile.effort} disabled={disabled} onChange={event => update(profile.purpose, { effort: event.target.value as ReasoningEffort })}>
            {(["low", "medium", "high", "xhigh"] as ReasoningEffort[]).map(effort => <option key={effort} value={effort}>{effort}</option>)}
          </select>
        </label>
        <label className="space-y-1.5 text-[10px] font-semibold uppercase tracking-wider text-neutral-600">Fallback profile
          <select className={fieldClass} value={profile.fallbackPurpose ?? ""} disabled={disabled} onChange={event => update(profile.purpose, { fallbackPurpose: (event.target.value || null) as ProfilePurpose | null })}>
            <option value="">Catalog default</option>{profilePurposes.filter(purpose => purpose !== profile.purpose).map(purpose => <option key={purpose} value={purpose}>{profileLabels[purpose]}</option>)}
          </select>
        </label>
        <label className="space-y-1.5 text-[10px] font-semibold uppercase tracking-wider text-neutral-600">Budget preference
          <select className={fieldClass} value={profile.budgetPreference ?? ""} disabled={disabled} onChange={event => update(profile.purpose, { budgetPreference: event.target.value || null })}><option value="">Balanced</option><option value="economy">Economy</option><option value="quality">Quality first</option></select>
        </label>
        <label className="space-y-1.5 text-[10px] font-semibold uppercase tracking-wider text-neutral-600">Latency preference
          <select className={fieldClass} value={profile.latencyPreference ?? ""} disabled={disabled} onChange={event => update(profile.purpose, { latencyPreference: event.target.value || null })}><option value="">Balanced</option><option value="fast">Low latency</option><option value="patient">Patient</option></select>
        </label>
        <label className="flex items-end pb-2 text-[11px] text-neutral-500"><span className="flex items-center gap-2"><input type="checkbox" checked={profile.pinned} disabled={disabled} onChange={event => update(profile.purpose, { pinned: event.target.checked, learningEnabled: event.target.checked ? false : profile.learningEnabled })} />Pin this profile</span></label>
      </div>
    </section>)}
  </div>;
}
