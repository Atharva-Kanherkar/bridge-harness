import { useEffect, useRef, useState } from "react";
import { SkeletonRows } from "@/components/ui/skeleton";
import { bridgeApi } from "../../api";
import { friendlyError } from "../../errors";
import type { AdapterDescriptor, Workspace } from "../../types";
import type { WorkerSettings } from "../../protocol/generated/protocol";
import { RouterSettingsDialog } from "../RouterSettingsDialog";
import { GhostButton, NumberField, Select, SettingsGroup, SettingsPage, SettingsRow, Switch, useSavedFlash } from "./kit";
import { DelegationNotifySetting } from "./DelegationNotifySetting";
import { PAGES } from "./sections";

const LIMITS = [
  ["maxConcurrentWorkers", "Tasks running at once", 0, 64, "Per workspace. 0 means no limit."],
  ["maxWorkersPerTurn", "Tasks per message", 0, 64, "0 means no limit."],
  ["stallTimeoutSeconds", "Stop waiting after (seconds)", 60, 7200, "Applies when an agent produces no output. Time waiting for your approval does not count."],
  ["warmRetentionMinutes", "Keep completed agents ready (minutes)", 0, 60, "Set to 0 to close an agent as soon as its task finishes."],
] as const;

/**
 * Every control saves on change. Saves run one after another in the order
 * they were made, and only the latest answer is drawn, so a quick second
 * change cannot be overwritten by the first one's reply.
 */
export function WorkersPage({ adapters }: { adapters: AdapterDescriptor[] }) {
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [workspace, setWorkspace] = useState("");
  const [settings, setSettings] = useState<WorkerSettings>();
  const [error, setError] = useState<string>();
  const [router, setRouter] = useState(false);
  const [isFlashed, flash] = useSavedFlash();
  const queue = useRef<Promise<void>>(Promise.resolve());
  const latest = useRef(0);
  useEffect(() => {
    let active = true;
    void bridgeApi.state().then(state => { if (active) { setWorkspaces(state.workspaces); setWorkspace(state.workspaces[0]?.id ?? ""); } }).catch(cause => { if (active) setError(friendlyError(cause, "Couldn't load your workspaces.")); });
    return () => { active = false; };
  }, []);
  useEffect(() => {
    if (!workspace) return;
    let active = true;
    setSettings(undefined); setError(undefined);
    void bridgeApi.workerSettings(workspace).then(value => { if (active) setSettings(value); }).catch(cause => { if (active) setError(friendlyError(cause, "Couldn't load the task settings.")); });
    return () => { active = false; };
  }, [workspace]);

  const persist = (patch: Partial<WorkerSettings>, key: string) => {
    if (!settings) return;
    const next = { ...settings, ...patch };
    setSettings(next); setError(undefined);
    const request = ++latest.current;
    const target = workspace;
    queue.current = queue.current.then(async () => {
      try {
        const stored = await bridgeApi.saveWorkerSettings(target, next);
        if (request === latest.current) { setSettings(stored); flash(key); }
      } catch (cause) {
        if (request !== latest.current) return;
        setError(friendlyError(cause, "Couldn't save the task settings."));
        // Show what the host actually kept, not the refused value.
        setSettings(await bridgeApi.workerSettings(target).catch(() => next));
      }
    });
  };

  return <SettingsPage title={PAGES.workers.label} description="Control how many tasks Bridge can run at once in this workspace, and what happens when an agent stops responding.">
    {error && <p role="alert" className="text-ui text-destructive">{error}</p>}
    <SettingsGroup><SettingsRow label="Workspace" control={<Select label="Background tasks workspace" value={workspace} onChange={setWorkspace} options={workspaces.map(item => ({ value: item.id, label: item.title }))} />} /></SettingsGroup>
    {!workspace && <p className="text-ui text-muted-foreground">Connect a workspace to configure background tasks. These controls do not apply to direct chats with a coding agent.</p>}
    {workspace && !settings && !error && <SkeletonRows label="Loading task settings" rows={4} trailing />}
    {settings && <>
      <SettingsGroup label="Agent choices" note="Model preferences and choices for a specific task override this default.">
        <SettingsRow anchor="preferred-agent" label="Preferred coding agent" saved={isFlashed("defaultHarness")} control={<Select label="Preferred coding agent" value={settings.defaultHarness ?? ""} onChange={value => persist({ defaultHarness: value || null }, "defaultHarness")} options={[{ value: "", label: "Automatic" }, ...adapters.filter(item => ["codex", "claude", "opencode"].includes(item.id)).map(item => ({ value: item.id, label: `${item.label}${item.available ? "" : " (unavailable)"}` }))]} />} />
        <SettingsRow label="Try another agent at a usage limit" description="When an agent reaches a provider limit, try an available alternative once. Retries share this one-attempt limit." saved={isFlashed("providerFailover")} control={<Switch label="Try another agent at a usage limit" checked={settings.providerFailover} onChange={value => persist({ providerFailover: value }, "providerFailover")} />} />
        <SettingsRow label="Automatic retry" description="Retry a temporary error once while the agent is still running. Do not retry when it stops responding or returns an unreadable result." saved={isFlashed("automaticRetry")} control={<Switch label="Automatic retry" checked={settings.automaticRetry} onChange={value => persist({ automaticRetry: value }, "automaticRetry")} />} />
        <SettingsRow label="Advanced agent selection" description="Exclude an agent from background tasks while keeping it available for chats." control={<GhostButton onClick={() => setRouter(true)}>Open</GhostButton>} />
      </SettingsGroup>
      <SettingsGroup anchor="task-limits" label="Task limits" note="Changes apply to new tasks. Lowering the limit does not stop tasks already running.">
        {LIMITS.map(([key, label, min, max, description]) => <SettingsRow
          key={key}
          label={label}
          description={`${description} Between ${min} and ${max}.`}
          saved={isFlashed(key)}
          control={<NumberField label={label} min={min} max={max} value={settings[key]} onChange={value => persist({ [key]: value }, key)} />}
        />)}
      </SettingsGroup>
    </>}
    {router && <RouterSettingsDialog open workspaceId={workspace} adapters={adapters} onClose={() => setRouter(false)} onError={setError} />}
    <DelegationNotifySetting />
  </SettingsPage>;
}
