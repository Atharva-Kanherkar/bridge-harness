import { useEffect, useMemo, useState } from "react";
import { Bot, Check, ChevronRight, Code2, LoaderCircle, Lock, Monitor, Moon, Plus, RotateCcw, Save, ScrollText, Settings2, Shield, ShieldOff, Sparkles, Sun, Trash2 } from "lucide-react";
import { bridgeApi } from "../api";
import { modelProfilesChanged, profileDraftsFromSetup } from "../modelProfiles";
import type { AdapterDescriptor, AgentDefinition, AgentRole, BridgeEvent, ConfigState, HarnessConfig, ModelProfileDraft, ModelSetupState, OpenCodeCatalog, PermissionPolicy, ReasoningEffort } from "../types";
import { ModelProfileEditor } from "./ModelProfileEditor";
import { ManagedAgentsPanel } from "./ManagedAgentsPanel";
import { PromptStudio } from "./PromptStudio";
import { WorkSettingsSection } from "./WorkSettingsSection";
import { SuggestionSettingsCard } from "./SuggestionSettingsCard";
import type { SuggestionSettingsSnapshot } from "../protocol/generated/protocol";
import { OpenCodeHarnessSettings, type OpenCodeAdvancedSettings } from "./OpenCodeHarnessSettings";
import { useThemePreference, type ThemePreference } from "../theme";
import { cn } from "@/lib/utils";

export type Section = "agents" | "harnesses" | "models" | "prompts" | "permissions" | "work" | "appearance";

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

const THEME_OPTIONS: { id: ThemePreference; label: string; hint: string; icon: React.ReactNode }[] = [
  { id: "system", label: "Match macOS", hint: "Follows your system appearance", icon: <Monitor size={15} /> },
  { id: "light", label: "Paper", hint: "Light mode", icon: <Sun size={15} /> },
  { id: "dark", label: "Graphite", hint: "Dark mode", icon: <Moon size={15} /> },
];

function AppearanceSection() {
  const { preference, resolved, setPreference } = useThemePreference();
  return <div className="mx-auto max-w-2xl">
    <div className="mb-5"><h2 className="font-display text-lg font-semibold">Appearance</h2><p className="mt-1 text-xs text-muted-foreground">Bridge follows macOS by default. Both modes render the same palette — currently showing {resolved === "dark" ? "graphite" : "paper"}.</p></div>
    <div className="grid gap-2 sm:grid-cols-3">
      {THEME_OPTIONS.map(option => <button
        type="button"
        key={option.id}
        onClick={() => setPreference(option.id)}
        aria-pressed={preference === option.id}
        className={cn(
          "flex flex-col items-start gap-1.5 rounded-2xl border p-4 text-left transition-colors",
          preference === option.id ? "border-ring bg-card" : "border-border hover:bg-accent",
        )}
      >
        <span className="text-muted-foreground">{option.icon}</span>
        <span className="text-[13px] font-medium text-foreground">{option.label}</span>
        <span className="text-[11px] text-muted-foreground">{option.hint}</span>
      </button>)}
    </div>
  </div>;
}

/// The two gates that outlive the bypass switch.
///
/// Named in the UI, not only in a doc comment, because the issue makes the copy
/// part of the contract: a switch that claims to silence everything and then
/// still prompts has to say up front where and why. Both are authorization
/// rather than convenience — a worker writing outside its lease, and an outward
/// effect like sending or purchasing.
const SURVIVING_GATES = [
  {
    title: "Worker write scope",
    copy: "A worker still needs your authorization for the paths it may write. Bypass covers convenience, not authorization.",
  },
  {
    title: "Browser outward effects",
    copy: "Send, submit, purchase, publish, and credential steps in the browser still ask, every time.",
  },
];

