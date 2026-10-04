import { useEffect, useState } from "react";
import { bridgeApi } from "../../api";
import type { AdapterDescriptor, Workspace } from "../../types";
import type { WorkerSettings } from "../../protocol/generated/protocol";
import { RouterSettingsDialog } from "../RouterSettingsDialog";
import { GhostButton, Select, SettingsGroup, SettingsPage, SettingsRow, Switch } from "./kit";
import { DelegationNotifySetting } from "./DelegationNotifySetting";
import { ReviewerSettingsSection } from "./ReviewerSettingsSection";

export function WorkersPage({ adapters }: { adapters: AdapterDescriptor[] }) {
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [workspace, setWorkspace] = useState("");
  const [settings, setSettings] = useState<WorkerSettings>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [saved, setSaved] = useState(false);
  const [router, setRouter] = useState(false);
  useEffect(() => {
    let active = true;
    void bridgeApi.state().then(state => { if (active) { setWorkspaces(state.workspaces); setWorkspace(state.workspaces[0]?.id ?? ""); } }).catch(cause => { if (active) setError(String(cause)); });
    return () => { active = false; };
  }, []);
  useEffect(() => {
    if (!workspace) return;
    let active = true;
    setSettings(undefined); setError(undefined); setSaved(false);
    void bridgeApi.workerSettings(workspace).then(value => { if (active) setSettings(value); }).catch(cause => { if (active) setError(String(cause)); });
    return () => { active = false; };
  }, [workspace]);
  const save = async () => {
    if (!settings) return;
    setBusy(true); setError(undefined); setSaved(false);
    try { setSettings(await bridgeApi.saveWorkerSettings(workspace, settings)); setSaved(true); }
    catch (cause) { setError(String(cause)); }
    finally { setBusy(false); }
  };
  const change = (patch: Partial<WorkerSettings>) => { setSaved(false); setSettings(current => current ? { ...current, ...patch } : current); };
  return <SettingsPage title="Background tasks" description="Control how many tasks Bridge can run at once in this workspace, and what happens when an agent stops responding.">
    {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
    <SettingsGroup><SettingsRow label="Workspace" control={<Select label="Background tasks workspace" disabled={busy} value={workspace} onChange={setWorkspace} options={workspaces.map(item => ({ value: item.id, label: item.title }))} />} /></SettingsGroup>
    {!workspace && <p className="text-sm text-muted-foreground">Connect a workspace to configure background tasks. These controls do not apply to direct chats with a coding agent.</p>}
    {workspace && !settings && !error && <p role="status" className="text-sm text-muted-foreground">Loading task settings...</p>}
    {settings && <form onSubmit={event => { event.preventDefault(); void save(); }} className="space-y-5">
      <SettingsGroup label="Agent choices" note="Model preferences and choices for a specific task override this default. Advanced settings can exclude an agent from background tasks while keeping it available for chats.">
        <SettingsRow label="Preferred coding agent" control={<Select label="Preferred coding agent" disabled={busy} value={settings.defaultHarness ?? ""} onChange={value => change({ defaultHarness: value || null })} options={[{ value: "", label: "Automatic" }, ...adapters.filter(item => ["codex", "claude", "opencode"].includes(item.id)).map(item => ({ value: item.id, label: `${item.label}${item.available ? "" : " (unavailable)"}` }))]} />} />
        <SettingsRow label="Try another agent at a usage limit" description="When an agent reaches a provider limit, try an available alternative once. Retries share this one-attempt limit." control={<Switch label="Try another agent at a usage limit" checked={settings.providerFailover} disabled={busy} onChange={value => change({ providerFailover: value })} />} />
        <SettingsRow label="Automatic retry" description="Retry a temporary error once while the agent is still running. Do not retry when it stops responding or returns an unreadable result." control={<Switch label="Automatic retry" checked={settings.automaticRetry} disabled={busy} onChange={value => change({ automaticRetry: value })} />} />
      </SettingsGroup>
      <SettingsGroup label="Task limits" note="Changes apply to new tasks. Lowering the limit does not stop tasks already running.">
        {([
          ["maxConcurrentWorkers", "Tasks running at once", 1, 16, "Per workspace"],
          ["maxWorkersPerTurn", "Tasks per message", 1, 16, "Other task budgets still apply"],
          ["stallTimeoutSeconds", "Stop waiting after (seconds)", 60, 7200, "Applies when an agent produces no output. Time waiting for your approval does not count."],
          ["warmRetentionMinutes", "Keep completed agents ready (minutes)", 0, 60, "Set to 0 to close an agent as soon as its task finishes."],
        ] as const).map(([key, label, min, max, description]) => <SettingsRow key={key} label={label} description={description} control={<input aria-label={label} type="number" required min={min} max={max} step={1} disabled={busy} value={settings[key]} onChange={event => change({ [key]: Number(event.target.value) })} className="h-8 w-24 rounded-lg border border-border bg-card px-2 text-sm tabular-nums" />} />)}
      </SettingsGroup>
      <div className="flex flex-wrap items-center gap-3"><button type="submit" disabled={busy} className="rounded-lg bg-primary px-4 py-2 text-xs font-medium text-primary-foreground disabled:opacity-50">{busy ? "Saving..." : "Save task settings"}</button>{saved && <span role="status" className="text-xs text-muted-foreground">Saved</span>}<GhostButton disabled={busy} onClick={() => setRouter(true)}>Advanced agent selection</GhostButton></div>
    </form>}
    {router && <RouterSettingsDialog open workspaceId={workspace} adapters={adapters} onClose={() => setRouter(false)} onError={setError} />}
    <DelegationNotifySetting />
    <ReviewerSettingsSection adapters={adapters} />
  </SettingsPage>;
}
