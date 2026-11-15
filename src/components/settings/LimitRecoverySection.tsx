import { useEffect, useState } from "react";
import { SkeletonRows } from "@/components/ui/skeleton";
import { bridgeApi } from "../../api";
import type { AdapterDescriptor } from "../../types";
import type { Effort, LimitRecoveryMode, LimitRecoverySettings, LimitRecoveryTarget } from "../../protocol/generated/protocol";
import { Select, SettingsGroup, SettingsRow } from "./kit";

const MODES: ReadonlyArray<{ value: LimitRecoveryMode; label: string; description: string }> = [
  { value: "off", label: "Do nothing", description: "The chat stops and waits for you." },
  { value: "resume", label: "Resume when the limit resets", description: "Bridge waits for the reset time the provider gives, then continues the chat on the same model and effort." },
  { value: "backup", label: "Switch to a backup model", description: "Bridge switches the chat to your backup model right away and continues. If the backup runs out too, it resumes when the backup's limit resets." },
];

const STORABLE_EFFORTS = ["low", "medium", "high", "xhigh"] as const satisfies readonly Effort[];

const targetValue = (target: Pick<LimitRecoveryTarget, "harness" | "model">) => JSON.stringify({ harness: target.harness ?? "", model: target.model ?? null });

/** Holds the form's draft; a backup mode without a backup stays a draft so
 * the server never sees a setting it would refuse. */
type Draft = { mode: LimitRecoveryMode; backup: LimitRecoveryTarget | null };
const normalize = (settings: LimitRecoverySettings): Draft => ({ mode: settings.mode ?? "off", backup: settings.backup?.harness ? settings.backup : null });

/** What a chat does when its provider says the usage limit is reached. One
 * setting for every chat. It works while this Mac is awake and Bridge is
 * running; Bridge keeps the Mac from idle-sleeping while a chat waits. */
export function LimitRecoverySection({ adapters }: { adapters: AdapterDescriptor[] }) {
  const [draft, setDraft] = useState<Draft>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [saved, setSaved] = useState(false);
  useEffect(() => {
    let active = true;
    void bridgeApi.limitRecoverySettings()
      .then(settings => { if (active) setDraft(normalize(settings)); })
      .catch(cause => { if (active) setError(String(cause)); });
    return () => { active = false; };
  }, []);
  const commit = async (next: Draft) => {
    setDraft(next); setSaved(false); setError(undefined);
    if (next.mode === "backup" && !next.backup) return;
    setBusy(true);
    try {
      setDraft(normalize(await bridgeApi.saveLimitRecoverySettings({ mode: next.mode, backup: next.backup })));
      setSaved(true);
    } catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); }
    finally { setBusy(false); }
  };
  if (error && !draft) return <p role="alert" className="text-ui text-destructive">{error}</p>;
  if (!draft) return <SkeletonRows label="Loading usage limit settings" rows={2} trailing />;
  const mode = MODES.find(option => option.value === draft.mode) ?? MODES[0];
  const runnable = adapters.filter(adapter => adapter.available);
  const modelOptions = runnable.flatMap(adapter => [
    ...(adapter.models.length === 0 ? [{ value: targetValue({ harness: adapter.id, model: null }), label: `${adapter.label} · Provider default` }] : []),
    ...adapter.models.map(model => ({ value: targetValue({ harness: adapter.id, model: model.id }), label: `${adapter.label} · ${model.label}`, disabled: model.available === false || model.compatible === false })),
  ]);
  const backupValue = draft.backup ? targetValue(draft.backup) : "";
  if (draft.backup && !modelOptions.some(option => option.value === backupValue)) {
    modelOptions.push({ value: backupValue, label: `${draft.backup.harness} · ${draft.backup.model ?? "default"} (unavailable)`, disabled: true });
  }
  const backupAdapter = adapters.find(adapter => adapter.id === draft.backup?.harness);
  // Only the levels the setting can store; a model's extra levels (Claude's
  // "max") are chat-only for now.
  const efforts = (backupAdapter?.models.find(model => model.id === (draft.backup?.model ?? backupAdapter.defaultModel))?.supportedEffortLevels ?? [])
    .filter(level => (STORABLE_EFFORTS as readonly string[]).includes(level));
  return <SettingsGroup label="When a usage limit stops a chat" note="Works while this Mac is awake and Bridge is open.">
    {error && <p role="alert" className="px-3 py-2 text-ui text-destructive">{error}</p>}
    <SettingsRow label="Action" description={mode.description} saved={saved && draft.mode !== "backup"} control={
      <Select label="When a usage limit stops a chat" disabled={busy} width="w-64" value={draft.mode} options={MODES.map(option => ({ value: option.value, label: option.label }))} onChange={value => void commit({ ...draft, mode: value as LimitRecoveryMode })} />
    } />
    {draft.mode === "backup" && <>
      <SettingsRow label="Backup model" description={draft.backup ? "The chat continues on this model until you switch it back." : "Choose a backup model to turn this on."} saved={saved} control={
        <Select label="Backup model" disabled={busy} width="w-64" placeholder="Choose a model" value={backupValue} options={modelOptions} onChange={value => {
          const picked = JSON.parse(value) as { harness: string; model: string | null };
          void commit({ ...draft, backup: { harness: picked.harness, model: picked.model, effort: null } });
        }} />
      } />
      {draft.backup && <SettingsRow label="Backup effort" description={efforts.length ? "Thinking effort on the backup model." : "This model has no thinking control."} control={
        <Select label="Backup effort" disabled={busy || efforts.length === 0} width="w-40" value={draft.backup.effort ?? ""} options={[{ value: "", label: "Model default" }, ...efforts.map(level => ({ value: level, label: level }))]} onChange={value => void commit({ ...draft, backup: { ...draft.backup!, effort: (value || null) as Effort | null } })} />
      } />}
    </>}
  </SettingsGroup>;
}
