// Models: one row per role profile, expanded in place.
//
// Six profiles laid out as six-field grids made the page a wall of selects with
// no way to see, at a glance, what each role was actually set to. A row now
// states the answer ("Codex · GPT-5 · high") and opens to the controls that
// produced it, so scanning and editing are different acts.
//
// Every control here persists on change: there is no text on this page, so
// there is no save bar and no header Save. Each write sends the whole profile
// list, because `saveModelProfiles` takes the set, and the one row that changed
// is the only one that differs from what was stored.

import { useState } from "react";
import { ArrowsClockwise, CaretDown, CaretRight } from "@phosphor-icons/react";
import {
  advertisedEfforts, availableModelOptions, isOrchestratorPurpose, normalizedEffort,
  profileLabels, profilePurposes,
} from "../../modelProfiles";
import type { AdapterDescriptor, ModelProfileDraft, ProfilePurpose, ReasoningEffort } from "../../types";
import { HarnessMark } from "../harnessMarks";
import {
  GhostButton, Select, SettingsGroup, SettingsPage, SettingsRow, StatusPill, Switch, useSavedFlash,
} from "./kit";

const VERIFICATION: ProfilePurpose[] = ["reviewer", "evaluator"];

const GROUPS: { label: string; note: string; includes: (purpose: ProfilePurpose) => boolean }[] = [
  { label: "Orchestration", note: "The model you talk to", includes: isOrchestratorPurpose },
  {
    label: "Workers",
    note: "Delegated agents, routed by capability tier",
    includes: purpose => !isOrchestratorPurpose(purpose) && !VERIFICATION.includes(purpose),
  },
  {
    label: "Verification",
    note: "Roles with a specialized rubric",
    includes: purpose => VERIFICATION.includes(purpose),
  },
];

export function ModelsPage({ profiles, adapters, version, busy, onSave, onRefreshCatalogs, onError }: {
  profiles: ModelProfileDraft[];
  adapters: AdapterDescriptor[];
  version: number | string | null;
  busy: boolean;
  onSave: (profiles: ModelProfileDraft[]) => Promise<void>;
  onRefreshCatalogs: () => Promise<void>;
  onError: (message: string) => void;
}) {
  const [expanded, setExpanded] = useState<ProfilePurpose | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [isFlashed, flash] = useSavedFlash();
  const options = availableModelOptions(adapters);
  const stale = adapters.filter(adapter => adapter.modelCatalog?.stale || adapter.modelCatalog?.lastError);

  const update = (purpose: ProfilePurpose, patch: Partial<ModelProfileDraft>) => {
    const next = profiles.map(profile => profile.purpose === purpose ? { ...profile, ...patch } : profile);
    void onSave(next).then(() => flash(purpose)).catch(error =>
      onError(error instanceof Error ? error.message : String(error)));
  };

  const refresh = async () => {
    setRefreshing(true);
    try { await onRefreshCatalogs(); }
    catch (error) { onError(error instanceof Error ? error.message : String(error)); }
    finally { setRefreshing(false); }
  };

  return <SettingsPage
    title="Models"
    description="Which model runs each Bridge role, and how hard it thinks."
    action={<StatusPill>Version {version ?? "none"}</StatusPill>}
  >
    {GROUPS.map(group => {
      const rows = profiles.filter(profile => group.includes(profile.purpose));
      if (rows.length === 0) return null;
      return <SettingsGroup key={group.label} label={group.label} note={group.note}>
        {rows.map(profile => <ProfileRow
          key={profile.purpose}
          profile={profile}
          options={options}
          busy={busy}
          saved={isFlashed(profile.purpose)}
          expanded={expanded === profile.purpose}
          onToggle={() => setExpanded(current => current === profile.purpose ? null : profile.purpose)}
          onUpdate={patch => update(profile.purpose, patch)}
        />)}
      </SettingsGroup>;
    })}

    <SettingsGroup label="Catalog" note="Where the model lists come from">
      {adapters.filter(adapter => adapter.available).map(adapter => {
        const catalog = adapter.modelCatalog;
        const isStale = catalog?.stale || catalog?.lastError;
        return <SettingsRow
          key={adapter.id}
          lead={<HarnessMark harness={adapter.id} size={14} />}
          label={adapter.label}
          description={catalog?.lastError
            ?? (catalog?.source === "last_known_good" ? "Using last-known-good models" : `${adapter.models.length} models`)}
          control={isStale
            ? <>
                <StatusPill tone="warning">Stale</StatusPill>
                <GhostButton disabled={refreshing} onClick={() => void refresh()}>
                  <ArrowsClockwise size={12} weight="regular" aria-hidden="true" className={refreshing ? "animate-spin" : undefined} />Retry
                </GhostButton>
              </>
            : <StatusPill tone="success">Fresh</StatusPill>}
        />;
      })}
      {stale.length === 0 && adapters.every(adapter => !adapter.available) && <SettingsRow label="No provider is available yet." />}
    </SettingsGroup>
  </SettingsPage>;
}

