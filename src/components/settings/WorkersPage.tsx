import { useEffect, useState } from "react";
import { bridgeApi } from "../../api";
import type { AdapterDescriptor, Workspace } from "../../types";
import type { WorkerSettings } from "../../protocol/generated/protocol";
import { RouterSettingsDialog } from "../RouterSettingsDialog";
import { GhostButton, Select, SettingsGroup, SettingsPage, SettingsRow, Switch } from "./kit";

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
  return <SettingsPage title="Workers" description="Operational limits for delegated work, scoped to one workspace. Permissions and independent verification remain enforced.">
    {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
    <SettingsGroup><SettingsRow label="Workspace" control={<Select label="Worker settings workspace" disabled={busy} value={workspace} onChange={setWorkspace} options={workspaces.map(item => ({ value: item.id, label: item.title }))} />} /></SettingsGroup>
    {!workspace && <p className="text-sm text-muted-foreground">Connect a workspace to configure delegated workers. Direct chats do not delegate.</p>}
    {workspace && !settings && !error && <p role="status" className="text-sm text-muted-foreground">Loading worker settings...</p>}
    {settings && <form onSubmit={event => { event.preventDefault(); void save(); }} className="space-y-5">
      <SettingsGroup label="Routing" note="Role model profiles and explicit route constraints take precedence over this default. Advanced routing can exclude a harness for workers without disabling it for chats.">
        <SettingsRow label="Default worker harness" control={<Select label="Default worker harness" disabled={busy} value={settings.defaultHarness ?? ""} onChange={value => change({ defaultHarness: value || null })} options={[{ value: "", label: "Automatic" }, ...adapters.filter(item => ["codex", "claude", "opencode"].includes(item.id)).map(item => ({ value: item.id, label: `${item.label}${item.available ? "" : " (unavailable)"}` }))]} />} />
        <SettingsRow label="Provider-limit failover" description="Relaunch on an eligible alternative after an observed provider limit. At most one automatic attempt per objective, shared with retries." control={<Switch label="Provider-limit failover" checked={settings.providerFailover} disabled={busy} onChange={value => change({ providerFailover: value })} />} />
        <SettingsRow label="Automatic retry" description="Retry a transient failure once on a live process. Never retry a stall or unreadable result." control={<Switch label="Automatic retry" checked={settings.automaticRetry} disabled={busy} onChange={value => change({ automaticRetry: value })} />} />
      </SettingsGroup>
      <SettingsGroup label="Capacity and lifecycle" note="Changes apply to subsequent routing and watchdog checks. Lowering capacity does not stop existing workers.">
        {([
          ["maxConcurrentWorkers", "Concurrent workers", 1, 16, "Per workspace"],
          ["maxWorkersPerTurn", "Workers per turn", 1, 16, "Capability and strong-worker budgets still apply"],
          ["stallTimeoutSeconds", "Stall timeout (seconds)", 60, 7200, "Working with no output; approval waits are not stalls"],
          ["warmRetentionMinutes", "Warm retention (minutes)", 0, 60, "Zero closes completed reusable workers immediately"],
        ] as const).map(([key, label, min, max, description]) => <SettingsRow key={key} label={label} description={description} control={<input aria-label={label} type="number" required min={min} max={max} step={1} disabled={busy} value={settings[key]} onChange={event => change({ [key]: Number(event.target.value) })} className="h-8 w-24 rounded-lg border border-border bg-card px-2 text-sm tabular-nums" />} />)}
      </SettingsGroup>
      <div className="flex flex-wrap items-center gap-3"><button type="submit" disabled={busy} className="rounded-lg bg-primary px-4 py-2 text-xs font-medium text-primary-foreground disabled:opacity-50">{busy ? "Saving..." : "Save worker settings"}</button>{saved && <span role="status" className="text-xs text-muted-foreground">Saved</span>}<GhostButton disabled={busy} onClick={() => setRouter(true)}>Advanced routing and role profiles</GhostButton></div>
    </form>}
    {router && <RouterSettingsDialog open workspaceId={workspace} adapters={adapters} onClose={() => setRouter(false)} onError={setError} />}
  </SettingsPage>;
}
