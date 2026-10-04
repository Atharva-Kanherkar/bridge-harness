import { useEffect, useMemo, useState } from "react";
import { LoaderCircle as CircleNotch } from "lucide-react";
import { bridgeApi } from "../api";
import { MenuBarSettingsPage } from "../features/menu-bar/MenuBarSettingsPage";
import { profileDraftsFromSetup } from "../modelProfiles";
import type { AdapterDescriptor, AgentDefinition, AgentRole, BridgeEvent, ConfigState, HarnessConfig, ModelProfileDraft, ModelSetupState, OpenCodeCatalog, PermissionPolicy, ReasoningEffort } from "../types";
import { useManagedAgents } from "./ManagedAgentsPanel";
import { PromptStudio } from "./PromptStudio";
import { WorkSettingsSection } from "./WorkSettingsSection";
import type { SuggestionSettingsSnapshot } from "../protocol/generated/protocol";
import { type OpenCodeAdvancedSettings } from "./OpenCodeHarnessSettings";
import { ImportHarnessSection } from "./ImportHarnessSection";
import { SettingsGroup, SettingsRow, TextButton } from "./settings/kit";
import { SettingsRail } from "./settings/SettingsRail";
import { ActiveTurnInputSetting } from "./settings/ActiveTurnInputSetting";
import { AppearancePage } from "./settings/AppearancePage";
import { UpdatesPage } from "./settings/UpdatesPage";
import type { UpdateInfo } from "../updater";
import { PermissionsSection } from "./settings/PermissionsPage";
import { ComposerPage } from "./settings/ComposerPage";
import { HarnessesPage, type HarnessDraft } from "./settings/HarnessesPage";
import { PresetsPage, newAgent } from "./settings/PresetsPage";
import { ModelsPage } from "./settings/ModelsPage";
import { StoragePage } from "./settings/StoragePage";
import { ArchivedChatsPage } from "./settings/ArchivedChatsPage";
import { WorkersPage } from "./settings/WorkersPage";
import { ClonesPage } from "./settings/ClonesPage";
import { STATIC_SETTINGS_ROWS, type SearchableRow } from "./settings/settingsSearch";
import { primarySection, SECTION_LABELS, type Section } from "./settings/sections";
import { VoiceSettingsPage } from "./settings/VoiceSettingsPage";

export type { Section };
export { PermissionsSection };

export function adapterSupportsAgentRole(adapter: AdapterDescriptor, role: string): boolean {
  const supportsSandbox = (mode: "read_only" | "workspace_write") => !adapter.sandboxModes?.length || adapter.sandboxModes.includes(mode);
  if (role === "orchestrator") return adapter.capabilities.includes("briefings") && supportsSandbox("workspace_write");
  if (role === "implementation") return supportsSandbox("workspace_write");
  return supportsSandbox("read_only");
}

