import { useEffect, useRef, useState } from "react";
import { bridgeApi } from "../../api";
import { layerChatDefaults } from "../../chatDefaults";
import type { AdapterDescriptor, AgentDefinition, Project } from "../../types";
import type { ChatDefaults, ChatDefaultsView, ChatLaunchTarget } from "../../protocol/generated/protocol";
import { SettingsGroup, SettingsPage, SettingsRow, TextButton, Select } from "./kit";

const INHERIT = "inherit";
function targetValue(target: ChatLaunchTarget | null | undefined) {
  return target?.kind === "agent" ? `agent:${target.agentId}` : target?.kind ?? INHERIT;
}
const modelValue = (model: NonNullable<ChatDefaults["model"]>) => JSON.stringify({ harness: model.harness, model: model.model ?? null });
type Choice = { value: string; label: string; disabled?: boolean };
/** The select value for a stored field, so inherited values read with the same labels as the menus. */
function selectValue(field: keyof ChatDefaults, defaults: ChatDefaults): string {
  if (defaults[field] == null) return INHERIT;
  if (field === "target") return targetValue(defaults.target);
  if (field === "model") return modelValue(defaults.model!);
  if (field === "effort") return defaults.effort!.value ?? "provider";
  return String(defaults[field]);
}
export function DefaultsSelect({ label, value, options, disabled, onChange }: {
  label: string; value: string; options: Array<{ value: string; label: string; disabled?: boolean }>; disabled?: boolean; onChange: (value: string) => void;
}) {
  const visible = options.some(option => option.value === value) ? options : [...options, { value, label: `Unavailable: ${value}`, disabled: true }];
  return <Select label={label} value={value} disabled={disabled} options={visible} onChange={onChange} />;
}

