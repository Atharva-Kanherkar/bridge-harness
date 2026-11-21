import { useEffect, useMemo, useRef, useState } from "react";
import { SkeletonRows } from "@/components/ui/skeleton";
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
import { SettingsRail } from "./settings/SettingsRail";
import { SettingsPage } from "./settings/kit";
import { AppearancePage } from "./settings/AppearancePage";
import { UpdatesPage } from "./settings/UpdatesPage";
import { NotificationsPage } from "./settings/NotificationsPage";
import type { UpdateInfo } from "../updater";
import { PermissionsSection } from "./settings/PermissionsPage";
import { ComposerPage } from "./settings/ComposerPage";
import { HarnessesPage, type HarnessDraft } from "./settings/HarnessesPage";
import { PresetsPage, newAgent } from "./settings/PresetsPage";
import { ModelsPage } from "./settings/ModelsPage";
import { StoragePage } from "./settings/StoragePage";
import type { StorageCopilotHost } from "./settings/StorageCopilot";
import { ArchivedChatsPage } from "./settings/ArchivedChatsPage";
import { WorkersPage } from "./settings/WorkersPage";
import { KeysPage } from "./settings/KeysPage";
import { PAGE_ROWS, SETTINGS_ROWS, type SearchableRow } from "./settings/settingsSearch";
import { canonicalSection, PAGES, type PageId, type Section } from "./settings/sections";
import { ChatDefaultsPage } from "./settings/ChatDefaultsPage";
import { VoiceSettingsPage } from "./settings/VoiceSettingsPage";

export type { PageId, Section };
export { PermissionsSection };

export function adapterSupportsAgentRole(adapter: AdapterDescriptor, role: string): boolean {
  const supportsSandbox = (mode: "read_only" | "workspace_write") => !adapter.sandboxModes?.length || adapter.sandboxModes.includes(mode);
  if (role === "orchestrator") return adapter.capabilities.includes("briefings") && supportsSandbox("workspace_write");
  if (role === "implementation") return supportsSandbox("workspace_write");
  return supportsSandbox("read_only");
}

/** Pages drawn from the shared config or model setup. Only these wait on that
 *  load; every other page fetches its own data and shows its own skeleton. */
const CONFIG_PAGES: ReadonlySet<PageId> = new Set(["chatDefaults", "permissions", "agents", "codingAgents", "models"]);