function ProfileRow({ profile, options, busy, saved, expanded, onToggle, onUpdate }: {
  profile: ModelProfileDraft;
  options: ReturnType<typeof availableModelOptions>;
  busy: boolean;
  saved: boolean;
  expanded: boolean;
  onToggle: () => void;
  onUpdate: (patch: Partial<ModelProfileDraft>) => void;
}) {
  const selected = options.find(option => option.value === `${profile.provider}:${profile.model}`);
  const orchestrator = isOrchestratorPurpose(profile.purpose);
  const selectionMode = profile.selectionMode ?? (profile.pinned ? "pinned" : "track_standard");
  // A model with no advertised effort levels has no thinking knob (Claude Haiku,
  // for one): show the levels it does advertise, or hide the control.
  const efforts = advertisedEfforts(selected?.model);
  const modelOptions = options.map(option => ({
    value: option.value,
    label: `${option.adapter.label} · ${option.model.label}`,
    lead: <HarnessMark harness={option.adapter.id} size={12} />,
  }));

  return <div>
    <SettingsRow
      lead={<HarnessMark harness={profile.provider} size={14} />}
      label={profile.purpose in profileLabels ? profileLabels[profile.purpose] : profile.purpose}
      description={`${selected?.adapter.label ?? profile.provider} · ${selected?.model.label ?? profile.model}${efforts.length > 0 ? ` · ${profile.effort}` : ""}`}
      saved={saved}
      control={<>
        {!orchestrator && <StatusPill tone={selectionMode === "pinned" ? "info" : "neutral"}>
          {selectionMode === "pinned" ? "Pinned" : "Tracks standard"}
        </StatusPill>}
        <button
          type="button"
          aria-expanded={expanded}
          aria-label={`${profileLabels[profile.purpose]} settings`}
          onClick={onToggle}
          className="grid size-7 shrink-0 place-items-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        >
          {expanded
            ? <CaretDown size={12} weight="regular" aria-hidden="true" />
            : <CaretRight size={12} weight="regular" aria-hidden="true" />}
        </button>
      </>}
    />
    {expanded && <div className="border-t border-border bg-popover/40">
      {orchestrator
        ? <>
            <SettingsRow
              label="Model"
              description="Any model your providers expose. Workers below are still routed by tier."
              control={<Select
                label={`${profileLabels[profile.purpose]} model`}
                value={`${profile.provider}:${profile.model}`}
                disabled={busy}
                options={modelOptions}
                onChange={value => {
                  const option = options.find(candidate => candidate.value === value);
                  if (!option) return;
                  onUpdate({
                    provider: option.adapter.id,
                    model: option.model.id,
                    effort: normalizedEffort(profile.effort, option.model),
                    selectionMode: "pinned",
                    pinned: true,
                    learningEnabled: false,
                  });
                }}
              />}
            />
            {efforts.length > 0 && <SettingsRow
              label="Thinking"
              control={<Select
                label={`${profileLabels[profile.purpose]} thinking`}
                value={profile.effort}
                disabled={busy}
                width="w-40"
                options={efforts.map(effort => ({ value: effort, label: effort }))}
                onChange={value => onUpdate({ effort: value as ReasoningEffort })}
              />}
            />}
          </>
        : <>
            <SettingsRow
              label="Selection behavior"
              control={<Select
                label={`${profileLabels[profile.purpose]} selection behavior`}
                value={selectionMode}
                disabled={busy}
                width="w-44"
                options={[{ value: "track_standard", label: "Track standard" }, { value: "pinned", label: "Pinned model" }]}
                onChange={value => {
                  const mode = value as "track_standard" | "pinned";
                  onUpdate({
                    selectionMode: mode,
                    pinned: mode === "pinned",
                    learningEnabled: mode === "pinned" ? false : profile.learningEnabled,
                  });
                }}
              />}
            />
            <SettingsRow
              label="Provider and model"
              control={<Select
                label={`${profileLabels[profile.purpose]} model`}
                value={`${profile.provider}:${profile.model}`}
                disabled={busy || selectionMode === "track_standard"}
                options={modelOptions}
                onChange={value => {
                  const option = options.find(candidate => candidate.value === value);
                  if (option) onUpdate({ provider: option.adapter.id, model: option.model.id });
                }}
              />}
            />
            <SettingsRow
              label="Reasoning effort"
              control={<Select
                label={`${profileLabels[profile.purpose]} effort`}
                value={profile.effort}
                disabled={busy}
                width="w-40"
                options={(["low", "medium", "high", "xhigh"] as ReasoningEffort[]).map(effort => ({ value: effort, label: effort }))}
                onChange={value => onUpdate({ effort: value as ReasoningEffort })}
              />}
            />
            <SettingsRow
              label="Fallback profile"
              control={<Select
                label={`${profileLabels[profile.purpose]} fallback`}
                value={profile.fallbackPurpose ?? ""}
                disabled={busy}
                options={[
                  { value: "", label: "Catalog default" },
                  ...profilePurposes.filter(purpose => purpose !== profile.purpose).map(purpose => ({ value: purpose, label: profileLabels[purpose] })),
                ]}
                onChange={value => onUpdate({ fallbackPurpose: (value || null) as ProfilePurpose | null })}
              />}
            />
            <SettingsRow
              label="Budget preference"
              control={<Select
                label={`${profileLabels[profile.purpose]} budget preference`}
                value={profile.budgetPreference ?? ""}
                disabled={busy}
                width="w-44"
                options={[
                  { value: "", label: "Balanced" },
                  { value: "economy", label: "Economy" },
                  { value: "quality", label: "Quality first" },
                ]}
                onChange={value => onUpdate({ budgetPreference: value || null })}
              />}
            />
            <SettingsRow
              label="Latency preference"
              control={<Select
                label={`${profileLabels[profile.purpose]} latency preference`}
                value={profile.latencyPreference ?? ""}
                disabled={busy}
                width="w-44"
                options={[
                  { value: "", label: "Balanced" },
                  { value: "fast", label: "Low latency" },
                  { value: "patient", label: "Patient" },
                ]}
                onChange={value => onUpdate({ latencyPreference: value || null })}
              />}
            />
            <SettingsRow
              label="Allow learning"
              description="Bridge may rank eligible models for this role. It can never widen a scope or skip an approval."
              control={<Switch
                label={`${profileLabels[profile.purpose]} allow learning`}
                checked={profile.learningEnabled}
                disabled={busy || selectionMode === "pinned"}
                onChange={next => onUpdate({ learningEnabled: next })}
              />}
            />
          </>}
    </div>}
  </div>;
}