export function ChatDefaultsPage({ adapters, agents, onOpenAgents }: { adapters: AdapterDescriptor[]; agents: AgentDefinition[]; onOpenAgents?: () => void }) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [scope, setScope] = useState("");
  const [view, setView] = useState<ChatDefaultsView>();
  const [draft, setDraft] = useState<ChatDefaults>({});
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState<string>();
  const [saved, setSaved] = useState(false);
  const [reload, setReload] = useState(0);
  const generation = useRef(0);
  useEffect(() => {
    const request = ++generation.current; setBusy(true); setView(undefined); setError(undefined); setSaved(false);
    void Promise.all([bridgeApi.state(), bridgeApi.getChatDefaults(scope || null)]).then(([state, next]) => {
      if (generation.current !== request) return;
      setProjects(state.projects); setView(next); setDraft(structuredClone((scope ? next.project : next.application)?.defaults ?? {}));
    }).catch(error => { if (generation.current === request) setError(String(error)); })
      .finally(() => { if (generation.current === request) setBusy(false); });
    return () => { generation.current++; };
  }, [scope, reload]);
  const change = (patch: ChatDefaults) => { setDraft(current => layerChatDefaults(current, patch)); setSaved(false); };
  const inherit = (field: keyof ChatDefaults) => { setDraft(current => { const next = { ...current }; delete next[field]; return next; }); setSaved(false); };
  const setField = <K extends keyof ChatDefaults>(field: K, value: string, parse: (value: string) => ChatDefaults[K]) => value === INHERIT ? inherit(field) : change({ [field]: parse(value) });
  const effective = layerChatDefaults(scope ? view?.application.defaults ?? {} : {}, draft);
  const model = effective.model;
  const adapter = adapters.find(a => a.id === model?.harness);
  const efforts = adapter?.models.find(m => m.id === (model?.model ?? adapter.defaultModel))?.supportedEffortLevels ?? [];
  const choices: Record<keyof ChatDefaults, Choice[]> = {
    target: [{ value: "direct", label: "Direct chat" }, { value: "orchestrator", label: "Orchestrator" }, ...agents.filter(a => a.enabled && a.role !== "orchestrator").map(a => ({ value: `agent:${a.id}`, label: a.name }))],
    model: adapters.filter(a => a.available).flatMap(a => [{ value: modelValue({ harness: a.id, model: null }), label: `${a.label} · Provider default` }, ...a.models.map(m => ({ value: modelValue({ harness: a.id, model: m.id }), label: `${a.label} · ${m.label}`, disabled: m.available === false || m.compatible === false }))]),
    effort: [{ value: "provider", label: "Provider default" }, ...efforts.map(value => ({ value, label: value }))],
    access: [{ value: "ask", label: "User approval" }, { value: "full", label: "Full access" }],
    createWorktree: [{ value: "false", label: "Current project checkout" }, { value: "true", label: "New isolated worktree" }],
    activeTurnInput: [{ value: "steer", label: "Steer" }, { value: "queue", label: "Queue" }],
  };
  const source = (field: keyof ChatDefaults) => {
    if (draft[field] != null) return scope ? "This project" : "Application default";
    const inherited = selectValue(field, view?.application.defaults ?? {});
    if (!scope || inherited === INHERIT) return "Uses the selected agent or Bridge default";
    return `Inherits application default: ${choices[field].find(choice => choice.value === inherited)?.label ?? inherited}`;
  };
  const save = async (defaults: ChatDefaults) => {
    if (!view) return;
    setBusy(true); setError(undefined); setSaved(false);
    const request = generation.current;
    try {
      const next = await bridgeApi.saveChatDefaults({ projectId: scope || null, expectedRevision: (scope ? view.project : view.application)?.revision ?? 0, defaults });
      if (generation.current !== request) return;
      setView(next); setDraft(structuredClone((scope ? next.project : next.application)?.defaults ?? {})); setSaved(true);
    } catch (error) { if (generation.current === request) setError(error instanceof Error ? error.message : String(error)); }
    finally { if (generation.current === request) setBusy(false); }
  };
  const inherited = { value: INHERIT, label: scope ? "Inherit application default" : "Use agent or Bridge default" };
  return <SettingsPage title="New chat defaults" description="Choose how new chats start. Each chat can change its own settings; saved defaults affect future chats.">
    <SettingsGroup label="Scope">
      <SettingsRow label="Apply defaults to" description="Project settings belong to this Bridge project on this device." control={<DefaultsSelect label="Defaults scope" value={scope} options={[{ value: "", label: "All projects (application default)" }, ...projects.map(p => ({ value: p.id, label: p.name }))]} onChange={setScope} />} />
    </SettingsGroup>
    {error && <div role="alert" className="text-ui text-destructive">{error} <TextButton onClick={() => setReload(value => value + 1)}>Reload defaults</TextButton></div>}
    {!view ? <p role="status" className="text-ui text-muted-foreground">{busy ? "Loading defaults…" : "Defaults could not be loaded."}</p> : <>
      <SettingsGroup label="When a chat starts">
        <SettingsRow label="Agent or chat mode" description={source("target")} control={<DefaultsSelect label="Default chat target" disabled={busy} value={selectValue("target", draft)} options={[inherited, ...choices.target]} onChange={value => setField("target", value, value => value.startsWith("agent:") ? { kind: "agent", agentId: value.slice(6) } : { kind: value as "direct" | "orchestrator" })} />} />
        <SettingsRow label="Model" description={source("model")} control={<DefaultsSelect label="Default chat model" disabled={busy} value={selectValue("model", draft)} options={[inherited, ...choices.model]} onChange={value => {
          if (value === INHERIT) inherit("model");
          else { const model = JSON.parse(value) as NonNullable<ChatDefaults["model"]>; change({ model, effort: { value: null } }); }
        }} />} />
        <SettingsRow label="Thinking effort" description={efforts.length ? source("effort") : "This model has no advertised thinking control. Provider default is available."} control={<DefaultsSelect label="Default thinking effort" disabled={busy} value={selectValue("effort", draft)} options={[inherited, ...choices.effort]} onChange={value => setField("effort", value, value => ({ value: value === "provider" ? null : value }))} />} />
        <SettingsRow label="Access" description={`${source("access")}. User approval keeps provider approval checks; Full access grants ordinary provider approvals. Browser and other special grants remain separate.`} control={<DefaultsSelect label="Default chat access" disabled={busy} value={selectValue("access", draft)} options={[inherited, ...choices.access]} onChange={value => setField("access", value, value => value as "ask" | "full")} />} />
        <SettingsRow label="Work location" description={`${source("createWorktree")}. An isolated worktree is created only when the first message is sent.`} control={<DefaultsSelect label="Default work location" disabled={busy} value={selectValue("createWorktree", draft)} options={[inherited, ...choices.createWorktree]} onChange={value => setField("createWorktree", value, value => value === "true")} />} />
        <SettingsRow label="Input while running" description={`${source("activeTurnInput")}. Providers without live steer queue input until a safe boundary.`} control={<DefaultsSelect label="Default input while running" disabled={busy} value={selectValue("activeTurnInput", draft)} options={[inherited, ...choices.activeTurnInput]} onChange={value => setField("activeTurnInput", value, value => value as "steer" | "queue")} />} />
      </SettingsGroup>
      <SettingsGroup label="Instructions">
        <SettingsRow label="Agent instructions" description="Use your existing agents to configure reusable instructions. Project defaults select the agent by its saved identity." onOpen={onOpenAgents} />
      </SettingsGroup>
      <div className="flex flex-wrap items-center gap-3">
        <button type="button" disabled={busy} onClick={() => void save(draft)} className="h-8 rounded-lg bg-primary px-3 text-ui font-medium text-primary-foreground disabled:opacity-45">Save defaults</button>
        <TextButton disabled={busy} onClick={() => void save({})}>Reset to inherited defaults</TextButton>
        {saved && <span role="status" className="text-ui text-muted-foreground">Saved</span>}
      </div>
    </>}
  </SettingsPage>;
}