export function SettingsScreen({ adapters, autoApprovals = [], initialSection = "general", onModelSetupChange, onSuggestionSettingsChange, onVoiceChanged, onOpenWorkBoard, onHealthChange = () => undefined, contextual = false, availableUpdate, onUpdate = () => undefined, onError, onAskBridge }: { adapters: AdapterDescriptor[]; autoApprovals?: BridgeEvent[]; initialSection?: Section; onOpenWorkBoard?: () => void; onModelSetupChange: (setup: ModelSetupState) => void; onSuggestionSettingsChange: (snapshot: SuggestionSettingsSnapshot) => void; onHealthChange?: () => void; onVoiceChanged?: () => void; contextual?: boolean; availableUpdate?: UpdateInfo; onUpdate?: (update: UpdateInfo | undefined) => void; onError: (message: string) => void; /** Start a Bridge chat with this first message (the storage copilot). */ onAskBridge?: (prompt: string) => void }) {
  const [section, setSection] = useState<Section>(initialSection);
  useEffect(() => {
    let active = true;
    let off: (() => void) | undefined;
    void bridgeApi.onMenuBarSettings(() => { if (active) setSection("menuBar"); }).then(fn => {
      if (active) off = fn; else fn();
    });
    return () => { active = false; off?.(); };
  }, []);
  const [query, setQuery] = useState("");
  const [config, setConfig] = useState<ConfigState>();
  const [modelSetup, setModelSetup] = useState<ModelSetupState>();
  const [profiles, setProfiles] = useState<ModelProfileDraft[]>([]);
  // Preset edits, keyed by preset id ("" is the one being created). Kept apart
  // from `presetDetailId` so leaving the page and coming back lands on the list
  // without throwing away what was typed, exactly as the harness and prompt
  // drafts behave.
  const [agentDrafts, setAgentDrafts] = useState<Record<string, AgentDefinition>>({});
  const [presetDetailId, setPresetDetailId] = useState<string | null>(null);
  // Unsaved *text* only. Switches and selects never enter this map: they write
  // the stored record directly, so a dirty system prompt cannot ride along with
  // a toggle. Held here rather than in the page so a draft survives navigating
  // away and back, as Prompt Studio's drafts do.
  const [harnessDrafts, setHarnessDrafts] = useState<Record<string, HarnessDraft>>({});
  const [harnessDetailId, setHarnessDetailId] = useState<string | null>(null);
  const [openCodeCatalog, setOpenCodeCatalog] = useState<OpenCodeCatalog>();
  const [openCodeDiscoveryError, setOpenCodeDiscoveryError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);
  // One copy of the runtime list, read by both the Harnesses list and a single
  // harness's detail page, so an install never has to be reported twice.
  const managed = useManagedAgents(undefined, onHealthChange);
  useEffect(() => { setSection(initialSection); setPresetDetailId(null); setHarnessDetailId(null); }, [initialSection]);

  useEffect(() => {
    let active = true;
    setBusy(true);
    Promise.all([bridgeApi.configState(), bridgeApi.modelSetup()]).then(([next, setup]) => {
      if (!active) return;
      setConfig(next);
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

  const modelOptions = useMemo(() => {
    const staticOptions = adapters.filter(item => item.available && item.id !== "opencode").flatMap(adapter => adapter.models.map(model => ({ adapter: adapter.id, id: model.id, label: `${adapter.label} · ${model.label}` })));
    const stored = (config?.harnesses.find(item => item.id === "opencode")?.advanced ?? {}) as OpenCodeAdvancedSettings;
    const visible = new Set(stored.visibleModels ?? []);
    const openCodeOptions = openCodeCatalog?.providers.flatMap(provider => provider.connected ? provider.models.filter(model => visible.size === 0 || visible.has(model.id)).map(model => ({ adapter: "opencode", id: model.id, label: `${provider.name} · ${model.label}` })) : []) ?? [];
    return [...staticOptions, ...openCodeOptions];
  }, [adapters, openCodeCatalog, config]);
  const flashSaved = () => { setSaved(true); window.setTimeout(() => setSaved(false), 1600); };
  const acceptConfig = (next: ConfigState) => setConfig(next);

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

  /** Throws on failure so the caller can decide between a Saved flash and an
   *  error: a page that swallowed this would flash "Saved" on a refusal. */
  const saveAgent = async (next: AgentDefinition) => {
    setBusy(true);
    try {
      const stored = await bridgeApi.saveAgentConfig(next);
      acceptConfig(stored);
      const savedAgent = stored.agents.find(item => item.id === next.id)
        ?? stored.agents.find(item => item.name === next.name);
      // The draft is spent: the stored record is now the truth for this preset,
      // and a newly created one moves from the "" slot to its real id.
      setAgentDrafts(current => {
        const drafts = { ...current };
        delete drafts[next.id ?? ""];
        return drafts;
      });
      if (savedAgent) setPresetDetailId(savedAgent.id ?? "");
      flashSaved();
    } finally { setBusy(false); }
  };

  const removeAgent = async (agent: AgentDefinition) => {
    const verb = agent.isBuiltIn ? "Reset" : "Delete";
    if (!window.confirm(`${verb} ${agent.name}?`)) return;
    setBusy(true);
    try {
      const stored = await bridgeApi.deleteAgentConfig(agent.id ?? "");
      acceptConfig(stored);
      // A deleted preset has no detail page left to sit on; a reset one is
      // reloaded from what the host returned.
      setAgentDrafts(current => {
        const drafts = { ...current };
        delete drafts[agent.id ?? ""];
        return drafts;
      });
      const restored = stored.agents.find(item => item.id === agent.id);
      setPresetDetailId(restored ? (restored.id ?? null) : null);
      flashSaved();
    }
    catch (error) { onError(String(error)); } finally { setBusy(false); }
  };

  const makeDefault = async (agent: AgentDefinition) => {
    setBusy(true);
    try {
      const stored = await bridgeApi.setDefaultAgent(agent.id ?? "");
      acceptConfig(stored);
      setAgentDrafts(current => {
        const drafts = { ...current };
        delete drafts[agent.id ?? ""];
        return drafts;
      });
      flashSaved();
    }
    catch (error) { onError(String(error)); } finally { setBusy(false); }
  };

  const saveHarness = async (next: HarnessConfig) => {
    setBusy(true);
    try {
      acceptConfig(await bridgeApi.saveHarnessConfig(next));
      if (next.id === "opencode") {
        setOpenCodeCatalog(await bridgeApi.refreshOpenCodeCatalog());
        setOpenCodeDiscoveryError(undefined);
      }
      flashSaved();
    } finally { setBusy(false); }
  };

  const resetHarness = async (id: string) => {
    setBusy(true);
    try {
      acceptConfig(await bridgeApi.resetHarnessConfig(id as HarnessConfig["id"]));
      setHarnessDrafts(current => { const draft = { ...current }; delete draft[id]; return draft; });
      if (id === "opencode") {
        setOpenCodeCatalog(await bridgeApi.refreshOpenCodeCatalog());
        setOpenCodeDiscoveryError(undefined);
      }
      flashSaved();
    }
    catch (error) { onError(String(error)); } finally { setBusy(false); }
  };

  const saveModels = async (next: ModelProfileDraft[]) => {
    setBusy(true);
    try {
      const setup = await bridgeApi.saveModelProfiles(next);
      setModelSetup(setup); setProfiles(profileDraftsFromSetup(setup)); onModelSetupChange(setup); flashSaved();
    } finally { setBusy(false); }
  };

  const resetEverything = async () => {
    setBusy(true);
    try {
      const [next, setup] = await Promise.all([bridgeApi.resetAllConfig(), bridgeApi.resetModelProfiles()]);
      acceptConfig(next); setAgentDrafts({}); setPresetDetailId(null); setHarnessDrafts({}); setHarnessDetailId(null);
      setModelSetup(setup); setProfiles(profileDraftsFromSetup(setup)); onModelSetupChange(setup); flashSaved();
    }
    catch (error) { onError(String(error)); } finally { setBusy(false); }
  };

  const navigate = (next: Section) => { setSection(next); setQuery(""); setPresetDetailId(null); setHarnessDetailId(null); };
  const agentPreferences = <>
    <SettingsGroup label="How Bridge works">
      <SettingsRow label="Model preferences" description="Choose models for chats and background tasks. Start with Bridge's defaults." onOpen={() => navigate("models")} />
      <SettingsRow label="Saved setups" description="Reuse your own agent choices and instructions. Also available beside New Chat." onOpen={() => navigate("agents")} />
    </SettingsGroup>
    <details className="text-ui"><summary className="cursor-pointer text-muted-foreground">Advanced</summary><div className="mt-3">
      <SettingsGroup>
        <SettingsRow label="Background tasks" description="Tasks running at once, retries, and how long Bridge waits for an agent." onOpen={() => navigate("workers")} />
        <SettingsRow label="Bridge instructions" description="Change instructions for Bridge's built-in roles. Permissions still apply." onOpen={() => navigate("prompts")} />
      </SettingsGroup>
    </div></details>
  </>;
  const generalLinks = <SettingsGroup label="Preferences">
    <SettingsRow label="Voice" description="Private, on-device dictation and local speech model setup." onOpen={() => navigate("voice")} />
    <SettingsRow label="Typing & search" description="Text suggestions while you type, and how Bridge searches chat history." onOpen={() => navigate("composer")} />
    <SettingsRow label="Menu bar" description="Show account usage and spend in the macOS menu bar." onOpen={() => navigate("menuBar")} />
    <SettingsRow label="Updates" description={availableUpdate ? `Bridge ${availableUpdate.version} is available.` : "Check for new versions of Bridge."} onOpen={() => navigate("updates")} />
  </SettingsGroup>;
  const dataLinks = <SettingsGroup label="Chat history">
    <SettingsRow label="Archived chats" description="Search, read and unarchive conversations." onOpen={() => navigate("archives")} />
    <SettingsRow label="Import history" description="Bring supported chat history and configuration from another coding agent." onOpen={() => navigate("import")} />
  </SettingsGroup>;
  return <div className="flex h-full min-h-0 flex-col md:flex-row">
    {!contextual && <SettingsRail
      section={section}
      query={query}
      rows={searchRows}
      resetting={busy}
      onQueryChange={setQuery}
      // A rail item is named for a page, not for whatever detail was last open
      // on it, so it lands on the list. Drafts survive; only the routing resets.
      onSelect={navigate}
      onResetAll={() => void resetEverything()}
    />}
    <div className="relative min-h-0 min-w-0 flex-1 overflow-y-auto">

      {busy && !config ? <div className="grid h-full place-items-center"><CircleNotch className="animate-spin text-muted-foreground" size={18} strokeWidth={1.7} /></div> : null}

      {!contextual && section !== primarySection(section) && section !== "harnesses" && <div className="mx-auto w-full max-w-page px-5 pt-4 sm:px-8"><TextButton onClick={() => navigate(primarySection(section))}>Back to {SECTION_LABELS[primarySection(section)]}</TextButton></div>}
      {section === "general" && <AppearancePage title="General" extra={<><ActiveTurnInputSetting onError={onError} />{generalLinks}</>} />}
      {section === "data" && <StoragePage title="Data & storage" extra={dataLinks} onError={onError} onAskBridge={onAskBridge} />}
      {section === "appearance" && <AppearancePage />}
      {section === "updates" && <UpdatesPage availableUpdate={availableUpdate} onUpdate={onUpdate} />}
      {section === "menuBar" && <MenuBarSettingsPage />}

      {section === "permissions" && config && <PermissionsSection
        policy={config.permissionPolicy}
        autoApprovals={autoApprovals}
        busy={busy}
        saved={saved}
        onChange={policy => void savePolicy(policy)}
        extra={<SettingsGroup label="Browser access"><SettingsRow label="Browser copies" description="How agents use a separate browser for work that needs sign-in." onOpen={() => navigate("clones")} /></SettingsGroup>}
      />}

      {section === "composer" && <ComposerPage adapters={adapters} onChange={onSuggestionSettingsChange} onError={onError} />}
      {section === "voice" && <VoiceSettingsPage onError={onError} onChanged={onVoiceChanged} />}

      {section === "prompts" && <PromptStudio />}
      {section === "import" && <ImportHarnessSection onError={onError} />}
      {section === "storage" && <StoragePage onError={onError} onAskBridge={onAskBridge} />}
      {section === "archives" && <ArchivedChatsPage />}
      {section === "workers" && <WorkersPage adapters={adapters} />}
      {section === "clones" && <ClonesPage onError={onError} />}
      {section === "work" && <WorkSettingsSection onError={onError} onOpenBoard={onOpenWorkBoard} />}

      {section === "agents" && config && <PresetsPage
        agents={config.agents}
        adapters={adapters}
        modelOptions={modelOptions}
        busy={busy}
        draft={presetDetailId === null ? undefined : (agentDrafts[presetDetailId] ?? config.agents.find(item => item.id === presetDetailId))}
        supportsRole={adapterSupportsAgentRole}
        onOpen={agent => setPresetDetailId(agent.id ?? "")}
        onClose={() => setPresetDetailId(null)}
        onNew={() => { setAgentDrafts(current => ({ ...current, "": newAgent() })); setPresetDetailId(""); }}
        onDraft={next => setAgentDrafts(current => ({ ...current, [next.id ?? ""]: next }))}
        onSave={saveAgent}
        onRemove={agent => void removeAgent(agent)}
        onMakeDefault={agent => void makeDefault(agent)}
        onError={onError}
      />}

      {(section === "harnesses" || section === "codingAgents") && config && <HarnessesPage
        adapters={adapters}
        harnesses={config.harnesses}
        modelOptions={modelOptions}
        managed={managed}
        preferences={agentPreferences}
        busy={busy}
        openCodeCatalog={openCodeCatalog}
        openCodeDiscoveryError={openCodeDiscoveryError}
        drafts={harnessDrafts}
        detailId={harnessDetailId}
        onOpenDetail={setHarnessDetailId}
        onCloseDetail={() => setHarnessDetailId(null)}
        onDraft={(id, patch) => setHarnessDrafts(current => ({ ...current, [id]: { ...current[id], ...patch } }))}
        onDiscard={id => setHarnessDrafts(current => { const next = { ...current }; delete next[id]; return next; })}
        onSaveHarness={saveHarness}
        onResetHarness={resetHarness}
        onCatalog={catalog => { setOpenCodeCatalog(catalog); setOpenCodeDiscoveryError(undefined); }}
        onError={message => { setOpenCodeDiscoveryError(message); onError(message); }}
        onAuthenticationChanged={onHealthChange}
      />}

      {section === "models" && modelSetup && <ModelsPage
        profiles={profiles}
        adapters={adapters}
        version={modelSetup.activeVersion ?? null}
        busy={busy}
        onSave={saveModels}
        onRefreshCatalogs={async () => { await bridgeApi.refreshModelCatalogs(); }}
        onError={onError}
      />}
    </div>
  </div>;
}