export function PermissionsSection({ policy, autoApprovals, busy, onChange }: {
  policy: PermissionPolicy;
  autoApprovals: BridgeEvent[];
  busy: boolean;
  onChange: (next: PermissionPolicy) => void;
}) {
  const on = policy.autoApproveProviderPermissions;
  return <div className="mx-auto max-w-2xl">
    <div className="mb-5">
      <h2 className="font-display text-lg font-semibold">Permissions</h2>
      <p className="mt-1 text-xs text-muted-foreground">How much Bridge asks before an agent acts.</p>
    </div>

    <button
      type="button"
      role="switch"
      aria-checked={on}
      disabled={busy}
      onClick={() => onChange({ ...policy, autoApproveProviderPermissions: !on })}
      // The ON state is carried by the border, the icon, and the pill — not by a
      // background swap. Tinting the card put `text-muted-foreground` body copy
      // on a lighter surface and the explanation went unreadable in dark mode,
      // which is the one state where the copy matters most.
      className={cn(
        "flex w-full items-start gap-3.5 rounded-2xl border p-4 text-left transition-colors hover:bg-accent disabled:opacity-50",
        on ? "border-warning/40" : "border-border",
      )}
    >
      <span className={cn("mt-0.5 shrink-0", on ? "text-warning" : "text-muted-foreground")}>
        {on ? <ShieldOff size={16} /> : <Shield size={16} />}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block text-[13px] font-medium text-foreground">Auto-approve provider permissions</span>
        <span className="mt-1 block text-[11.5px] leading-relaxed text-muted-foreground">
          Permission requests from Claude, Codex, OpenCode, and Cursor are accepted automatically when the provider offers an allow option. Questions and macOS prompts still wait for you.
        </span>
      </span>
      <span className={cn(
        "mt-0.5 shrink-0 rounded-full px-2 py-0.5 text-[9px] font-semibold tracking-[0.07em]",
        on ? "bg-warning/15 text-warning" : "border border-border text-muted-foreground",
      )}>{on ? "ON" : "OFF"}</span>
    </button>

    <div className="mt-3 rounded-2xl border border-border p-4">
      <p className="flex items-center gap-1.5 text-[11px] font-medium text-foreground"><Lock size={12} aria-hidden="true" /> These keep asking either way</p>
      <ul className="mt-2.5 grid gap-2.5">
        {SURVIVING_GATES.map(gate => <li key={gate.title}>
          <b className="block text-[12px] font-medium text-foreground">{gate.title}</b>
          <span className="mt-0.5 block text-[11px] leading-relaxed text-muted-foreground">{gate.copy}</span>
        </li>)}
      </ul>
    </div>

    <div className="mt-5">
      <h3 className="text-[12px] font-medium text-foreground">Recent auto-approvals</h3>
      <p className="mt-1 text-[11px] text-muted-foreground">Every automatic decision is recorded, so a bypassed approval is auditable rather than invisible.</p>
      {autoApprovals.length === 0
        ? <p className="mt-3 rounded-xl border border-border px-3 py-2.5 font-mono text-[10.5px] text-muted-foreground/70">Nothing has been auto-approved yet.</p>
        : <ul className="mt-3 grid gap-1.5">
            {autoApprovals.map(event => <li key={event.id} className="flex items-baseline gap-2.5 rounded-xl border border-border px-3 py-2">
              <span className="min-w-0 flex-1 truncate font-mono text-[10.5px] text-foreground/85">{event.body}</span>
              <time className="shrink-0 font-mono text-[9.5px] text-muted-foreground/70">{new Date(event.createdAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}</time>
            </li>)}
          </ul>}
    </div>
  </div>;
}

function newAgent(): AgentDefinition {
  return { id: "", name: "New agent", description: "", role: "orchestrator", harness: "bridge", model: null, effort: "medium", systemPrompt: "", enabled: true, isDefault: false, isBuiltIn: false, createdAt: "", updatedAt: "" };
}

function SectionButton({ active, icon, label, onClick }: { active: boolean; icon: React.ReactNode; label: string; onClick: () => void }) {
  return <button type="button" onClick={onClick} className={cn("flex h-10 w-full items-center gap-2.5 rounded-xl px-3 text-left text-[13px] transition-colors", active ? "bg-foreground/[0.08] text-foreground" : "text-muted-foreground hover:bg-foreground/[0.045] hover:text-foreground")}>
    {icon}<span className="flex-1">{label}</span>{active && <ChevronRight size={13} aria-hidden="true" />}
  </button>;
}

export function SettingsScreen({ adapters, autoApprovals = [], initialSection = "agents", onModelSetupChange, onSuggestionSettingsChange, onError }: { adapters: AdapterDescriptor[]; autoApprovals?: BridgeEvent[]; initialSection?: Section; onModelSetupChange: (setup: ModelSetupState) => void; onSuggestionSettingsChange: (snapshot: SuggestionSettingsSnapshot) => void; onError: (message: string) => void }) {
  const [section, setSection] = useState<Section>(initialSection);
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
    // Fetch on mount (not only in the Harnesses section): the Agents section
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
    if (!window.confirm("Reset every Bridge, Codex, Claude, and agent override? Custom agents will be deleted.")) return;
    setBusy(true);
    try { const [next, setup] = await Promise.all([bridgeApi.resetAllConfig(), bridgeApi.resetModelProfiles()]); acceptConfig(next); setSelectedAgentId(next.defaultAgentId); setModelSetup(setup); setProfiles(profileDraftsFromSetup(setup)); onModelSetupChange(setup); flashSaved(); }
    catch (error) { onError(String(error)); } finally { setBusy(false); }
  };

  return <div className="flex h-full min-h-0 flex-col">
    <header className="flex h-[64px] shrink-0 items-center border-b border-border/70 px-6 sm:px-8" data-tauri-drag-region="deep">
      <div className="min-w-0 flex-1"><h1 className="font-display text-lg font-semibold tracking-tight text-foreground">Settings</h1><p className="text-xs text-muted-foreground">Providers, models, prompts, and agent presets.</p></div>
      <div className="flex items-center gap-2">{saved && <span className="inline-flex items-center gap-1 text-xs text-success"><Check size={13} />Saved</span>}<button type="button" disabled={busy} onClick={() => void resetEverything()} className="inline-flex h-9 items-center gap-2 rounded-xl px-3 text-xs text-muted-foreground hover:bg-foreground/[0.05] hover:text-foreground disabled:opacity-45"><RotateCcw size={13} />Reset all</button></div>
    </header>
    <div className="flex min-h-0 flex-1">
      <nav className="w-48 shrink-0 border-r border-border/60 p-3">
        <SectionButton active={section === "agents"} icon={<Bot size={15} />} label="Agents" onClick={() => setSection("agents")} />
        <SectionButton active={section === "harnesses"} icon={<Code2 size={15} />} label="Harnesses" onClick={() => setSection("harnesses")} />
        <SectionButton active={section === "models"} icon={<Settings2 size={15} />} label="Role models" onClick={() => setSection("models")} />
        <SectionButton active={section === "prompts"} icon={<ScrollText size={15} />} label="Prompt Studio" onClick={() => setSection("prompts")} />
        <SectionButton active={section === "permissions"} icon={<Shield size={15} />} label="Permissions" onClick={() => setSection("permissions")} />
        <SectionButton active={section === "work"} icon={<Sparkles size={15} />} label="Work" onClick={() => setSection("work")} />
        <SectionButton active={section === "appearance"} icon={<Sun size={15} />} label="Appearance" onClick={() => setSection("appearance")} />
        <div className="mt-4 rounded-2xl border border-border/70 bg-foreground/[0.025] p-3"><Shield size={14} className="text-success"/><p className="mt-2 text-[10px] leading-relaxed text-muted-foreground">Prompts change behavior, never permissions. Existing running sessions keep their current configuration until restarted.</p></div>
      </nav>
      <div className="min-w-0 flex-1 overflow-y-auto p-5 sm:p-7">
        {busy && !config ? <div className="grid h-full place-items-center"><LoaderCircle className="animate-spin text-muted-foreground" size={18}/></div> : null}
        {section === "agents" && config && <div className="mx-auto flex max-w-5xl gap-5">
          <section className="w-64 shrink-0">
            <div className="mb-3 flex items-center justify-between"><div><h2 className="font-display text-base font-semibold">Agents</h2><p className="text-[11px] text-muted-foreground">{config.agents.length} presets</p></div><button type="button" onClick={() => { const draft = newAgent(); setSelectedAgentId(""); setAgentDraft(draft); }} className="grid h-8 w-8 place-items-center rounded-xl bg-foreground text-background hover:opacity-90" aria-label="Create agent"><Plus size={14}/></button></div>
            <div className="space-y-1">{config.agents.map(agent => <button type="button" key={agent.id} onClick={() => { setSelectedAgentId(agent.id ?? ""); setAgentDraft(structuredClone(agent)); }} className={cn("w-full rounded-2xl border p-3 text-left transition-colors", agentDraft?.id === agent.id ? "border-foreground/15 bg-foreground/[0.07]" : "border-transparent hover:bg-foreground/[0.04]")}><div className="flex items-center gap-2"><span className={cn("h-2 w-2 rounded-full", agent.enabled ? "bg-success" : "bg-muted-foreground/30")}/><span className="min-w-0 flex-1 truncate text-[13px] font-medium">{agent.name}</span>{agent.isDefault && <span className="rounded-full bg-accent px-1.5 py-0.5 text-[8px] uppercase tracking-wider text-muted-foreground">default</span>}</div><p className="mt-1 pl-4 text-[10px] capitalize text-muted-foreground">{agent.role} · {agent.harness}</p></button>)}</div>
          </section>
          {agentDraft && <section className="min-w-0 flex-1 rounded-3xl border border-border/80 bg-card/45 p-5">
            <div className="mb-5 flex items-start justify-between gap-3"><div><h2 className="font-display text-lg font-semibold">{agentDraft.id ? agentDraft.name : "Create agent"}</h2><p className="mt-1 text-xs text-muted-foreground">{agentDraft.isBuiltIn ? "Built-in preset · reset restores Bridge defaults" : "Custom preset · safe to delete at any time"}</p></div><label className="flex items-center gap-2 text-xs text-muted-foreground"><input type="checkbox" checked={agentDraft.enabled} onChange={event => setAgentDraft(value => value && ({ ...value, enabled: event.target.checked }))}/>Enabled</label></div>
            <div className="grid gap-4 sm:grid-cols-2"><label className="space-y-1.5 text-[11px] font-medium text-muted-foreground">Name<input className={field} value={agentDraft.name} onChange={event => setAgentDraft(value => value && ({ ...value, name: event.target.value }))}/></label><label className="space-y-1.5 text-[11px] font-medium text-muted-foreground">Role<select className={field} value={agentDraft.role} onChange={event => { const role = event.target.value as AgentRole; setAgentDraft(value => { if (!value) return value; const current = adapters.find(adapter => adapter.id === value.harness); return { ...value, role, harness: value.harness === "bridge" || (current && adapterSupportsAgentRole(current, role)) ? value.harness : "bridge", model: value.harness === "bridge" || (current && adapterSupportsAgentRole(current, role)) ? value.model : null }; }); }}>{roles.map(role => <option key={role.id} value={role.id}>{role.label}</option>)}</select></label><label className="space-y-1.5 text-[11px] font-medium text-muted-foreground">Runtime<select className={field} value={agentDraft.harness} onChange={event => setAgentDraft(value => value && ({ ...value, harness: event.target.value as AgentDefinition["harness"], model: null }))}><option value="bridge">Bridge chooses</option>{adapters.filter(adapter => adapterSupportsAgentRole(adapter, agentDraft.role)).map(adapter => <option key={adapter.id} value={adapter.id}>{adapter.label}</option>)}</select></label><label className="space-y-1.5 text-[11px] font-medium text-muted-foreground">Model<select className={field} value={agentDraft.model ?? ""} disabled={agentDraft.harness === "bridge"} onChange={event => setAgentDraft(value => value && ({ ...value, model: event.target.value || null }))}><option value="">Provider default</option>{modelOptions.filter(option => option.adapter === agentDraft.harness).map(option => <option key={`${option.adapter}:${option.id}`} value={option.id}>{option.label}</option>)}</select></label><label className="space-y-1.5 text-[11px] font-medium text-muted-foreground">Effort<select className={field} value={agentDraft.effort} onChange={event => setAgentDraft(value => value && ({ ...value, effort: event.target.value as ReasoningEffort }))}>{efforts.map(value => <option key={value} value={value}>{value}</option>)}</select></label><label className="space-y-1.5 text-[11px] font-medium text-muted-foreground">Description<input className={field} value={agentDraft.description} onChange={event => setAgentDraft(value => value && ({ ...value, description: event.target.value }))}/></label></div>
            <label className="mt-4 block space-y-1.5 text-[11px] font-medium text-muted-foreground">System prompt <span className="font-normal text-muted-foreground/60">appended after Bridge safety and routing policy</span><textarea className={textarea} value={agentDraft.systemPrompt} placeholder="Add role-specific behavior…" onChange={event => setAgentDraft(value => value && ({ ...value, systemPrompt: event.target.value }))}/></label>
            <div className="mt-5 flex flex-wrap items-center gap-2"><button type="button" disabled={busy || !agentDraft.name.trim()} onClick={() => void saveAgent()} className="inline-flex h-9 items-center gap-2 rounded-xl bg-foreground px-3.5 text-xs font-medium text-background disabled:opacity-40"><Save size={13}/>{agentDraft.id ? "Save agent" : "Create agent"}</button>{agentDraft.id && agentDraft.role === "orchestrator" && !agentDraft.isDefault && <button type="button" disabled={busy || !agentDraft.enabled} onClick={() => void makeDefault()} className="h-9 rounded-xl border border-border px-3 text-xs text-foreground hover:bg-foreground/[0.05] disabled:opacity-40">Make default orchestrator</button>}{agentDraft.id && <button type="button" disabled={busy} onClick={() => void removeAgent()} className="ml-auto inline-flex h-9 items-center gap-2 rounded-xl px-3 text-xs text-destructive hover:bg-destructive/10"><Trash2 size={13}/>{agentDraft.isBuiltIn ? "Reset agent" : "Delete agent"}</button>}</div>
          </section>}
        </div>}
        {section === "permissions" && config && <PermissionsSection
          policy={config.permissionPolicy}
          autoApprovals={autoApprovals}
          busy={busy}
          onChange={policy => void savePolicy(policy)}
        />}
        {section === "prompts" && <div className="mx-auto h-full max-w-6xl"><PromptStudio /></div>}
        {section === "appearance" && <AppearanceSection />}
        {section === "work" && <WorkSettingsSection onError={onError} />}
        {section === "harnesses" && config && <div className="mx-auto max-w-4xl">
          <div className="mb-5"><h2 className="font-display text-lg font-semibold">Harness configuration</h2><p className="mt-1 text-xs text-muted-foreground">Defaults apply to new sessions. Provider credentials stay in each harness’s own credential store.</p></div>
          {/* Runtime installation, above configuration: whether an agent is
              installed at all is the question that comes first, and it is a
              different question from how it behaves once it runs. */}
          <section className="mb-6" aria-labelledby="managed-runtimes-heading">
            <div className="mb-3"><h3 id="managed-runtimes-heading" className="font-display text-sm font-semibold">Agent runtimes</h3><p className="mt-1 text-xs text-muted-foreground">Bridge installs these from each vendor’s official source. A runtime you installed yourself keeps working and is never removed by Bridge.</p></div>
            <ManagedAgentsPanel />
          </section>
          <div className="space-y-4">{config.harnesses.map(item => {
            const draft = harnessDrafts[item.id] ?? item;
            return <section key={item.id} className="rounded-3xl border border-border/80 bg-card/45 p-5">
              <div className="flex items-center gap-3"><span className="grid h-9 w-9 place-items-center rounded-xl bg-foreground/[0.06]"><Code2 size={16}/></span><div className="flex-1"><h3 className="font-display font-semibold">{draft.label}</h3><p className="text-[10px] text-muted-foreground">{item.isOverride ? "Customized" : "Bridge defaults"}</p></div><label className="flex items-center gap-2 text-xs text-muted-foreground"><input type="checkbox" checked={draft.enabled} onChange={event => setHarnessDrafts(current => ({ ...current, [item.id]: { ...draft, enabled: event.target.checked } }))}/>Enabled</label></div>
              <div className="mt-4 grid gap-4 sm:grid-cols-2"><label className="space-y-1.5 text-[11px] font-medium text-muted-foreground">Default model<select className={field} value={draft.defaultModel ?? ""} disabled={item.id === "bridge"} onChange={event => setHarnessDrafts(current => ({ ...current, [item.id]: { ...draft, defaultModel: event.target.value || null } }))}><option value="">Automatic</option>{modelOptions.filter(option => option.adapter === item.id).map(option => <option key={option.id} value={option.id}>{option.label}</option>)}</select></label><label className="space-y-1.5 text-[11px] font-medium text-muted-foreground">Default effort<select className={field} value={draft.effort ?? ""} onChange={event => setHarnessDrafts(current => ({ ...current, [item.id]: { ...draft, effort: (event.target.value || null) as ReasoningEffort | null } }))}><option value="">Role default</option>{efforts.map(value => <option key={value} value={value}>{value}</option>)}</select></label></div>
              <label className="mt-4 block space-y-1.5 text-[11px] font-medium text-muted-foreground">Harness system prompt<textarea className={textarea} value={draft.systemPrompt} placeholder={`Instructions for every ${draft.label} session…`} onChange={event => setHarnessDrafts(current => ({ ...current, [item.id]: { ...draft, systemPrompt: event.target.value } }))}/></label>
              {item.id === "opencode" ? <OpenCodeHarnessSettings value={openCodeAdvanced()} catalog={openCodeCatalog} discoveryError={openCodeDiscoveryError} disabled={busy} onChange={value => { updateOpenCodeAdvanced(value); if (draft.defaultModel && value.visibleModels?.length && !value.visibleModels.includes(draft.defaultModel)) setHarnessDrafts(current => ({ ...current, opencode: { ...draft, defaultModel: null } })); }} onCatalog={catalog => { setOpenCodeCatalog(catalog); setOpenCodeDiscoveryError(undefined); }} onError={message => { setOpenCodeDiscoveryError(message); onError(message); }}/> : <label className="mt-4 block space-y-1.5 text-[11px] font-medium text-muted-foreground">Advanced JSON<textarea className={cn(textarea, "min-h-24")} spellCheck={false} value={advancedText[item.id] ?? "{}"} onChange={event => setAdvancedText(current => ({ ...current, [item.id]: event.target.value }))}/></label>}
              <div className="mt-4 flex gap-2"><button type="button" disabled={busy} onClick={() => void saveHarness(item.id)} className="inline-flex h-9 items-center gap-2 rounded-xl bg-foreground px-3.5 text-xs font-medium text-background disabled:opacity-40"><Save size={13}/>Save {draft.label}</button><button type="button" disabled={busy || !item.isOverride} onClick={() => void resetHarness(item.id)} className="inline-flex h-9 items-center gap-2 rounded-xl px-3 text-xs text-muted-foreground hover:bg-foreground/[0.05] hover:text-foreground disabled:opacity-35"><RotateCcw size={13}/>Reset</button></div>
            </section>;
          })}</div>
        </div>}
        {section === "models" && modelSetup && <div className="mx-auto max-w-5xl"><div className="mb-5 flex items-start justify-between gap-4"><div><h2 className="font-display text-lg font-semibold">Role model profiles</h2><p className="mt-1 text-xs text-muted-foreground">Provider, model, effort, fallback, learning, cost, and latency for every Bridge role. Version {modelSetup.activeVersion ?? "—"}.</p></div><button type="button" disabled={busy || !modelProfilesChanged(profiles, modelSetup)} onClick={() => void saveModels()} className="inline-flex h-9 items-center gap-2 rounded-xl bg-foreground px-3.5 text-xs font-medium text-background disabled:opacity-40"><Save size={13}/>Save profiles</button></div><ModelProfileEditor profiles={profiles} adapters={adapters} disabled={busy} onChange={setProfiles}/><SuggestionSettingsCard adapters={adapters} onChange={onSuggestionSettingsChange} onError={onError}/></div>}
      </div>
    </div>
  </div>;
}