export function SettingsScreen({ adapters, autoApprovals = [], initialSection = "general", onModelSetupChange, onSuggestionSettingsChange, onVoiceChanged, onOpenWorkBoard, onExit, onHealthChange = () => undefined, availableUpdate, onUpdate = () => undefined, onError, storageCopilot }: { adapters: AdapterDescriptor[]; autoApprovals?: BridgeEvent[]; initialSection?: Section; onOpenWorkBoard?: () => void; /** Leave Settings; the rail's Back row. */ onExit?: () => void; onModelSetupChange: (setup: ModelSetupState) => void; onSuggestionSettingsChange: (snapshot: SuggestionSettingsSnapshot) => void; onHealthChange?: () => void; onVoiceChanged?: () => void; availableUpdate?: UpdateInfo; onUpdate?: (update: UpdateInfo | undefined) => void; onError: (message: string) => void; /** The storage page's docked chat. */ storageCopilot?: StorageCopilotHost }) {
  // Always a real page: a retired id from an old deep link lands on the page
  // that replaced it.
  const [section, setSection] = useState<PageId>(() => canonicalSection(initialSection));
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
  useEffect(() => { setSection(canonicalSection(initialSection)); setPresetDetailId(null); setHarnessDetailId(null); }, [initialSection]);
  // The row a search hit asked for, revealed once its page has drawn it.
  const [reveal, setReveal] = useState<string | null>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const highlightTimers = useRef<number[]>([]);
  useEffect(() => () => { for (const id of highlightTimers.current) window.clearTimeout(id); }, []);
  useEffect(() => {
    if (!reveal) return;
    let cancelled = false;
    let attempts = 0;
    let retry: number | undefined;
    // Pages load their data after they mount, so the row may not exist on the
    // first frame. Look for it for a couple of seconds, then give up quietly.
    const attempt = () => {
      if (cancelled) return;
      const row = contentRef.current?.querySelector<HTMLElement>(`[data-settings-row="${reveal}"]`);
      if (!row) {
        if (++attempts < 40) retry = window.setTimeout(attempt, 50); else setReveal(null);
        return;
      }
      for (let parent = row.parentElement; parent; parent = parent.parentElement) {
        if (parent instanceof HTMLDetailsElement) parent.open = true;
      }
      row.scrollIntoView?.({ block: "center", behavior: "smooth" });
      row.setAttribute("data-search-highlight", "");
      highlightTimers.current.push(window.setTimeout(() => row.removeAttribute("data-search-highlight"), 1600));
      setReveal(null);
    };
    attempt();
    return () => { cancelled = true; if (retry !== undefined) window.clearTimeout(retry); };
  }, [reveal, section, config, modelSetup]);

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

  /** Everything the rail search can reach: every page, every fixed row, and
   *  whatever the user's own configuration named. A harness or an agent opens
   *  its own detail page. */
  const searchRows: SearchableRow[] = useMemo(() => [
    ...PAGE_ROWS,
    ...SETTINGS_ROWS,
    ...(config?.harnesses ?? []).map(item => ({
      page: "codingAgents" as const,
      label: item.label,
      description: item.isOverride ? "Customized" : "Bridge defaults",
      target: { kind: "harness" as const, id: item.id },
    })),
    ...(config?.agents ?? []).filter(item => item.id).map(item => ({
      page: "agents" as const,
      label: item.name,
      description: `${item.role} · ${item.harness}`,
      target: { kind: "agent" as const, id: item.id! },
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

  const navigate = (next: PageId) => { setSection(next); setQuery(""); setPresetDetailId(null); setHarnessDetailId(null); };
  const openSearchHit = (row: SearchableRow) => {
    navigate(row.page);
    if (row.target?.kind === "harness") setHarnessDetailId(row.target.id);
    if (row.target?.kind === "agent") setPresetDetailId(row.target.id);
    if (row.id) setReveal(row.id);
  };
  const waitingForConfig = CONFIG_PAGES.has(section) && (section === "models" ? !modelSetup : !config);
  return <div className="flex h-full min-h-0 flex-col md:flex-row">
    <SettingsRail
      section={section}
      query={query}
      rows={searchRows}
      resetting={busy}
      availableUpdate={availableUpdate}
      onQueryChange={setQuery}
      // A rail item is named for a page, not for whatever detail was last open
      // on it, so it lands on the list. Drafts survive; only the routing resets.
      onSelect={navigate}
      onSearchSelect={openSearchHit}
      onResetAll={() => void resetEverything()}
      onExit={onExit}
    />
    <div className="flex min-h-0 min-w-0 flex-1 flex-col bg-background">
    {/* The window's title strip over the page column: it drags the window and
        lines the first heading up with the rail's search, nothing more. */}
    <div data-tauri-drag-region="deep" aria-hidden="true" className="hidden h-11 shrink-0 md:block" />
    <div ref={contentRef} className="relative min-h-0 min-w-0 flex-1 overflow-y-auto">

      {/* The same loading look as every page that fetches its own data: the
          real title, then skeleton rows where the groups will be. */}
      {waitingForConfig && busy ? <SettingsPage title={PAGES[section].label} description={PAGES[section].description}>
        <SkeletonRows label="Loading settings" rows={4} trailing />
      </SettingsPage> : null}

      {section === "chatDefaults" && config && <ChatDefaultsPage adapters={adapters} agents={config.agents} />}
      {section === "general" && <AppearancePage />}
      {section === "data" && <StoragePage onError={onError} copilot={storageCopilot} />}
      {section === "updates" && <UpdatesPage availableUpdate={availableUpdate} onUpdate={onUpdate} />}
      {section === "menuBar" && <MenuBarSettingsPage />}
      {section === "notifications" && <NotificationsPage />}

      {section === "permissions" && config && <PermissionsSection
        policy={config.permissionPolicy}
        autoApprovals={autoApprovals}
        busy={busy}
        saved={saved}
        onChange={policy => void savePolicy(policy)}
      />}

      {section === "composer" && <ComposerPage adapters={adapters} onChange={onSuggestionSettingsChange} onError={onError} />}
      {section === "voice" && <VoiceSettingsPage onError={onError} onChanged={onVoiceChanged} />}

      {section === "prompts" && <PromptStudio />}
      {section === "import" && <ImportHarnessSection onError={onError} />}
      {section === "archives" && <ArchivedChatsPage />}
      {section === "workers" && <WorkersPage adapters={adapters} />}
      {section === "keys" && <KeysPage onError={onError} />}
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

      {section === "codingAgents" && config && <HarnessesPage
        adapters={adapters}
        harnesses={config.harnesses}
        modelOptions={modelOptions}
        managed={managed}
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
    </div>
  </div>;
}
