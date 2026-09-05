import { useEffect, useMemo, useState } from "react";
import { CircleNotch, Plus } from "@phosphor-icons/react";
import { bridgeApi } from "../api";
import { modelProfilesChanged, profileDraftsFromSetup } from "../modelProfiles";
import type { AdapterDescriptor, AgentDefinition, AgentRole, BridgeEvent, ConfigState, HarnessConfig, ModelProfileDraft, ModelSetupState, OpenCodeCatalog, PermissionPolicy, ReasoningEffort } from "../types";
import { ModelProfileEditor } from "./ModelProfileEditor";
import { ManagedAgentsPanel } from "./ManagedAgentsPanel";
import { PromptStudio } from "./PromptStudio";
import { WorkSettingsSection } from "./WorkSettingsSection";
import type { SuggestionSettingsSnapshot } from "../protocol/generated/protocol";
import { OpenCodeHarnessSettings, type OpenCodeAdvancedSettings } from "./OpenCodeHarnessSettings";
import { cn } from "@/lib/utils";
import { ImportHarnessSection } from "./ImportHarnessSection";
import { SettingsRail } from "./settings/SettingsRail";
import { AppearancePage } from "./settings/AppearancePage";
import { PermissionsSection } from "./settings/PermissionsPage";
import { ComposerPage } from "./settings/ComposerPage";
import { STATIC_SETTINGS_ROWS, type SearchableRow } from "./settings/settingsSearch";
import { type Section } from "./settings/sections";

export type { Section };
export { PermissionsSection };

const roles: { id: AgentRole; label: string }[] = [
  { id: "orchestrator", label: "Orchestrator" }, { id: "research", label: "Research" },
  { id: "implementation", label: "Implementation" }, { id: "verification", label: "Verification" },
  { id: "planning", label: "Planning" }, { id: "documentation", label: "Documentation" },
];
const efforts: ReasoningEffort[] = ["low", "medium", "high", "xhigh"];
const field = "h-10 w-full rounded-xl border border-border bg-background/60 px-3 text-sm text-foreground outline-none transition-colors focus:border-foreground/25 disabled:opacity-45";
const textarea = "min-h-36 w-full resize-y rounded-xl border border-border bg-background/60 px-3 py-2.5 font-mono text-xs leading-relaxed text-foreground outline-none transition-colors focus:border-foreground/25 disabled:opacity-45";

export function adapterSupportsAgentRole(adapter: AdapterDescriptor, role: string): boolean {
  const supportsSandbox = (mode: "read_only" | "workspace_write") => !adapter.sandboxModes?.length || adapter.sandboxModes.includes(mode);
  if (role === "orchestrator") return adapter.capabilities.includes("briefings") && supportsSandbox("workspace_write");
  if (role === "implementation") return supportsSandbox("workspace_write");
  return supportsSandbox("read_only");
}

function newAgent(): AgentDefinition {
  return { id: "", name: "New agent", description: "", role: "orchestrator", harness: "bridge", model: null, effort: "medium", systemPrompt: "", enabled: true, isDefault: false, isBuiltIn: false, createdAt: "", updatedAt: "" };
}

