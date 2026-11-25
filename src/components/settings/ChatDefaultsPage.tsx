import { useEffect, useRef, useState } from "react";
import { SkeletonRows } from "@/components/ui/skeleton";
import { bridgeApi } from "../../api";
import { layerChatDefaults, WORKER_EFFORTS } from "../../chatDefaults";
import type { AdapterDescriptor, AgentDefinition, Project } from "../../types";
import type { ChatDefaults, ChatDefaultsView, ChatLaunchTarget } from "../../protocol/generated/protocol";
import { Segmented, SettingsGroup, SettingsPage, SettingsRow, TextButton, Select, useSavedFlash } from "./kit";
import { acceptApplicationChatDefaults } from "../../activeTurnSettings";
import { LimitRecoverySection } from "./LimitRecoverySection";
import { PAGES } from "./sections";

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

export function ChatDefaultsPage({ adapters, agents }: { adapters: AdapterDescriptor[]; agents: AgentDefinition[] }) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [scope, setScope] = useState("");
  const [view, setView] = useState<ChatDefaultsView>();
  const [draft, setDraft] = useState<ChatDefaults>({});
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState<string>();
  const [isFlashed, flash] = useSavedFlash();
  const [reload, setReload] = useState(0);
  const generation = useRef(0);
  useEffect(() => {
    const request = ++generation.current; setBusy(true); setView(undefined); setError(undefined);
    void Promise.all([bridgeApi.state(), bridgeApi.getChatDefaults(scope || null)]).then(([state, next]) => {
      if (generation.current !== request) return;
      setProjects(state.projects); setView(next); setDraft(structuredClone((scope ? next.project : next.application)?.defaults ?? {}));
      if (!scope) acceptApplicationChatDefaults(next);
    }).catch(error => { if (generation.current === request) setError(String(error)); })
      .finally(() => { if (generation.current === request) setBusy(false); });
    // oxlint-disable-next-line react-hooks/exhaustive-deps -- a request generation counter, not a DOM ref: cleanup must bump its latest value
    return () => { generation.current++; };
  }, [scope, reload]);
  // Every control saves on change, against the revision it was loaded at, so
  // a choice can never be lost by leaving the page.
  const change = (patch: ChatDefaults, key: keyof ChatDefaults) => { void save(layerChatDefaults(draft, patch), key); };
  const inherit = (field: keyof ChatDefaults) => { const next = { ...draft }; delete next[field]; void save(next, field); };
  const setField = <K extends keyof ChatDefaults>(field: K, value: string, parse: (value: string) => ChatDefaults[K]) => value === INHERIT ? inherit(field) : change({ [field]: parse(value) }, field);
  const effective = layerChatDefaults(scope ? view?.application.defaults ?? {} : {}, draft);
  const model = effective.model;
  const adapter = adapters.find(a => a.id === model?.harness);
  const efforts = (adapter?.models.find(m => m.id === (model?.model ?? adapter.defaultModel))?.supportedEffortLevels ?? [])
    .filter(value => effective.target?.kind !== "agent" || WORKER_EFFORTS.includes(value));
  const choices: Record<keyof ChatDefaults, Choice[]> = {
    target: [{ value: "direct", label: "Direct chat" }, { value: "orchestrator", label: "Orchestrator" }, ...agents.filter(a => a.enabled && a.role !== "orchestrator").map(a => ({ value: `agent:${a.id}`, label: a.name }))],
    model: adapters.filter(a => a.available).flatMap(a => [{ value: modelValue({ harness: a.id, model: null }), label: `${a.label} · Provider default` }, ...a.models.map(m => ({ value: modelValue({ harness: a.id, model: m.id }), label: `${a.label} · ${m.label}`, disabled: m.available === false || m.compatible === false }))]),
    effort: [{ value: "provider", label: "Provider default" }, ...efforts.map(value => ({ value, label: value }))],
    access: [{ value: "ask", label: "Ask" }, { value: "full", label: "Full access" }],
    createWorktree: [{ value: "false", label: "Checkout" }, { value: "true", label: "Worktree" }],
    activeTurnInput: [{ value: "steer", label: "Steer" }, { value: "queue", label: "Queue" }],
  };
  const source = (field: keyof ChatDefaults) => {
    if (draft[field] != null) return scope ? "This project" : "Application default";
    const inherited = selectValue(field, view?.application.defaults ?? {});
    if (!scope || inherited === INHERIT) return "Uses the selected agent or Bridge default";
    return `Inherits application default: ${choices[field].find(choice => choice.value === inherited)?.label ?? inherited}`;
  };
  const save = async (defaults: ChatDefaults, key?: string) => {
    if (!view) return;
    setDraft(defaults);
    setBusy(true); setError(undefined);
    const request = generation.current;
    try {
      const next = await bridgeApi.saveChatDefaults({ projectId: scope || null, expectedRevision: (scope ? view.project : view.application)?.revision ?? 0, defaults });
      if (generation.current !== request) return;
      setView(next); setDraft(structuredClone((scope ? next.project : next.application)?.defaults ?? {}));
      if (!scope) acceptApplicationChatDefaults(next);
      flash(key ?? "reset");
    } catch (error) {
      if (generation.current !== request) return;
      setError(error instanceof Error ? error.message : String(error));
      setDraft(structuredClone((scope ? view.project : view.application)?.defaults ?? {}));
    }
    finally { if (generation.current === request) setBusy(false); }
  };
  const inherited = { value: INHERIT, label: scope ? "Inherit application default" : "Use agent or Bridge default" };
  const inheritShort = { value: INHERIT, label: scope ? "Inherit" : "Default" };
  return <SettingsPage title={PAGES.chatDefaults.label} description="Choose how new chats start. Each chat can change its own settings; saved defaults affect future chats.">
    <SettingsGroup label="Scope">
      <SettingsRow label="Apply defaults to" description="Project settings belong to this Bridge project on this device." control={<DefaultsSelect label="Defaults scope" value={scope} options={[{ value: "", label: "All projects (application default)" }, ...projects.map(p => ({ value: p.id, label: p.name }))]} onChange={setScope} />} />
    </SettingsGroup>
    {error && <div role="alert" className="text-ui text-destructive">{error} <TextButton onClick={() => setReload(value => value + 1)}>Reload defaults</TextButton></div>}
    {!view ? busy ? <SkeletonRows label="Loading defaults" rows={4} trailing /> : <p role="status" className="text-ui text-muted-foreground">Defaults could not be loaded.</p> : <>
      <SettingsGroup label="When a chat starts">
        <SettingsRow anchor="default-target" label="Agent or chat mode" saved={isFlashed("target")} description={source("target")} control={<DefaultsSelect label="Default chat target" disabled={busy} value={selectValue("target", draft)} options={[inherited, ...choices.target]} onChange={value => setField("target", value, value => value.startsWith("agent:") ? { kind: "agent", agentId: value.slice(6) } : { kind: value as "direct" | "orchestrator" })} />} />
        <SettingsRow anchor="default-model" label="Model" saved={isFlashed("model")} description={source("model")} control={<DefaultsSelect label="Default chat model" disabled={busy} value={selectValue("model", draft)} options={[inherited, ...choices.model]} onChange={value => {
          if (value === INHERIT) inherit("model");
          else { const model = JSON.parse(value) as NonNullable<ChatDefaults["model"]>; change({ model, effort: { value: null } }, "model"); }
        }} />} />
        <SettingsRow anchor="default-effort" label="Thinking effort" saved={isFlashed("effort")} description={efforts.length ? source("effort") : "This model has no advertised thinking control. Provider default is available."} control={<DefaultsSelect label="Default thinking effort" disabled={busy} value={selectValue("effort", draft)} options={[inherited, ...choices.effort]} onChange={value => setField("effort", value, value => ({ value: value === "provider" ? null : value }))} />} />
        <SettingsRow anchor="default-access" label="Access" saved={isFlashed("access")} description={`${source("access")}. Ask keeps provider approval checks; Full access grants ordinary provider approvals. Browser and other special grants remain separate.`} control={<Segmented label="Default chat access" disabled={busy} value={selectValue("access", draft)} options={[inheritShort, ...choices.access]} onChange={value => setField("access", value, value => value as "ask" | "full")} />} />
        <SettingsRow anchor="default-location" label="Work location" saved={isFlashed("createWorktree")} description={`${source("createWorktree")}. Checkout works in the project folder; a new isolated worktree is created only when the first message is sent.`} control={<Segmented label="Default work location" disabled={busy} value={selectValue("createWorktree", draft)} options={[inheritShort, ...choices.createWorktree]} onChange={value => setField("createWorktree", value, value => value === "true")} />} />
        <SettingsRow anchor="input-while-running" label="Input while running" saved={isFlashed("activeTurnInput")} description={`${source("activeTurnInput")}. Steer sends guidance into the live turn; Queue holds it until the current step finishes. Providers without live steer queue input until a safe boundary.`} control={<Segmented label="Default input while running" disabled={busy} value={selectValue("activeTurnInput", draft)} options={[inheritShort, ...choices.activeTurnInput]} onChange={value => setField("activeTurnInput", value, value => value as "steer" | "queue")} />} />
      </SettingsGroup>
      <SettingsGroup>
        <SettingsRow label="Reset these defaults" description={scope ? "Clear this project's choices so it inherits the application defaults." : "Clear every application default so chats use the selected agent or Bridge default."} saved={isFlashed("reset")} control={<TextButton disabled={busy} onClick={() => void save({})}>Reset</TextButton>} />
      </SettingsGroup>
    </>}
    {!scope && <LimitRecoverySection adapters={adapters} />}
  </SettingsPage>;
}