export function SettingsScreen({ adapters, autoApprovals = [], initialSection = "agents", onModelSetupChange, onSuggestionSettingsChange, onError }: { adapters: AdapterDescriptor[]; autoApprovals?: BridgeEvent[]; initialSection?: Section; onModelSetupChange: (setup: ModelSetupState) => void; onSuggestionSettingsChange: (snapshot: SuggestionSettingsSnapshot) => void; onError: (message: string) => void }) {
  const [section, setSection] = useState<Section>(initialSection);
  const [query, setQuery] = useState("");
  const [config, setConfig] = useState<ConfigState>();
  const [modelSetup, setModelSetup] = useState<ModelSetupState>();
  const [profiles, setProfiles] = useState<ModelProfileDraft[]>([]);
  const [selectedAgentId, setSelectedAgentId] = useState("bridge-orchestrator");
  const [agentDraft, setAgentDraft] = useState<AgentDefinition>();
  const [harnessDrafts, setHarnessDrafts] = useState<Record<string, HarnessConfig>>({});
  const [advancedText, setAdvancedText] = useState<Record<string, string>>({});
  const [openCodeCatalog, setOpenCodeCatalog] = useState<OpenCodeCatalog>();
  const [openCodeDiscoveryError, setOpenCodeDiscoveryError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    let active = true;
    setBusy(true);
    Promise.all([bridgeApi.configState(), bridgeApi.modelSetup()]).then(([next, setup]) => {
      if (!active) return;
      setConfig(next); setHarnessDrafts(Object.fromEntries(next.harnesses.map(item => [item.id, item])));
      setAdvancedText(Object.fromEntries(next.harnesses.map(item => [item.id, JSON.stringify(item.advanced, null, 2)])));
      setModelSetup(setup); setProfiles(profileDraftsFromSetup(setup));
    }).catch(error => onError(String(error))).finally(() => { if (active) setBusy(false); });
    return () => { active = false; };
  }, [onError]);

  /// Render from what was stored, not from what was clicked. The switch is a
  /// security control, so the UI must never show it on because a request was
  /// sent — only because the host confirmed it.
  async function savePolicy(policy: PermissionPolicy) {
    setBusy(true);
    try { setConfig(await bridgeApi.savePermissionPolicy(policy)); flashSaved(); }
    catch (error) { onError(String(error)); }
    finally { setBusy(false); }
  }

  useEffect(() => {
    // Fetch on mount (not only in the Harnesses section): the Presets page
    // needs the catalog to list OpenCode models for opencode-harness agents.
    if (openCodeCatalog || openCodeDiscoveryError) return;
    let active = true;
    bridgeApi.refreshOpenCodeCatalog()
      .then(catalog => { if (active) setOpenCodeCatalog(catalog); })
      .catch(error => { if (active) setOpenCodeDiscoveryError(error instanceof Error ? error.message : String(error)); });
    return () => { active = false; };
  }, [openCodeCatalog, openCodeDiscoveryError]);

  useEffect(() => {
    if (!config || selectedAgentId === "") return;
    const selected = config.agents.find(item => item.id === selectedAgentId) ?? config.agents[0];
    if (selected) { setSelectedAgentId(selected.id ?? ""); setAgentDraft(structuredClone(selected)); }
  }, [config, selectedAgentId]);

  const modelOptions = useMemo(() => {
    const staticOptions = adapters.filter(item => item.available && item.id !== "opencode").flatMap(adapter => adapter.models.map(model => ({ adapter: adapter.id, id: model.id, label: `${adapter.label} · ${model.label}` })));
    let configuredVisibleModels: string[] = [];
    try {
      const parsed = JSON.parse(advancedText.opencode || "{}");
      if (Array.isArray(parsed.visibleModels)) configuredVisibleModels = parsed.visibleModels;
    } catch { /* the save action reports malformed JSON */ }
    const visible = new Set(configuredVisibleModels);
    const openCodeOptions = openCodeCatalog?.providers.flatMap(provider => provider.connected ? provider.models.filter(model => visible.size === 0 || visible.has(model.id)).map(model => ({ adapter: "opencode", id: model.id, label: `${provider.name} · ${model.label}` })) : []) ?? [];
    return [...staticOptions, ...openCodeOptions];
  }, [adapters, openCodeCatalog, advancedText.opencode]);
  const flashSaved = () => { setSaved(true); window.setTimeout(() => setSaved(false), 1600); };
  const acceptConfig = (next: ConfigState) => {
    setConfig(next); setHarnessDrafts(Object.fromEntries(next.harnesses.map(item => [item.id, item])));
    setAdvancedText(Object.fromEntries(next.harnesses.map(item => [item.id, JSON.stringify(item.advanced, null, 2)])));
  };

  /** The rows the rail search can reach: the fixed ones, plus whatever the
   *  user's own configuration named. A harness or a preset is findable by its
   *  own name, not only by the page it happens to live on. */
  const searchRows: SearchableRow[] = useMemo(() => [
    ...STATIC_SETTINGS_ROWS,
    ...(config?.harnesses ?? []).map(item => ({
      section: "harnesses" as const,
      label: item.label,
      description: item.isOverride ? "Customized" : "Bridge defaults",
    })),
    ...(config?.agents ?? []).map(item => ({
      section: "agents" as const,
      label: item.name,
      description: `${item.role} · ${item.harness}`,
    })),
  ], [config]);

  const saveAgent = async () => {
    if (!agentDraft) return;
    setBusy(true);
    try {
      const next = await bridgeApi.saveAgentConfig(agentDraft);
      acceptConfig(next);
      const savedAgent = next.agents.find(item => item.id === agentDraft.id) ?? next.agents.find(item => item.name === agentDraft.name);
      if (savedAgent) setSelectedAgentId(savedAgent.id ?? "");
      flashSaved();
    } catch (error) { onError(error instanceof Error ? error.message : String(error)); }
    finally { setBusy(false); }
  };

  const removeAgent = async () => {
    if (!agentDraft) return;
    const verb = agentDraft.isBuiltIn ? "Reset" : "Delete";
    if (!window.confirm(`${verb} ${agentDraft.name}?`)) return;
    setBusy(true);
    try { const next = await bridgeApi.deleteAgentConfig(agentDraft.id ?? ""); acceptConfig(next); setSelectedAgentId(next.defaultAgentId); flashSaved(); }
    catch (error) { onError(String(error)); } finally { setBusy(false); }
  };

  const makeDefault = async () => {
    if (!agentDraft) return;
    setBusy(true);
    try { acceptConfig(await bridgeApi.setDefaultAgent(agentDraft.id ?? "")); flashSaved(); }
    catch (error) { onError(String(error)); } finally { setBusy(false); }
  };

  const saveHarness = async (id: string) => {
    const draft = harnessDrafts[id]; if (!draft) return;
    let advanced: Record<string, unknown>;
    try { const parsed: unknown = JSON.parse(advancedText[id] || "{}"); if (!parsed || Array.isArray(parsed) || typeof parsed !== "object") throw new Error("Advanced configuration must be a JSON object"); advanced = parsed as Record<string, unknown>; }
    catch (error) { onError(error instanceof Error ? error.message : String(error)); return; }
    setBusy(true);
    try {
      acceptConfig(await bridgeApi.saveHarnessConfig({ ...draft, advanced }));
      if (id === "opencode") {
        setOpenCodeCatalog(await bridgeApi.refreshOpenCodeCatalog());
        setOpenCodeDiscoveryError(undefined);
      }
      flashSaved();
    }
    catch (error) { onError(String(error)); } finally { setBusy(false); }
  };

  const openCodeAdvanced = (): OpenCodeAdvancedSettings => {
    try { return JSON.parse(advancedText.opencode || "{}"); }
    catch { return {}; }
  };
  const updateOpenCodeAdvanced = (value: OpenCodeAdvancedSettings) => {
    setAdvancedText(current => ({ ...current, opencode: JSON.stringify(value, null, 2) }));
  };

  const resetHarness = async (id: HarnessConfig["id"]) => {
    setBusy(true);
    try {
      acceptConfig(await bridgeApi.resetHarnessConfig(id));
      if (id === "opencode") {
        setOpenCodeCatalog(await bridgeApi.refreshOpenCodeCatalog());
        setOpenCodeDiscoveryError(undefined);
      }
      flashSaved();
    }
    catch (error) { onError(String(error)); } finally { setBusy(false); }
  };

  const saveModels = async () => {
    setBusy(true);
    try { const setup = await bridgeApi.saveModelProfiles(profiles); setModelSetup(setup); setProfiles(profileDraftsFromSetup(setup)); onModelSetupChange(setup); flashSaved(); }
    catch (error) { onError(String(error)); } finally { setBusy(false); }
  };

  const resetEverything = async () => {
    setBusy(true);
    try { const [next, setup] = await Promise.all([bridgeApi.resetAllConfig(), bridgeApi.resetModelProfiles()]); acceptConfig(next); setSelectedAgentId(next.defaultAgentId); setModelSetup(setup); setProfiles(profileDraftsFromSetup(setup)); onModelSetupChange(setup); flashSaved(); }
    catch (error) { onError(String(error)); } finally { setBusy(false); }
  };

  return <div className="flex h-full min-h-0">
    <SettingsRail
      section={section}
      query={query}
      rows={searchRows}
      resetting={busy}
      onQueryChange={setQuery}
      onSelect={next => { setSection(next); setQuery(""); }}
      onResetAll={() => void resetEverything()}
    />
    <div className="relative min-h-0 flex-1 overflow-y-auto">
      <div className="h-4" data-tauri-drag-region="deep" />
      {busy && !config ? <div className="grid h-full place-items-center"><CircleNotch className="animate-spin text-muted-foreground" size={18} weight="regular" /></div> : null}

      {section === "appearance" && <AppearancePage />}

      {section === "permissions" && config && <PermissionsSection
        policy={config.permissionPolicy}
        autoApprovals={autoApprovals}
        busy={busy}
        saved={saved}
        onChange={policy => void savePolicy(policy)}
      />}

      {section === "composer" && <ComposerPage adapters={adapters} onChange={onSuggestionSettingsChange} onError={onError} />}

      {section === "prompts" && <div className="mx-auto h-full max-w-6xl"><PromptStudio /></div>}
      {section === "import" && <ImportHarnessSection onError={onError} />}
      {section === "work" && <WorkSettingsSection onError={onError} />}

      {section === "agents" && config && <div className="mx-auto flex max-w-5xl gap-5 p-5">
        <section className="w-64 shrink-0">
          <div className="mb-3 flex items-center justify-between"><div><h2 className="font-display text-base font-semibold">Presets</h2><p className="text-[11px] text-muted-foreground">{config.agents.length} presets</p></div><button type="button" onClick={() => { const draft = newAgent(); setSelectedAgentId(""); setAgentDraft(draft); }} className="grid h-8 w-8 place-items-center rounded-xl bg-foreground text-background hover:opacity-90" aria-label="Create agent"><Plus size={14} weight="regular" /></button></div>
          <div className="space-y-1">{config.agents.map(agent => <button type="button" key={agent.id} onClick={() => { setSelectedAgentId(agent.id ?? ""); setAgentDraft(structuredClone(agent)); }} className={cn("w-full rounded-2xl border p-3 text-left transition-colors", agentDraft?.id === agent.id ? "border-foreground/15 bg-foreground/[0.07]" : "border-transparent hover:bg-foreground/[0.04]")}><div className="flex items-center gap-2"><span className={cn("h-2 w-2 rounded-full", agent.enabled ? "bg-success" : "bg-muted-foreground/30")}/><span className="min-w-0 flex-1 truncate text-[13px] font-medium">{agent.name}</span>{agent.isDefault && <span className="rounded-full bg-accent px-1.5 py-0.5 text-[8px] uppercase tracking-wider text-muted-foreground">default</span>}</div><p className="mt-1 pl-4 text-[10px] capitalize text-muted-foreground">{agent.role} · {agent.harness}</p></button>)}</div>
        </section>
        {agentDraft && <section className="min-w-0 flex-1 rounded-3xl border border-border/80 bg-card/45 p-5">
          <div className="mb-5 flex items-start justify-between gap-3"><div><h2 className="font-display text-lg font-semibold">{agentDraft.id ? agentDraft.name : "Create agent"}</h2><p className="mt-1 text-xs text-muted-foreground">{agentDraft.isBuiltIn ? "Built-in preset · reset restores Bridge defaults" : "Custom preset · safe to delete at any time"}</p></div><label className="flex items-center gap-2 text-xs text-muted-foreground"><input type="checkbox" checked={agentDraft.enabled} onChange={event => setAgentDraft(value => value && ({ ...value, enabled: event.target.checked }))}/>Enabled</label></div>
          <div className="grid gap-4 sm:grid-cols-2"><label className="space-y-1.5 text-[11px] font-medium text-muted-foreground">Name<input className={field} value={agentDraft.name} onChange={event => setAgentDraft(value => value && ({ ...value, name: event.target.value }))}/></label><label className="space-y-1.5 text-[11px] font-medium text-muted-foreground">Role<select className={field} value={agentDraft.role} onChange={event => { const role = event.target.value as AgentRole; setAgentDraft(value => { if (!value) return value; const current = adapters.find(adapter => adapter.id === value.harness); return { ...value, role, harness: value.harness === "bridge" || (current && adapterSupportsAgentRole(current, role)) ? value.harness : "bridge", model: value.harness === "bridge" || (current && adapterSupportsAgentRole(current, role)) ? value.model : null }; }); }}>{roles.map(role => <option key={role.id} value={role.id}>{role.label}</option>)}</select></label><label className="space-y-1.5 text-[11px] font-medium text-muted-foreground">Runtime<select className={field} value={agentDraft.harness} onChange={event => setAgentDraft(value => value && ({ ...value, harness: event.target.value as AgentDefinition["harness"], model: null }))}><option value="bridge">Bridge chooses</option>{adapters.filter(adapter => adapterSupportsAgentRole(adapter, agentDraft.role)).map(adapter => <option key={adapter.id} value={adapter.id}>{adapter.label}</option>)}</select></label><label className="space-y-1.5 text-[11px] font-medium text-muted-foreground">Model<select className={field} value={agentDraft.model ?? ""} disabled={agentDraft.harness === "bridge"} onChange={event => setAgentDraft(value => value && ({ ...value, model: event.target.value || null }))}><option value="">Provider default</option>{modelOptions.filter(option => option.adapter === agentDraft.harness).map(option => <option key={`${option.adapter}:${option.id}`} value={option.id}>{option.label}</option>)}</select></label><label className="space-y-1.5 text-[11px] font-medium text-muted-foreground">Effort<select className={field} value={agentDraft.effort} onChange={event => setAgentDraft(value => value && ({ ...value, effort: event.target.value as ReasoningEffort }))}>{efforts.map(value => <option key={value} value={value}>{value}</option>)}</select></label><label className="space-y-1.5 text-[11px] font-medium text-muted-foreground">Description<input className={field} value={agentDraft.description} onChange={event => setAgentDraft(value => value && ({ ...value, description: event.target.value }))}/></label></div>
          <label className="mt-4 block space-y-1.5 text-[11px] font-medium text-muted-foreground">System prompt <span className="font-normal text-muted-foreground/60">appended after Bridge safety and routing policy</span><textarea className={textarea} value={agentDraft.systemPrompt} placeholder="Add role-specific behavior…" onChange={event => setAgentDraft(value => value && ({ ...value, systemPrompt: event.target.value }))}/></label>
          <div className="mt-5 flex flex-wrap items-center gap-2"><button type="button" disabled={busy || !agentDraft.name.trim()} onClick={() => void saveAgent()} className="inline-flex h-9 items-center gap-2 rounded-xl bg-foreground px-3.5 text-xs font-medium text-background disabled:opacity-40">{agentDraft.id ? "Save agent" : "Create agent"}</button>{agentDraft.id && agentDraft.role === "orchestrator" && !agentDraft.isDefault && <button type="button" disabled={busy || !agentDraft.enabled} onClick={() => void makeDefault()} className="h-9 rounded-xl border border-border px-3 text-xs text-foreground hover:bg-foreground/[0.05] disabled:opacity-40">Make default orchestrator</button>}{agentDraft.id && <button type="button" disabled={busy} onClick={() => void removeAgent()} className="ml-auto inline-flex h-9 items-center gap-2 rounded-xl px-3 text-xs text-destructive hover:bg-destructive/10">{agentDraft.isBuiltIn ? "Reset agent" : "Delete agent"}</button>}</div>
        </section>}
      </div>}

      {section === "harnesses" && config && <div className="mx-auto max-w-4xl p-5">
        <div className="mb-5"><h2 className="font-display text-lg font-semibold">Harness configuration</h2><p className="mt-1 text-xs text-muted-foreground">Defaults apply to new sessions. Provider credentials stay in each harness's own credential store.</p></div>
        <section className="mb-6" aria-labelledby="managed-runtimes-heading">
          <div className="mb-3"><h3 id="managed-runtimes-heading" className="font-display text-sm font-semibold">Agent runtimes</h3></div>
          <ManagedAgentsPanel />
        </section>
        <div className="space-y-4">{config.harnesses.map(item => {
          const draft = harnessDrafts[item.id] ?? item;
          return <section key={item.id} className="rounded-3xl border border-border/80 bg-card/45 p-5">
            <div className="flex items-center gap-3"><div className="flex-1"><h3 className="font-display font-semibold">{draft.label}</h3><p className="text-[10px] text-muted-foreground">{item.isOverride ? "Customized" : "Bridge defaults"}</p></div><label className="flex items-center gap-2 text-xs text-muted-foreground"><input type="checkbox" checked={draft.enabled} onChange={event => setHarnessDrafts(current => ({ ...current, [item.id]: { ...draft, enabled: event.target.checked } }))}/>Enabled</label></div>
            <div className="mt-4 grid gap-4 sm:grid-cols-2"><label className="space-y-1.5 text-[11px] font-medium text-muted-foreground">Default model<select className={field} value={draft.defaultModel ?? ""} disabled={item.id === "bridge"} onChange={event => setHarnessDrafts(current => ({ ...current, [item.id]: { ...draft, defaultModel: event.target.value || null } }))}><option value="">Automatic</option>{modelOptions.filter(option => option.adapter === item.id).map(option => <option key={option.id} value={option.id}>{option.label}</option>)}</select></label><label className="space-y-1.5 text-[11px] font-medium text-muted-foreground">Default effort<select className={field} value={draft.effort ?? ""} onChange={event => setHarnessDrafts(current => ({ ...current, [item.id]: { ...draft, effort: (event.target.value || null) as ReasoningEffort | null } }))}><option value="">Role default</option>{efforts.map(value => <option key={value} value={value}>{value}</option>)}</select></label></div>
            <label className="mt-4 block space-y-1.5 text-[11px] font-medium text-muted-foreground">Harness system prompt<textarea className={textarea} value={draft.systemPrompt} placeholder={`Instructions for every ${draft.label} session…`} onChange={event => setHarnessDrafts(current => ({ ...current, [item.id]: { ...draft, systemPrompt: event.target.value } }))}/></label>
            {item.id === "opencode" ? <OpenCodeHarnessSettings value={openCodeAdvanced()} catalog={openCodeCatalog} discoveryError={openCodeDiscoveryError} disabled={busy} onChange={value => { updateOpenCodeAdvanced(value); if (draft.defaultModel && value.visibleModels?.length && !value.visibleModels.includes(draft.defaultModel)) setHarnessDrafts(current => ({ ...current, opencode: { ...draft, defaultModel: null } })); }} onCatalog={catalog => { setOpenCodeCatalog(catalog); setOpenCodeDiscoveryError(undefined); }} onError={message => { setOpenCodeDiscoveryError(message); onError(message); }}/> : <label className="mt-4 block space-y-1.5 text-[11px] font-medium text-muted-foreground">Advanced JSON<textarea className={cn(textarea, "min-h-24")} spellCheck={false} value={advancedText[item.id] ?? "{}"} onChange={event => setAdvancedText(current => ({ ...current, [item.id]: event.target.value }))}/></label>}
            <div className="mt-4 flex gap-2"><button type="button" disabled={busy} onClick={() => void saveHarness(item.id)} className="inline-flex h-9 items-center gap-2 rounded-xl bg-foreground px-3.5 text-xs font-medium text-background disabled:opacity-40">Save {draft.label}</button><button type="button" disabled={busy || !item.isOverride} onClick={() => void resetHarness(item.id)} className="inline-flex h-9 items-center gap-2 rounded-xl px-3 text-xs text-muted-foreground hover:bg-foreground/[0.05] hover:text-foreground disabled:opacity-35">Reset</button></div>
          </section>;
        })}</div>
      </div>}

      {section === "models" && modelSetup && <div className="mx-auto max-w-5xl p-5"><div className="mb-5 flex items-start justify-between gap-4"><div><h2 className="font-display text-lg font-semibold">Models</h2><p className="mt-1 text-xs text-muted-foreground">Version {modelSetup.activeVersion ?? "—"}.</p></div><button type="button" disabled={busy || !modelProfilesChanged(profiles, modelSetup)} onClick={() => void saveModels()} className="inline-flex h-9 items-center gap-2 rounded-xl bg-foreground px-3.5 text-xs font-medium text-background disabled:opacity-40">Save profiles</button></div><ModelProfileEditor profiles={profiles} adapters={adapters} disabled={busy} onChange={setProfiles}/></div>}
    </div>
  </div>;
}
