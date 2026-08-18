import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { applyFileMention as insertFileMention, fileMentionQuery } from "./fileMentions";
import { Activity, Archive, Bot, Check, ChevronDown, CircleDot, Clock3, Code2, FileCode2, FileDiff, FileText, GitBranch, GitCommitHorizontal, GitPullRequest, Inbox, LayoutGrid, LoaderCircle, Maximize2, MessageSquareText, Minimize2, Monitor, PanelLeft, Play, Plus, Search, Settings2, Square, TerminalSquare, X } from "lucide-react";
import { bridgeApi } from "./api";
import { appendAgentEventBatch } from "./agentEvents";
import type { AgentEvent, ApprovalDecision, BridgeState, CapabilitySuggestion, Harness, Health, ModelSetupState, Project, RiskTier, Session, SessionForestSnapshot, SessionStatus, SkillProvider, WorkerRepositoryBinding, Workspace, WorkspaceChangesResult, WorkspaceFileChange } from "./types";
import { AgentConversation } from "./components/AgentConversation";
import { BridgeSidebar } from "./components/BridgeSidebar";
import { MissionControl } from "./components/MissionControl";
import { isVisibleWorker } from "./components/workerStatus";
import { ComposerPill } from "./components/ComposerPill";
import { BrowserSurface } from "./components/BrowserSurface";
import { PatchView } from "./components/DiffView";
import { WorkspaceCreateDialog } from "./components/WorkspaceCreateDialog";
import { OrchestratorCreateDialog } from "./components/OrchestratorCreateDialog";
import { RouterSettingsDialog } from "./components/RouterSettingsDialog";
import { ModelSetupWizard } from "./components/ModelSetupWizard";
import { UsageWidget } from "./components/UsageWidget";
import { formatElapsed, harnessLabel, tierRuntimeLabel } from "./utils";
import { projectSessionConversation, reduceConversation } from "./conversation";
import { resolveProfileOption, shouldRequireModelSetup } from "./modelProfiles";
import { pickGreeting } from "./greetings";
import { useThemePreference } from "./theme";
import { cn } from "@/lib/utils";
import { buildCacheDiagnostics, buildUsageHistory, clampPercent, extractUsageSnapshot, type UsageProvider, type UsageRateSample, type UsageSnapshot } from "./usage";
import { describeError, errorMessage } from "./errors";
import { forestSnapshotKey, mergeForestSnapshot } from "./forest";
import { queueExplanation, restorationPresentation, turnBudget } from "./observability";
import { startSerialPoll } from "./polling";
import { Alert, AlertAction, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { InputGroup, InputGroupAddon, InputGroupInput } from "@/components/ui/input-group";
import { Kbd } from "@/components/ui/kbd";
import { Tabs, TabsList, TabsTab } from "@/components/ui/tabs";

const MarketplaceScreen = lazy(() => import("./components/MarketplaceScreen").then(module => ({ default: module.MarketplaceScreen })));
const SettingsScreen = lazy(() => import("./components/SettingsScreen").then(module => ({ default: module.SettingsScreen })));
const TerminalPane = lazy(() => import("./components/TerminalPane").then(module => ({ default: module.TerminalPane })));
const CodePanel = lazy(() => import("./components/CodePanel").then(module => ({ default: module.CodePanel })));
// Lazy for the same reason as CodePanel: CodeMirror is a large dependency, and
// a static import here would drag it into the startup bundle for everyone,
// including sessions that never open a diff.
const InlineFileEditor = lazy(() => import("./components/editor/InlineFileEditor").then(module => ({ default: module.InlineFileEditor })));

const emptyState: BridgeState = { projects: [], workspaces: [], sessions: [], events: [] };
const statusCopy: Record<SessionStatus, string> = { idle: "IDLE", starting: "STARTING", working: "WORKING", waiting: "NEEDS YOU", warm: "WARM", checkpointing: "CHECKPOINTING", ready: "READY", stopped: "STOPPED", resuming: "RESUMING", restored: "RESTORED", failed: "FAILED", completed: "COMPLETED", cancelled: "CANCELLED" };
const liveStatuses: SessionStatus[] = ["working", "waiting", "ready"];

function orderSessionTree(sessions: Session[]): Session[] {
  const byParent = new Map<string | undefined, Session[]>();
  for (const s of sessions) {
    const key = s.parentSessionId ?? undefined;
    const list = byParent.get(key) ?? [];
    list.push(s); byParent.set(key, list);
  }
  const out: Session[] = [];
  const visit = (parent: string | undefined) => { for (const s of byParent.get(parent) ?? []) { out.push(s); visit(s.id); } };
  visit(undefined);
  for (const s of sessions) if (!out.includes(s)) out.push(s);
  return out;
}

function StatusDot({ status }: { status: SessionStatus }) {
  const color = status === "working" ? "bg-success" : status === "waiting" ? "bg-warning" : status === "ready" ? "bg-info" : status === "failed" ? "bg-destructive" : "bg-ring";
  return <span className={`w-1.5 h-1.5 rounded-full shrink-0 ${color}`} />;
}

export function App() {
  const [state, setState] = useState<BridgeState>(emptyState);
  const [agentEvents, setAgentEvents] = useState<AgentEvent[]>([]);
  const [health, setHealth] = useState<Health>();
  const [modelSetup, setModelSetup] = useState<ModelSetupState>();
  const [selectedSessionId, setSelectedSessionId] = useState<string>();
  const [view, setView] = useState<"workspace" | "marketplace" | "settings">("workspace");
  const [navOpen, setNavOpen] = useState(false);
  // Two ways to look at the workspace: the classic single-session view, or the
  // Mission Control grid where every live agent is its own window at once.
  const [paradigm, setParadigm] = useState<"single" | "grid">("single");
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [activeTab, setActiveTab] = useState<"agent" | "changes" | "code" | "events" | "terminal">("agent");
  // Fullscreen is a property of the workspace surface, not of one tab: it
  // drops the sidebar and the session header so the active tab gets the whole
  // window. The tab strip stays, because it is also the way back out.
  const [fullscreen, setFullscreen] = useState(false);
  // Tabs mount on first visit and then stay mounted. Unmounting the Changes
  // and Code panels on every tab switch would throw away open files, expanded
  // diffs, and — now that both tabs can edit — unsaved text.
  const [visitedTabs, setVisitedTabs] = useState<Set<string>>(() => new Set(["agent"]));
  useEffect(() => { setVisitedTabs(previous => previous.has(activeTab) ? previous : new Set(previous).add(activeTab)); }, [activeTab]);
  const [modal, setModal] = useState<"chat" | "workspace" | "orchestrator" | "router" | null>(null);
  const [pendingWorkspaceId, setPendingWorkspaceId] = useState<string>();
  const [title, setTitle] = useState("");
  const [composer, setComposer] = useState("");
  const [slashCommands, setSlashCommands] = useState<import("./types").SlashCommand[]>([]);
  const [slashIndex, setSlashIndex] = useState(0);
  const [slashDismissed, setSlashDismissed] = useState(false);
  const [workspaceFiles, setWorkspaceFiles] = useState<string[]>([]);
  const [mentionIndex, setMentionIndex] = useState(0);
  const [mentionDismissed, setMentionDismissed] = useState(false);
  const [skillSuggestions, setSkillSuggestions] = useState<CapabilitySuggestion[]>([]);
  const [busy, setBusy] = useState(false);
  const [browserOpen, setBrowserOpen] = useState(false);
  const [error, setError] = useState<string>();
  const [forest, setForest] = useState<SessionForestSnapshot>();
  // Completion blocks while a child's changes live only in its own worktree, so
  // the user must be able to see and resolve that here — otherwise the session
  // waits forever with no visible cause.
  const [pendingAdoptions, setPendingAdoptions] = useState<WorkerRepositoryBinding[]>([]);
  const [pending, setPending] = useState<{ key: string; sessionId: string; text: string }[]>([]);
  const [usageByProvider, setUsageByProvider] = useState<Partial<Record<UsageProvider, UsageSnapshot>>>({});
  const [usageSamples, setUsageSamples] = useState<Partial<Record<UsageProvider, UsageRateSample[]>>>({});
  const startedRef = useRef<Set<string>>(new Set());
  const pendingWelcomeMessageRef = useRef<string | null>(null);
  const forestKeyRef = useRef("");
  const agentEventQueueRef = useRef<AgentEvent[]>([]);
  const agentEventTimerRef = useRef<number | undefined>(undefined);
  const browserSessionRef = useRef<string>();

  const reload = useCallback(async () => { setState(await bridgeApi.state()); }, []);
  useEffect(() => {
    void Promise.all([reload(), bridgeApi.modelSetup()])
      .then(([, setup]) => { setModelSetup(setup); })
      .catch(value => setError(errorMessage(value)));
    let offState: (() => void) | undefined;
    let offAgent: (() => void) | undefined;
    let offUsage: (() => void) | undefined;
    let offAdapters: (() => void) | undefined;
    let active = true;
    const reloadHealth = () => {
      void bridgeApi.health().then(setHealth).catch(value => setError(errorMessage(value)));
    };
    void bridgeApi.onStateChanged(reload).then(fn => offState = fn);
    // Adapter availability can change after startup (OpenCode catalog discovery
    // runs in the background) — re-read health when the backend says so.
    void bridgeApi.onAdaptersChanged(reloadHealth).then(fn => {
      if (!active) { fn(); return; }
      offAdapters = fn;
      // Fetch only after the listener is installed: discovery can complete
      // during startup, and Tauri events are not buffered for the webview.
      reloadHealth();
    }).catch(value => {
      if (!active) return;
      setError(errorMessage(value));
      reloadHealth();
    });
    const queueAgentEvent = (event: AgentEvent) => {
      agentEventQueueRef.current.push(event);
      if (agentEventTimerRef.current !== undefined) return;
      agentEventTimerRef.current = window.setTimeout(() => {
        const batch = agentEventQueueRef.current.splice(0);
        agentEventTimerRef.current = undefined;
        setAgentEvents(current => appendAgentEventBatch(current, batch));
      }, 50);
    };
    void bridgeApi.onAgentEvent(queueAgentEvent).then(fn => offAgent = fn);
    void bridgeApi.onAccountUsage(payload => {
      const snapshot = extractUsageSnapshot({ rateLimits: payload.rateLimits });
      if (!snapshot) return;
      setUsageByProvider(current => ({ ...current, [payload.provider]: snapshot }));
      if (snapshot.windows.length) {
        const usedPercent = clampPercent(Math.max(...snapshot.windows.map(window => window.usedPercent)));
        setUsageSamples(current => ({
          ...current,
          [payload.provider]: [...(current[payload.provider] ?? []), { usedPercent, capturedAt: snapshot.capturedAt }].slice(-24),
        }));
      }
    }).then(fn => offUsage = fn);
    return () => {
      active = false;
      offState?.(); offAgent?.(); offUsage?.(); offAdapters?.();
      if (agentEventTimerRef.current !== undefined) window.clearTimeout(agentEventTimerRef.current);
      agentEventTimerRef.current = undefined;
      agentEventQueueRef.current = [];
    };
  }, [reload]);
  useThemePreference();
  useEffect(() => { setNavOpen(false); }, [view, selectedSessionId]);
  useEffect(() => {
    const previous = browserSessionRef.current;
    browserSessionRef.current = selectedSessionId;
    if (previous && previous !== selectedSessionId) void bridgeApi.browserBridgeState().then(browser => browser.lease ? bridgeApi.detachBrowser() : undefined).catch(() => undefined);
  }, [selectedSessionId]);

  const adapters = health?.adapters ?? [];
  const adaptersReady = adapters.some(adapter => adapter.available);
  const topSessions = useMemo(() => state.sessions.filter(s => s.harness !== "shell" && !s.parentSessionId), [state.sessions]);
  const standaloneChats = useMemo(() => topSessions.filter(s => !s.workspaceId), [topSessions]);
  // Resolve across every session, not just top-level ones: a worker can be
  // opened directly (from Mission Control or a blocked-approval link) so its own
  // conversation — and the approval card that lives on it — is reachable.
  const session = state.sessions.find(s => s.id === selectedSessionId && s.harness !== "shell");
  const workspace = session?.workspaceId ? state.workspaces.find(w => w.id === session.workspaceId) : undefined;
  const hasRepo = !!workspace?.path;
  const usesIsolatedWorktree = !!session?.cwd && !!workspace?.path && session.cwd !== workspace.path;
  const isDirectChat = session?.kind === "direct";
  // A focused worker is watchable and its approvals are resolvable, but the
  // backend rejects worker turns, so it gets no composer.
  const isWorkerView = !!session?.parentSessionId;
  const sessionConnected = !!session && !session.endedAt && liveStatuses.includes(session.status);
  const sessionEvents = useMemo(() => agentEvents.filter(event => event.sessionId === session?.id), [agentEvents, session?.id]);
  const childWorkers = useMemo(() => session ? state.sessions.filter(worker => {
    if (worker.parentSessionId !== session.id) return false;
    const runtime = forest?.workerRuntimes.find(item => item.sessionId === worker.id);
    return isVisibleWorker(worker, runtime);
  }) : [], [state.sessions, session?.id, forest?.workerRuntimes]);
  const pendingForSession = useMemo(() => pending.filter(p => p.sessionId === session?.id).map(p => p.text), [pending, session?.id]);
  const usageHistory = useMemo(() => buildUsageHistory(forest?.usage ?? [], state.sessions), [forest?.usage, state.sessions]);
  const cacheDiagnostics = useMemo(() => buildCacheDiagnostics(forest?.usage ?? []), [forest?.usage]);
  const latestContext = session?.contextPercent ?? usageHistory.find(entry => entry.contextPercent != null)?.contextPercent;
  const latestContextSource = session?.contextPercent != null ? session.metricSource as import("./usage").MetricSource : usageHistory.find(entry => entry.contextPercent != null)?.source;
  const slashQuery = /^\/([^\s]*)$/.exec(composer)?.[1];
  const slashMatches = useMemo(() => {
    if (slashQuery == null) return [];
    const query = slashQuery.toLowerCase();
    return slashCommands
      .filter(command => !query || command.name.toLowerCase().includes(query) || command.description.toLowerCase().includes(query))
      .sort((a, b) => {
        const aName = a.name.toLowerCase();
        const bName = b.name.toLowerCase();
        const aPrefix = query ? Number(aName.startsWith(query)) : 0;
        const bPrefix = query ? Number(bName.startsWith(query)) : 0;
        if (aPrefix !== bPrefix) return bPrefix - aPrefix;
        const aHarness = Number(a.harness === session?.harness);
        const bHarness = Number(b.harness === session?.harness);
        if (aHarness !== bHarness) return bHarness - aHarness;
        return aName.localeCompare(bName);
      });
  }, [slashQuery, slashCommands, session?.harness]);
  const slashOpen = slashQuery != null && slashMatches.length > 0 && !slashDismissed;
  const slashListRef = useRef<HTMLDivElement>(null);
  // @file mention: match a token being typed at the end of the composer, at the
  // start or after whitespace (so email-style name@host fragments are ignored).
  const mentionQuery = fileMentionQuery(composer);
  const workspaceFileOptions = useMemo(() => workspaceFiles.map(path => {
    const lowerPath = path.toLowerCase();
    return { path, lowerPath, lowerBase: lowerPath.split("/").pop() ?? lowerPath };
  }), [workspaceFiles]);
  const fileMatches = useMemo(() => {
    if (mentionQuery == null) return [];
    const query = mentionQuery.toLowerCase();
    return workspaceFileOptions
      .filter(file => !query || file.lowerPath.includes(query))
      .sort((a, b) => {
        const aPrefix = query ? Number(a.lowerBase.startsWith(query) || a.lowerPath.startsWith(query)) : 0;
        const bPrefix = query ? Number(b.lowerBase.startsWith(query) || b.lowerPath.startsWith(query)) : 0;
        if (aPrefix !== bPrefix) return bPrefix - aPrefix;
        return a.path.length - b.path.length || a.path.localeCompare(b.path);
      })
      .slice(0, 50)
      .map(file => file.path);
  }, [mentionQuery, workspaceFileOptions]);
  const mentionOpen = mentionQuery != null && fileMatches.length > 0 && !mentionDismissed;
  const mentionListRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const query = composer.trim();
    if (!session || !(["codex", "claude", "opencode"] as Harness[]).includes(session.harness) || query.length < 8 || query.startsWith("/")) { setSkillSuggestions([]); return; }
    const provider = session.harness as SkillProvider;
    let active = true;
    const timer = window.setTimeout(() => { void bridgeApi.skillSuggestions(query, provider).then(items => { if (active) setSkillSuggestions(items.slice(0, 3)); }).catch(() => { if (active) setSkillSuggestions([]); }); }, 300);
    return () => { active = false; window.clearTimeout(timer); };
  }, [composer, session]);

  useEffect(() => {
    if (!slashOpen) return;
    setSlashIndex(index => Math.min(index, Math.max(0, slashMatches.length - 1)));
  }, [slashOpen, slashMatches.length]);

  useEffect(() => {
    if (!slashOpen) return;
    const root = slashListRef.current;
    if (!root) return;
    const active = root.querySelector<HTMLElement>(`[data-slash-index="${slashIndex}"]`);
    active?.scrollIntoView({ block: "nearest" });
  }, [slashOpen, slashIndex]);

  // Load the connected workspace's file list for @mention autocomplete.
  useEffect(() => {
    if (!session?.id || !hasRepo) { setWorkspaceFiles([]); return; }
    setWorkspaceFiles([]);
    let active = true;
    void bridgeApi.listWorkspaceFiles(session.id)
      .then(files => { if (active) setWorkspaceFiles(files); })
      .catch(() => { if (active) setWorkspaceFiles([]); });
    return () => { active = false; };
  }, [session?.id, hasRepo]);

  useEffect(() => {
    if (!mentionOpen) return;
    setMentionIndex(index => Math.min(index, Math.max(0, fileMatches.length - 1)));
  }, [mentionOpen, fileMatches.length]);

  useEffect(() => {
    if (!mentionOpen) return;
    const root = mentionListRef.current;
    if (!root) return;
    const active = root.querySelector<HTMLElement>(`[data-mention-index="${mentionIndex}"]`);
    active?.scrollIntoView({ block: "nearest" });
  }, [mentionOpen, mentionIndex]);

  useEffect(() => {
    forestKeyRef.current = "";
    setForest(undefined);
    setPendingAdoptions([]);
    if (!session?.id) return;
    let active = true;
    const refresh = async () => {
      const [value, adoptions] = await Promise.all([
        bridgeApi.sessionForest(session.id).catch(() => undefined),
        bridgeApi.pendingWorkerAdoptions(session.id).catch(() => []),
      ]);
      if (!active) return;
      setPendingAdoptions(adoptions);
      if (!value) return;
      const key = forestSnapshotKey(value);
      if (key === forestKeyRef.current) return;
      forestKeyRef.current = key;
      setForest(current => mergeForestSnapshot(current, value));
    };
    const stop = startSerialPoll(refresh, 3000);
    return () => { active = false; stop(); };
  }, [session?.id]);

  // Keep git stats fresh for the selected chat's connected workspace.
  useEffect(() => {
    const workspaceId = workspace?.id;
    if (!workspaceId || !hasRepo || !("__TAURI_INTERNALS__" in window)) return;
    let active = true;
    const stop = startSerialPoll(async () => {
      const next = await bridgeApi.refreshWorkspace(workspaceId).catch(() => undefined);
      if (active && next) setState(next);
    }, 5000);
    return () => { active = false; stop(); };
  }, [workspace?.id, hasRepo]);

  /** Pull the workspace's Git stats now, rather than waiting out the poll —
   *  a save should move the Changes badge immediately. */
  const refreshWorkspaceStats = useCallback(async (workspaceId: string) => {
    const next = await bridgeApi.refreshWorkspace(workspaceId).catch(() => undefined);
    if (next) setState(next);
  }, []);

  // Poll real subscription usage for every provider, independent of the chat on screen.
  useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;
    return startSerialPoll(() => bridgeApi.refreshAccountUsage().catch(() => undefined), 30_000);
  }, [adaptersReady]);

  // Drop an optimistic message once its real user turn arrives from the backend.
  useEffect(() => {
    setPending(current => {
      if (!current.length) return current;
      const durable = forest?.entries?.length ? projectSessionConversation(forest.entries, forest.head?.activeEntryId ?? null) : [];
      const live = reduceConversation(sessionEvents);
      const userTexts = new Set([...durable, ...live].filter(item => item.type === "message" && item.role === "user").map(item => item.text.trim()));
      const next = current.filter(item => !userTexts.has(item.text.trim()));
      return next.length === current.length ? current : next;
    });
  }, [sessionEvents, forest]);

  // Load available slash commands + skills from signed-in providers.
  useEffect(() => { void bridgeApi.listSlashCommands().then(setSlashCommands).catch(() => undefined); }, [adaptersReady]);

  // Always land on the Agent tab: focusing a session (especially a blocked
  // worker from Mission Control) must reveal its conversation and approval card,
  // not whatever tab — Changes/Terminal — happened to be open before.
  function openSession(id: string) { setView("workspace"); setParadigm("single"); setActiveTab("agent"); setSelectedSessionId(id); }
  // New chat opens instantly (no picker up front). Preserve the current direct
  // chat's harness/model so switching to OpenCode also changes the next-chat
  // default; otherwise fall back to the configured standard profile.
  async function openNewChat(initialMessage?: string) {
    if (!adaptersReady) { setError("No model adapter is available. Install or sign in to Codex, Claude, or OpenCode, then retry model setup."); return; }
    setView("workspace");
    const currentAdapter = session?.kind === "direct"
      ? adapters.find(adapter => adapter.id === session.harness && adapter.available)
      : undefined;
    const profile = modelSetup ? resolveProfileOption("standard_orchestrator", modelSetup, adapters) : undefined;
    const preferred = currentAdapter ?? profile?.adapter ?? adapters.find(adapter => adapter.available) ?? adapters[0];
    const harness = (preferred?.id as Harness) ?? "codex";
    const model = currentAdapter
      ? session?.model ?? currentAdapter.defaultModel ?? currentAdapter.models[0]?.id ?? null
      : profile?.model.id ?? preferred?.defaultModel ?? preferred?.models[0]?.id ?? null;
    const draft = initialMessage?.trim() ?? "";
    if (draft) pendingWelcomeMessageRef.current = draft;
    setBusy(true); setError(undefined);
    try {
      const next = await bridgeApi.createChat(harness, model, null);
      const created = [...next.sessions].reverse().find(s => !s.parentSessionId && !s.workspaceId);
      setState(next);
      if (created) setSelectedSessionId(created.id);
    } catch (e) {
      pendingWelcomeMessageRef.current = null;
      setError(errorMessage(e));
    }
    finally { setBusy(false); }
  }

  useEffect(() => {
    const draft = pendingWelcomeMessageRef.current;
    if (!draft || !session) return;
    pendingWelcomeMessageRef.current = null;
    void sendPrompt(draft);
  }, [session?.id]);
  // Workspace "+": ask whether this orchestrator should get an isolated worktree.
  function requestWorkspaceSession(workspaceId: string) {
    if (!adaptersReady) { setError("No model adapter is available. Install or sign in to Codex, Claude, or OpenCode before starting an orchestrator."); return; }
    setPendingWorkspaceId(workspaceId);
    setModal("orchestrator");
  }
  async function newWorkspaceSession(createWorktree: boolean) {
    if (!pendingWorkspaceId) return;
    const workspaceId = pendingWorkspaceId;
    setBusy(true); setError(undefined);
    try {
      const next = await bridgeApi.createWorkspaceSession(workspaceId, createWorktree);
      const created = [...next.sessions].reverse().find(s => !s.parentSessionId && s.workspaceId === workspaceId);
      setState(next);
      setExpanded(current => new Set(current).add(workspaceId));
      if (created) setSelectedSessionId(created.id);
      setModal(null); setPendingWorkspaceId(undefined);
    } catch (e) { setError(errorMessage(e)); }
    finally { setBusy(false); }
  }
  async function changeChatModel(harness: Harness, model: string | null) {
    if (!session) return;
    setBusy(true); setError(undefined);
    try { setState(await bridgeApi.updateChatModel(session.id, harness, model)); }
    catch (e) { setError(errorMessage(e)); }
    finally { setBusy(false); }
  }
  async function submitNewWorkspace() {
    const name = title.trim(); if (!name) return;
    setBusy(true); setError(undefined);
    try {
      const next = await bridgeApi.createWorkspace(name);
      setState(next); setModal(null); setTitle("");
      const created = [...next.workspaces].reverse()[0];
      if (created) setExpanded(current => new Set(current).add(created.id));
    } catch (e) { setError(errorMessage(e)); }
    finally { setBusy(false); }
  }
  async function connectFolder(workspaceId: string) {
    setError(undefined);
    const value = ("__TAURI_INTERNALS__" in window)
      ? await open({ directory: true, multiple: false, title: "Connect a folder or git repository" })
      : "/Users/you/Developer/project";
    if (!value || typeof value !== "string") return;
    setBusy(true);
    try { setState(await bridgeApi.connectWorkspaceFolder(workspaceId, value)); }
    catch (e) { setError(errorMessage(e)); }
    finally { setBusy(false); }
  }
  async function endChat() {
    if (!session) return; setBusy(true); setError(undefined);
    try {
      const browser = await bridgeApi.browserBridgeState();
      if (browser.lease) await bridgeApi.detachBrowser().catch(() => undefined);
      setState(await bridgeApi.stopSession(session.id));
    }
    catch (e) { setError(errorMessage(e)); }
    finally { setBusy(false); }
  }
  // Send a message. The agent starts lazily on the first message, like a normal
  // chat app — there is no explicit "start" step. Slash commands belonging to
  // another provider auto-switch the direct-chat harness first.
  async function sendPrompt(forcedText?: string) {
    const submittedText = (forcedText ?? composer).trim();
    if (!session || !submittedText) return;
    const key = crypto.randomUUID();
    let target = session;
    let retryText = submittedText;
    setComposer("");
    setSlashIndex(0);
    try {
      const prepared = await bridgeApi.prepareTurn(target.id, submittedText);
      const text = prepared.text;
      retryText = text;
      setPending(current => [...current, { key, sessionId: target.id, text }]);
      const resolved = await bridgeApi.resolveSlashCommand(target.id, text).catch(() => null);
      if (resolved?.switchHarness && target.kind === "direct") {
        const adapter = adapters.find(item => item.id === resolved.harness);
        const next = await bridgeApi.updateChatModel(target.id, resolved.harness as Harness, adapter?.defaultModel ?? null);
        setState(next);
        target = next.sessions.find(item => item.id === target.id) ?? target;
      }
      const localOnly = /^\/(usage|cost|stats|clear|new|reset|compact)(\s|$)/i.test(text);
      if (!localOnly && !liveStatuses.includes(target.status)) {
        startedRef.current.add(target.id);
        setState(await bridgeApi.startChat(target.id));
      }
      await bridgeApi.sendTurn(target.id, text);
      if (localOnly) {
        setPending(current => current.filter(item => item.key !== key));
        await reload();
      }
    }
    catch (e) { setComposer(retryText); setPending(current => current.filter(item => item.key !== key)); setError(errorMessage(e)); }
  }
  const resolveApproval = useCallback(async (eventId: number, decision: ApprovalDecision) => {
    if (!session?.id) return;
    try { await bridgeApi.resolveApproval(session.id, eventId, decision); await reload(); }
    catch (e) { setError(errorMessage(e)); }
  }, [reload, session?.id]);
  const waiveCompletion = useCallback(async (attemptId: string, checkIds: string[], reason: string) => {
    const completion = await bridgeApi.waiveCompletion(attemptId, checkIds, reason);
    setForest(current => current ? { ...current, completion } : current);
  }, []);
  const resolveAdoption = useCallback(async (childSessionId: string, decision: "adopt" | "discard") => {
    if (decision === "adopt") await bridgeApi.adoptWorkerWorktree(childSessionId);
    else await bridgeApi.discardWorkerWorktree(childSessionId, "Discarded from the workspace panel");
    if (!session) return;
    const [next, adoptions] = await Promise.all([
      bridgeApi.sessionForest(session.id),
      bridgeApi.pendingWorkerAdoptions(session.id).catch(() => []),
    ]);
    forestKeyRef.current = forestSnapshotKey(next);
    setForest(next);
    setPendingAdoptions(adoptions);
  }, [session]);
  // The "refresh" half of a stale-base warning. A strict fast-forward, so it
  // refuses rather than rewrites when the workspace has its own commits.
  const refreshWorkspaceBase = useCallback(async () => {
    if (!session) throw new Error("Open a session before refreshing its workspace");
    await bridgeApi.refreshWorkspaceBase(session.id);
    setForest(await bridgeApi.sessionForest(session.id));
  }, [session]);
  async function applySlash(command: import("./types").SlashCommand) {
    if (session?.kind === "direct" && command.harness !== session.harness) {
      const adapter = adapters.find(item => item.id === command.harness);
      try { setState(await bridgeApi.updateChatModel(session.id, command.harness as Harness, adapter?.defaultModel ?? null)); }
      catch (e) { setError(errorMessage(e)); return; }
    }
    setComposer(`/${command.name} `);
    setSlashIndex(0);
    setSlashDismissed(true);
  }
  // Replace the @token being typed at the end of the composer with the picked
  // path, preserving any leading whitespace the mention started after.
  function applyFileMention(path: string) {
    setComposer(current => insertFileMention(current, path));
    setMentionIndex(0);
    setMentionDismissed(true);
  }
  function onComposerKeyDown(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    if (e.nativeEvent.isComposing) return;
    if (mentionOpen) {
      if (e.key === "ArrowDown") { e.preventDefault(); setMentionIndex(index => Math.min(index + 1, fileMatches.length - 1)); return; }
      if (e.key === "ArrowUp") { e.preventDefault(); setMentionIndex(index => Math.max(index - 1, 0)); return; }
      if (e.key === "Escape") { e.preventDefault(); setMentionDismissed(true); return; }
      if ((e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) || e.key === "Tab") { e.preventDefault(); applyFileMention(fileMatches[Math.min(mentionIndex, fileMatches.length - 1)]); return; }
    }
    if (slashOpen) {
      if (e.key === "ArrowDown") { e.preventDefault(); setSlashIndex(index => Math.min(index + 1, slashMatches.length - 1)); return; }
      if (e.key === "ArrowUp") { e.preventDefault(); setSlashIndex(index => Math.max(index - 1, 0)); return; }
      if (e.key === "Escape") { e.preventDefault(); setSlashDismissed(true); return; }
      if ((e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) || e.key === "Tab") { e.preventDefault(); void applySlash(slashMatches[Math.min(slashIndex, slashMatches.length - 1)]); return; }
    }
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) { e.preventDefault(); void sendPrompt(); }
  }

  // ⌥⌘F rather than ⌃⌘F: the latter is macOS's own native-fullscreen binding,
  // and this is an in-window layout change, not a window state change.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.altKey && (event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "f") {
        event.preventDefault();
        setFullscreen(value => !value);
      } else if (event.key === "Escape") {
        setFullscreen(false);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  const toggleExpanded = (id: string) => setExpanded(current => { const next = new Set(current); if (next.has(id)) next.delete(id); else next.add(id); return next; });

  const turnActive = !!session?.activeTurnId || pendingForSession.length > 0;
  if (!health || !modelSetup) return <div className="relative grid h-[100dvh] place-items-center overflow-hidden bg-background text-muted-foreground"><div className="relative z-10 flex max-w-md items-center gap-2 px-6 text-center text-xs">{error ? <><X size={14} className="text-destructive" aria-hidden="true" />{error}</> : <><LoaderCircle className="animate-spin" size={14} aria-hidden="true" />Loading Bridge…</>}</div></div>;
  if (shouldRequireModelSetup(modelSetup, health.adapters)) return <div className="relative h-[100dvh] overflow-hidden bg-background"><ModelSetupWizard adapters={health.adapters} onComplete={setModelSetup} onError={setError} />{error && <Alert variant="error" className="fixed bottom-5 right-5 z-[60] max-w-md"><AlertTitle>Model setup failed</AlertTitle><AlertDescription>{error}</AlertDescription></Alert>}</div>;
  return <div className="relative flex h-[100dvh] overflow-hidden bg-background text-foreground">

    {!fullscreen && <div className="fixed right-2 top-1.5 z-40 flex items-center gap-1.5 sm:right-5 sm:top-5">
      {view === "workspace" && <Button type="button" variant={paradigm === "grid" ? "secondary" : "ghost"} size="sm" className="text-muted-foreground" onClick={() => setParadigm(current => current === "grid" ? "single" : "grid")} aria-pressed={paradigm === "grid"}><LayoutGrid size={13} aria-hidden="true" /> <span className="hidden sm:inline">{paradigm === "grid" ? "Focus" : "Mission Control"}</span></Button>}
      <UsageWidget usage={usageByProvider} samples={usageSamples} history={usageHistory} cacheDiagnostics={cacheDiagnostics} contextPercent={latestContext ?? undefined} contextSource={latestContextSource} />
    </div>}

    {!fullscreen && <div
      className="fixed inset-x-0 top-0 z-20 flex h-11 items-center gap-2 border-b border-border bg-sidebar pl-[84px] pr-[148px] sm:hidden"
      data-tauri-drag-region
    >
      <button
        type="button"
        onClick={() => setNavOpen(true)}
        className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        aria-label="Open navigation"
        aria-expanded={navOpen}
      >
        <PanelLeft size={16} strokeWidth={1.7} aria-hidden="true" />
      </button>
      <span className="min-w-0 flex-1 truncate text-[12.5px] font-medium text-foreground">{view === "marketplace" ? "Marketplace" : view === "settings" ? "Settings" : session?.title || session?.label || "Bridge"}</span>
    </div>}

    {!fullscreen && <BridgeSidebar
      mobileOpen={navOpen}
      onCloseMobile={() => setNavOpen(false)}
      standaloneChats={standaloneChats}
      workspaces={state.workspaces}
      workspaceChats={workspaceId => topSessions.filter(s => s.workspaceId === workspaceId)}
      activeSessionId={session?.id}
      marketplaceActive={view === "marketplace"}
      settingsActive={view === "settings"}
      expanded={expanded}
      busy={busy}
      workers={childWorkers}
      workerRuntimes={forest?.workerRuntimes ?? []}
      workerReasons={forest?.reasons ?? []}
      onOpenNewChat={() => void openNewChat()}
      onOpenMarketplace={() => setView("marketplace")}
      onOpenSettings={() => setView("settings")}
      onOpenSession={openSession}
      onToggleWorkspace={toggleExpanded}
      onNewWorkspace={() => { setTitle(""); setModal("workspace"); }}
      onNewWorkspaceSession={requestWorkspaceSession}
      onConnectFolder={workspaceId => void connectFolder(workspaceId)}
    />}
    <main className={cn("relative z-10 min-w-0 flex-1 overflow-hidden flex flex-col animate-page-mount", fullscreen ? "pt-0" : "pt-11 sm:pt-0")}>
      {!adaptersReady && <Alert variant="warning" className="mx-auto mt-4 w-[calc(100%-2rem)] max-w-2xl"><AlertTitle>No model adapters available</AlertTitle><AlertDescription>Bridge remains accessible, but chats and orchestrators are disabled until Codex, Claude, or OpenCode is installed and signed in.</AlertDescription></Alert>}
      {view === "marketplace" ? <Suspense fallback={<PanelLoading label="Opening marketplace…"/>}><MarketplaceScreen /></Suspense> : view === "settings" ? <Suspense fallback={<PanelLoading label="Opening settings…"/>}><SettingsScreen adapters={adapters} onModelSetupChange={setModelSetup} onError={setError} /></Suspense> : paradigm === "grid" ? <MissionControl
        sessions={state.sessions}
        runtimes={forest?.workerRuntimes ?? []}
        reasons={forest?.reasons ?? []}
        events={agentEvents}
        activeSessionId={session?.id}
        onFocusSession={openSession}
      /> : session ? <>
        {!fullscreen && <div className={`shrink-0 px-4 sm:px-6 flex items-center border-b border-border ${isDirectChat ? "h-[48px]" : "min-h-[52px] py-2"}`}>
          <div className="min-w-0 flex-1">
            <h1 className="m-0 font-display text-sm sm:text-[15px] leading-tight text-foreground font-semibold tracking-tight whitespace-nowrap overflow-hidden text-ellipsis">{session.title || session.label}</h1>
            {!isDirectChat && <div className="mt-1 flex items-center gap-1.5 text-muted-foreground font-mono text-[10px]">
              <Bot size={12} aria-hidden="true" />{session.kind === "orchestrator" ? "Orchestrator" : harnessLabel(session.harness)}<span>·</span>{harnessLabel(session.harness)}<span>·</span>{modelDisplayName(adapters, session.harness, session.model)}
              {hasRepo && workspace && (usesIsolatedWorktree
                ? <><span>·</span><GitBranch size={12} aria-hidden="true" />isolated worktree</>
                : <><span>·</span><GitBranch size={12} aria-hidden="true" />{workspace.branch ?? "folder"}<span>·</span>{workspace.dirtyFiles ? <span className="text-warning">{workspace.dirtyFiles} changed</span> : <span>clean</span>}</>)}
            </div>}
          </div>
          <div className="ml-auto flex items-center gap-[7px]">
            <Button type="button" variant={browserOpen ? "secondary" : "ghost"} size="sm" className="text-muted-foreground" onClick={() => setBrowserOpen(value => !value)}><Monitor size={13} aria-hidden="true" /> Browser</Button>
            {!isDirectChat && workspace && <Button type="button" variant="ghost" size="icon-sm" className="text-muted-foreground" onClick={() => setModal("router")} aria-label="Learning router settings"><Settings2 size={14} aria-hidden="true" /></Button>}
            {sessionConnected && <Button type="button" variant="ghost" size="sm" className="text-muted-foreground hover:text-destructive" disabled={busy} onClick={() => void endChat()}>{busy ? <LoaderCircle className="animate-spin" size={14} aria-hidden="true" /> : <Square size={13} aria-hidden="true" />} End</Button>}
          </div>
        </div>}
        {hasRepo && <Tabs value={activeTab} onValueChange={v => setActiveTab(v as typeof activeTab)} className="shrink-0">
          {/* In fullscreen this strip is the topmost row, so it has to leave
              the traffic lights their corner. */}
          <div className={cn("flex items-center gap-2 border-b-0 pr-1.5 pt-1", fullscreen ? "pl-[84px]" : "px-[14px]")} data-tauri-drag-region={fullscreen ? "" : undefined}>
            <TabsList variant="underline" className="min-w-0 flex-1 justify-start gap-[2px] bg-transparent p-0">
              <TabsTab value="agent" className="h-[28px] px-2 text-[10.5px] text-muted-foreground rounded-none"><MessageSquareText size={14} aria-hidden="true" /> Agent</TabsTab>
              <TabsTab value="changes" className="h-[28px] px-2 text-[10.5px] text-muted-foreground rounded-none"><FileCode2 size={14} aria-hidden="true" /> Changes {workspace && workspace.dirtyFiles > 0 && <Badge variant="secondary" size="sm">{workspace.dirtyFiles}</Badge>}</TabsTab>
              <TabsTab value="code" className="h-[28px] px-2 text-[10.5px] text-muted-foreground rounded-none"><Code2 size={14} aria-hidden="true" /> Code</TabsTab>
              <TabsTab value="terminal" className="h-[28px] px-2 text-[10.5px] text-muted-foreground rounded-none"><TerminalSquare size={14} aria-hidden="true" /> Terminal</TabsTab>
            </TabsList>
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              className="shrink-0 text-muted-foreground"
              onClick={() => setFullscreen(value => !value)}
              aria-pressed={fullscreen}
              aria-label={fullscreen ? "Exit fullscreen" : "Fullscreen"}
              title={fullscreen ? "Exit fullscreen (⌥⌘F or Esc)" : "Fullscreen (⌥⌘F)"}
            >
              {fullscreen ? <Minimize2 size={13} aria-hidden="true" /> : <Maximize2 size={13} aria-hidden="true" />}
            </Button>
          </div>
        </Tabs>}
        <section className="flex-1 min-h-0 overflow-hidden flex relative">
          <div className="flex-1 min-w-0 flex flex-col relative">
            {(activeTab === "agent" || !hasRepo) && <>
              <div className="flex-1 min-h-0 relative">
                <AgentConversation
                  session={session}
                  onOpenSession={openSession}
                  events={sessionEvents}
                  forestEntries={forest?.entries}
                  activeLeafId={forest?.head?.activeEntryId}
                  repositoryDivergence={forest?.repositoryDivergence.status}
                  completion={forest?.completion}
                  onWaiveCompletion={waiveCompletion}
                  onRefreshBase={refreshWorkspaceBase}
                  pendingAdoptions={pendingAdoptions}
                  onResolveAdoption={resolveAdoption}
                  continuationFidelity={session?.continuationFidelity}
                  preview={false}
                  working={turnActive}
                  pendingMessages={pendingForSession}
                  onResolve={resolveApproval}
                />
              </div>
              <div className="pointer-events-none absolute bottom-0 left-0 right-0 h-16 bg-gradient-to-t from-background to-transparent sm:h-20" />
              <div className="relative z-10 flex-none safe-bottom">
                {hasRepo && workspace && workspace.dirtyFiles > 0 && <div className="mx-auto mb-2 flex max-w-2xl justify-center px-4 sm:px-6">
                  <div className="u-glass-soft inline-flex items-center gap-2 h-[30px] px-3.5 rounded-full text-muted-foreground text-xs">
                    <FileDiff size={12} aria-hidden="true" />
                    <span>{`${workspace.dirtyFiles} file${workspace.dirtyFiles === 1 ? "" : "s"}`}</span>
                    <em className="not-italic font-mono text-[11px]"><b className="text-success">+{workspace.additions}</b> <b className="text-destructive">−{workspace.deletions}</b></em>
                  </div>
                </div>}
                {isWorkerView ? <div className="mx-auto max-w-2xl px-4 sm:px-6"><div className="u-glass-soft flex items-center gap-2.5 rounded-2xl px-4 py-3 text-[12px] text-muted-foreground"><Bot size={14} className="shrink-0 text-muted-foreground" aria-hidden="true" /><span>This is a background worker. Watch it or resolve its approvals here — it takes direction from its orchestrator, so you can&apos;t message it directly.</span></div></div> : <div className="relative mx-auto max-w-2xl">
                  {!slashOpen && !mentionOpen && skillSuggestions.length > 0 && <div className="u-glass-popover absolute bottom-full left-4 right-4 z-20 mb-2 overflow-hidden rounded-2xl sm:left-6 sm:right-6"><div className="border-b border-border px-3 py-1.5 text-[9px] uppercase tracking-[0.12em] text-muted-foreground/70">Available skills for this task</div>{skillSuggestions.map(suggestion => <button key={suggestion.id} type="button" onMouseDown={event => { event.preventDefault(); setComposer(current => `/${suggestion.command} ${current}`); setSkillSuggestions([]); }} className="flex w-full items-start gap-3 border-b border-border px-3 py-2 text-left last:border-0 hover:bg-accent"><span className="mt-0.5 rounded border border-success/25 bg-success/10 px-1.5 py-0.5 text-[8.5px] uppercase text-success">installed</span><span className="min-w-0 flex-1"><b className="block truncate text-[11px] font-medium text-foreground">{suggestion.name}</b><small className="mt-0.5 block text-[9.5px] leading-4 text-muted-foreground">{suggestion.relevance} · {suggestion.source} · {suggestion.risk} risk · {suggestion.permissions.join(", ")}</small></span></button>)}</div>}
                  {mentionOpen && <div id="file-mention-listbox" role="listbox" className="u-glass-popover absolute left-4 right-4 sm:left-6 sm:right-6 bottom-full mb-2 z-20 rounded-2xl overflow-hidden flex flex-col max-h-[min(420px,55vh)]">
                    <div className="shrink-0 px-3 py-1.5 text-[9px] uppercase tracking-[0.12em] text-muted-foreground/70 border-b border-border flex items-center gap-2">
                      <span>Reference a file</span>
                      <span className="normal-case tracking-normal text-muted-foreground/50">{fileMatches.length}</span>
                    </div>
                    <div ref={mentionListRef} className="min-h-0 flex-1 overflow-y-auto overscroll-contain" onWheel={e => e.stopPropagation()}>
                      {fileMatches.map((file, index) => { const dir = file.includes("/") ? file.slice(0, file.lastIndexOf("/") + 1) : ""; const base = file.slice(dir.length); return <button id={`file-mention-option-${index}`} role="option" aria-selected={index === mentionIndex} key={file} type="button" data-mention-index={index} onMouseEnter={() => setMentionIndex(index)} onMouseDown={e => { e.preventDefault(); applyFileMention(file); }} className={`min-h-11 w-full flex items-center gap-2 px-3 py-2 text-left transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-ring ${index === mentionIndex ? "bg-accent" : "hover:bg-accent"}`}>
                        <FileText size={13} className="shrink-0 text-muted-foreground" aria-hidden="true" />
                        <span className="flex-1 min-w-0 text-[12px] whitespace-nowrap overflow-hidden text-ellipsis"><span className="text-muted-foreground">{dir}</span><span className="text-foreground">{base}</span></span>
                      </button>; })}
                    </div>
                  </div>}
                  {slashOpen && <div className="u-glass-popover absolute left-4 right-4 sm:left-6 sm:right-6 bottom-full mb-2 z-20 rounded-2xl overflow-hidden flex flex-col max-h-[min(420px,55vh)]">
                    <div className="shrink-0 px-3 py-1.5 text-[9px] uppercase tracking-[0.12em] text-muted-foreground/70 border-b border-border flex items-center gap-2">
                      <span>Commands & skills</span>
                      <span className="normal-case tracking-normal text-muted-foreground/50">{slashMatches.length}</span>
                    </div>
                    <div ref={slashListRef} className="min-h-0 flex-1 overflow-y-auto overscroll-contain" onWheel={e => e.stopPropagation()}>
                      {slashMatches.map((command, index) => <button key={`${command.harness}:${command.kind}:${command.name}`} type="button" data-slash-index={index} onMouseEnter={() => setSlashIndex(index)} onMouseDown={e => { e.preventDefault(); void applySlash(command); }} className={`w-full flex items-center gap-2 px-3 py-2 text-left transition-colors ${index === slashIndex ? "bg-accent" : "hover:bg-accent"}`}>
                        <span className="font-mono text-[12px] text-foreground whitespace-nowrap">/{command.name}</span>
                        <span className="flex-1 min-w-0 text-[11px] text-muted-foreground whitespace-nowrap overflow-hidden text-ellipsis">{command.description}</span>
                        <span className="shrink-0 text-[8.5px] uppercase tracking-[0.06em] text-muted-foreground border border-border rounded px-1 py-[1px]">{harnessLabel(command.harness)}</span>
                      </button>)}
                    </div>
                  </div>}
                  <ComposerPill
                    layout="dock"
                    value={composer}
                    onChange={value => { setComposer(value); setSlashDismissed(false); setSlashIndex(0); setMentionDismissed(false); setMentionIndex(0); }}
                    onSubmit={() => void sendPrompt()}
                    onKeyDown={onComposerKeyDown}
                    autocomplete={mentionOpen ? {
                      controls: "file-mention-listbox",
                      activeDescendant: `file-mention-option-${mentionIndex}`,
                    } : undefined}
                    placeholder={isDirectChat ? "Ask Bridge…" : sessionConnected ? "Message…" : "Message…  (starts the agent)"}
                    disabled={!session}
                    working={!!session?.activeTurnId}
                    onStop={session ? () => void bridgeApi.interruptTurn(session.id) : undefined}
                    onPlusClick={() => { setComposer(""); setSlashDismissed(false); }}
                    trailing={session.kind === "direct" || session.kind === "orchestrator"
                      ? <ChatModelControl adapters={adapters} harness={session.harness} model={session.model ?? null} disabled={busy || turnActive} disabledReason={turnActive ? "Wait for the current response before switching models" : undefined} onChange={(harness, model) => void changeChatModel(harness, model)} compact roleLabel={session.kind === "orchestrator" ? "Orchestrator" : "Chat"} />
                      : <span className="inline-flex items-center gap-1 h-8 px-2.5 text-foreground/75 text-[13px] rounded-full">{harnessLabel(session.harness)}</span>}
                  />
                </div>}
              </div>
            </>}
            {hasRepo && workspace && visitedTabs.has("changes") && <div className={cn("absolute inset-0", activeTab !== "changes" && "hidden")}><ChangesPanel workspace={workspace}/></div>}
            {hasRepo && workspace && visitedTabs.has("code") && <div className={cn("absolute inset-0", activeTab !== "code" && "hidden")}><Suspense fallback={<PanelLoading label="Opening editor…"/>}><CodePanel workspaceId={workspace.id} visible={activeTab === "code"} onSaved={() => void refreshWorkspaceStats(workspace.id)}/></Suspense></div>}
            {hasRepo && workspace && activeTab === "terminal" && <div className="absolute inset-0"><Suspense fallback={<PanelLoading label="Opening terminal…"/>}><TerminalPane workspaceId={workspace.id}/></Suspense></div>}
          </div>
          {browserOpen && <BrowserSurface onClose={() => setBrowserOpen(false)} onError={setError} />}
        </section>
      </> : <Welcome
        adapters={adapters}
        modelSetup={modelSetup}
        canStartChat={adaptersReady}
        busy={busy}
        onStartChat={text => void openNewChat(text)}
        onNewWorkspace={() => { setTitle(""); setModal("workspace"); }}
      />}
    </main>
    {error && (() => {
      const described = describeError(error, {
        provider: session ? harnessLabel(session.harness) : undefined,
        snapshot: session ? usageByProvider[session.harness as UsageProvider] : undefined,
      });
      return (
        <Alert variant={described.kind === "usage-limit" ? "warning" : "error"} className="u-overlay fixed right-3 bottom-3 z-40 max-w-[min(32rem,calc(100vw-1.5rem))] rounded-xl sm:right-[18px] sm:bottom-[18px]">
          <AlertTitle>{described.title}</AlertTitle>
          <AlertDescription>{described.message}</AlertDescription>
          <AlertAction>
            <Button type="button" size="icon-sm" variant="ghost" aria-label="Dismiss error" onClick={() => setError(undefined)}><X size={14} aria-hidden="true" /></Button>
          </AlertAction>
        </Alert>
      );
    })()}

    <WorkspaceCreateDialog
      open={modal === "workspace"}
      title={title}
      busy={busy}
      onTitleChange={setTitle}
      onClose={() => setModal(null)}
      onSubmit={() => void submitNewWorkspace()}
    />
    <OrchestratorCreateDialog
      open={modal === "orchestrator"}
      workspaceTitle={state.workspaces.find(item => item.id === pendingWorkspaceId)?.title ?? "workspace"}
      canCreateWorktree={!!state.workspaces.find(item => item.id === pendingWorkspaceId)?.projectId}
      busy={busy}
      onCreateWorktree={() => void newWorkspaceSession(true)}
      onUseCurrentFolder={() => void newWorkspaceSession(false)}
      onClose={() => void newWorkspaceSession(false)}
    />
    <RouterSettingsDialog open={modal === "router"} workspaceId={workspace?.id} adapters={adapters} databasePath={health.database} onModelSetupChange={setModelSetup} onClose={() => setModal(null)} onError={setError} />
  </div>;
}

function PanelLoading({ label }: { label: string }) {
  return <div role="status" className="absolute inset-0 grid place-items-center text-xs text-muted-foreground">{label}</div>;
}


function modelDisplayName(adapters: import("./types").AdapterDescriptor[], harness: Harness, model?: string | null): string {
  const adapter = adapters.find(item => item.id === harness);
  return adapter?.models.find(option => option.id === model)?.label ?? model ?? "Automatic";
}

export function ChatModelControl({ adapters, harness, model, disabled, disabledReason, onChange, compact, roleLabel = "Chat" }: { adapters: import("./types").AdapterDescriptor[]; harness: Harness; model: string | null; disabled?: boolean; disabledReason?: string; onChange: (harness: Harness, model: string | null) => void; compact?: boolean; roleLabel?: string }) {
  const [open, setOpen] = useState(false);
  const chatAdapters = adapters.filter(adapter => ["codex", "claude", "opencode"].includes(adapter.id));
  const current = chatAdapters.find(adapter => adapter.id === harness);
  const currentModel = current?.models.find(option => option.id === model) ?? current?.models.find(option => option.defaultForTier) ?? current?.models[0];
  const modelLabel = currentModel?.label ?? model ?? "Default";
  const compactLabel = `${harnessLabel(harness)} · ${modelLabel}`;
  return <div className="relative">
    <button type="button" disabled={disabled} onClick={() => setOpen(value => !value)} className={`flex max-w-[220px] items-center gap-1 rounded-full transition-colors disabled:opacity-45 ${compact ? "h-8 px-2 text-xs text-muted-foreground hover:bg-accent" : "h-[28px] px-2 text-[11.5px] text-foreground/90 hover:bg-accent"}`} title={disabled ? disabledReason ?? "Model selection is temporarily unavailable" : `Choose ${roleLabel.toLowerCase()} model`} aria-label={`${roleLabel} model: ${harnessLabel(harness)} ${modelLabel}`}>
      <span className="whitespace-nowrap overflow-hidden text-ellipsis">{compactLabel}</span>
      <ChevronDown size={compact ? 14 : 12} className={`shrink-0 text-muted-foreground/55 transition-transform ${open ? "rotate-180" : ""}`} aria-hidden="true" />
    </button>
    {open && <>
      <div className="fixed inset-0 z-30" onClick={() => setOpen(false)} />
      <div className="u-glass-popover absolute left-0 bottom-full mb-2 z-40 w-[280px] py-1.5 rounded-2xl max-h-[340px] overflow-y-auto">
        <div className="border-b border-border/60 px-3 pb-2 pt-1">
          <p className="text-[10px] font-semibold uppercase tracking-[0.1em] text-muted-foreground/65">{roleLabel} runtime</p>
          <p className="mt-1 text-[10px] leading-4 text-muted-foreground/55">Switching starts a fresh provider session. The chat stays visible, but provider reasoning state resets.</p>
        </div>
        {chatAdapters.map((adapter, index) => <div key={adapter.id} className={index > 0 ? "mt-1 pt-1 border-t border-border/60" : ""}>
          <div className="px-3 py-1.5 text-[9px] font-semibold tracking-[0.12em] uppercase text-muted-foreground/50 flex items-center gap-2">
            <span>{adapter.label}</span>
            {!adapter.available && <span className="normal-case tracking-normal font-normal text-muted-foreground/40">unavailable</span>}
          </div>
          {(adapter.models.length ? adapter.models : [{ id: "", label: "Default", tier: "fast" as const, defaultForTier: true }]).map(option => {
            const selected = adapter.id === harness && (option.id ? option.id === model : !model);
            return <button key={`${adapter.id}:${option.id || "default"}`} type="button" disabled={!adapter.available} onClick={() => { onChange(adapter.id as Harness, option.id || null); setOpen(false); }} className={`w-full flex items-center gap-2 px-3 py-2 text-left transition-colors disabled:opacity-40 ${selected ? "bg-foreground/[0.08]" : "hover:bg-foreground/[0.05]"}`}>
              <span className="flex-1 min-w-0 text-[12.5px] text-foreground whitespace-nowrap overflow-hidden text-ellipsis">{option.label}</span>
              <span className="text-[9.5px] uppercase tracking-[0.06em] text-muted-foreground/45">{option.tier}</span>
              {selected && <Check size={13} className="text-foreground/80" aria-hidden="true" />}
            </button>;
          })}
        </div>)}
      </div>
    </>}
  </div>;
}

function EnvPanel({ workspace, project, session, sessions, forest, onChanges, onSelectLeaf, onCompact }: { workspace: Workspace; project?: Project; session?: Session; sessions: Session[]; forest?: SessionForestSnapshot; onChanges: () => void; onSelectLeaf: (entryId: string) => void; onCompact: () => void }) {
  const workers = sessions.filter(s => s.parentSessionId && s.parentSessionId === session?.id);
  const doneWorkers = workers.filter(s => s.status === "stopped" || s.status === "ready").length;
  const budget = turnBudget(forest, session?.activeTurnId);
  const restoration = restorationPresentation(session?.restorationMode ?? "fresh");
  return <aside className="hidden">
  </aside>;
}

const IMPORTANCE_RANK: Record<RiskTier, number> = { high: 0, medium: 1, low: 2 };
const IMPORTANCE_BADGE: Record<RiskTier, { label: string; variant: "error" | "warning" | "outline" }> = {
  high: { label: "High", variant: "error" },
  medium: { label: "Medium", variant: "warning" },
  low: { label: "Low", variant: "outline" },
};

function FileDiffView({ patch, binary, path }: { patch: string; binary: boolean; path: string }) {
  if (binary) return <div className="px-3.5 py-4 text-[11.5px] text-muted-foreground">Binary file — no diff to show.</div>;
  if (!patch.trim()) return <div className="px-3.5 py-4 text-[11.5px] text-muted-foreground">No diff content.</div>;
  return <PatchView patch={patch} path={path} />;
}

/** Proportional add/delete bar. Silent when a file has no line changes. */
function DiffStatBar({ additions, deletions, className }: { additions: number; deletions: number; className?: string }) {
  const total = additions + deletions;
  if (!total) return null;
  return <span className={cn("flex h-1 w-10 shrink-0 overflow-hidden rounded-full bg-muted", className)} aria-hidden="true">
    <span className="bg-success" style={{ width: `${(additions / total) * 100}%` }} />
    <span className="bg-destructive" style={{ width: `${(deletions / total) * 100}%` }} />
  </span>;
}

function ChangeFileRow({ file, viewed, expanded, workspaceId, onToggleViewed, onToggleExpanded, onSaved }: {
  file: WorkspaceFileChange;
  viewed: boolean;
  expanded: boolean;
  workspaceId: string;
  onToggleViewed: () => void;
  onToggleExpanded: () => void;
  onSaved: () => void;
}) {
  // Reading the diff and fixing what you just read are the same motion, so
  // the row carries both. Diff stays the default: review first.
  const [mode, setMode] = useState<"diff" | "edit">("diff");
  // "Low" is the default state, so labelling it adds noise to every row. Only
  // a file that actually wants attention gets a badge.
  const badge = file.importance === "low" ? undefined : IMPORTANCE_BADGE[file.importance];
  const cut = file.path.lastIndexOf("/") + 1;
  return <div className={cn("transition-opacity", viewed && !expanded && "opacity-55")}>
    <div className="flex items-center gap-2 px-2 py-1">
      <button type="button" onClick={onToggleExpanded} aria-expanded={expanded} className="flex min-w-0 flex-1 items-center gap-1.5 rounded-md px-1.5 py-1.5 text-left hover:bg-accent">
        <ChevronDown size={13} className={cn("shrink-0 text-muted-foreground/60 transition-transform", !expanded && "-rotate-90")} aria-hidden="true" />
        <span className="truncate font-mono text-[12px]">
          {cut > 0 && <span className="text-muted-foreground/70">{file.path.slice(0, cut)}</span>}
          <span className="text-foreground">{file.path.slice(cut)}</span>
        </span>
      </button>
      {badge && <Badge variant={badge.variant} size="sm" className="hidden shrink-0 sm:inline-flex">{badge.label}</Badge>}
      <span className="hidden shrink-0 items-center gap-1.5 font-mono text-[10.5px] tabular-nums sm:flex">
        <span className="text-success">+{file.additions}</span>
        <span className="text-destructive">−{file.deletions}</span>
        <DiffStatBar additions={file.additions} deletions={file.deletions} />
      </span>
      <button
        type="button"
        onClick={onToggleViewed}
        aria-pressed={viewed}
        title={viewed ? "Mark as not viewed" : "Mark as viewed"}
        aria-label={viewed ? `Mark ${file.path} as not viewed` : `Mark ${file.path} as viewed`}
        className={cn(
          "grid h-6 w-6 shrink-0 place-items-center rounded-md border transition-colors",
          viewed ? "border-success/40 bg-success/10 text-success" : "border-border text-muted-foreground/60 hover:bg-accent hover:text-foreground",
        )}
      >
        <Check size={12} aria-hidden="true" />
      </button>
    </div>
    {expanded && <div className="border-t border-border bg-code">
      {!file.binary && <div className="flex items-center gap-1 border-b border-border px-2 py-1">
        {(["diff", "edit"] as const).map(option => <button
          key={option}
          type="button"
          onClick={() => setMode(option)}
          aria-pressed={mode === option}
          className={cn(
            "h-[20px] rounded-[5px] px-2 text-[10.5px] capitalize transition-colors",
            mode === option ? "bg-accent text-foreground" : "text-muted-foreground hover:text-foreground",
          )}
        >{option}</button>)}
      </div>}
      {mode === "edit" && !file.binary
        ? <Suspense fallback={<div className="px-3.5 py-4 text-[11.5px] text-muted-foreground">Opening editor…</div>}><InlineFileEditor workspaceId={workspaceId} path={file.path} onSaved={onSaved} /></Suspense>
        : <FileDiffView patch={file.patch} binary={file.binary} path={file.path} />}
    </div>}
  </div>;
}

function ChangesPanel({ workspace }: { workspace: Workspace }) {
  const [changes, setChanges] = useState<WorkspaceChangesResult>();
  const [loadError, setLoadError] = useState<string>();
  const [expandedPaths, setExpandedPaths] = useState<Set<string>>(new Set());
  const [viewedPaths, setViewedPaths] = useState<Set<string>>(new Set());
  const [showLowSignal, setShowLowSignal] = useState(false);

  /** Re-read the changeset in place. Saving from an expanded row calls this,
   *  so the diff under the editor catches up without collapsing the review. */
  const reloadChanges = useCallback(async () => {
    try {
      setChanges(await bridgeApi.workspaceChanges(workspace.id));
      setLoadError(undefined);
    } catch (error) {
      setLoadError(errorMessage(error));
    }
  }, [workspace.id]);

  useEffect(() => {
    setChanges(undefined); setLoadError(undefined); setExpandedPaths(new Set()); setShowLowSignal(false);
    void reloadChanges();
  }, [reloadChanges]);

  const sortedFiles = useMemo(() => [...(changes?.files ?? [])].sort((a, b) =>
    IMPORTANCE_RANK[a.importance] - IMPORTANCE_RANK[b.importance] || a.path.localeCompare(b.path)
  ), [changes]);
  const visibleFiles = sortedFiles.filter(file => showLowSignal || !file.lowSignal);
  const lowSignalCount = sortedFiles.length - sortedFiles.filter(file => !file.lowSignal).length;

  const toggle = (setter: typeof setExpandedPaths, path: string) => setter(previous => {
    const next = new Set(previous);
    if (next.has(path)) next.delete(path); else next.add(path);
    return next;
  });

  if (loadError) return <div className="p-[38px_44px] max-w-[780px]">
    <div className="text-muted-foreground/65 text-[10.5px] font-semibold tracking-[0.1em]">CHANGES</div>
    <p className="mt-2.5 text-destructive text-[13px]">{loadError}</p>
  </div>;

  if (!changes) return <div className="p-[38px_44px] max-w-[780px]">
    <div className="text-muted-foreground/65 text-[10.5px] font-semibold tracking-[0.1em]">CHANGES</div>
    <p className="mt-2.5 text-muted-foreground text-[13px]">Loading changes…</p>
  </div>;

  const totalAdditions = changes.files.reduce((sum, file) => sum + file.additions, 0);
  const totalDeletions = changes.files.reduce((sum, file) => sum + file.deletions, 0);
  const viewedCount = sortedFiles.filter(file => viewedPaths.has(file.path)).length;

  return <div className="mx-auto h-full max-w-3xl overflow-y-auto px-4 py-6 sm:px-6 sm:py-8">
    <div className="text-[10.5px] font-semibold tracking-[0.1em] text-muted-foreground/65">CHANGES</div>
    <h2 className="my-2 font-heading text-[20px] tracking-[-0.015em] text-foreground">{changes.files.length ? `${changes.files.length} file${changes.files.length === 1 ? "" : "s"} changed` : "Workspace is clean"}</h2>
    {changes.files.length === 0
      ? <p className="max-w-[560px] text-[13px] leading-relaxed text-muted-foreground">No uncommitted changes against HEAD.</p>
      : <>
        <div className="mb-3 flex flex-wrap items-center gap-x-3 gap-y-1.5 font-mono text-[11.5px]">
          <span className="text-success">+{totalAdditions}</span>
          <span className="text-destructive">−{totalDeletions}</span>
          <DiffStatBar additions={totalAdditions} deletions={totalDeletions} className="w-24" />
          <span className="ml-auto text-muted-foreground">{viewedCount}/{sortedFiles.length} viewed</span>
        </div>
        {/* One list with dividers, not a stack of floating cards — a review
            reads down a column of paths, and cards fight that. */}
        <div className="divide-y divide-border overflow-hidden rounded-lg border border-border bg-card">
          {visibleFiles.map(file => <ChangeFileRow
            key={file.path}
            file={file}
            workspaceId={workspace.id}
            onSaved={() => void reloadChanges()}
            viewed={viewedPaths.has(file.path)}
            expanded={expandedPaths.has(file.path)}
            onToggleViewed={() => toggle(setViewedPaths, file.path)}
            onToggleExpanded={() => toggle(setExpandedPaths, file.path)}
          />)}
        </div>
        {lowSignalCount > 0 && <button type="button" onClick={() => setShowLowSignal(value => !value)} className="mt-2.5 px-1.5 py-1 text-left text-[11.5px] text-muted-foreground transition-colors hover:text-foreground">
          {showLowSignal ? "Hide low-signal files" : `${lowSignalCount} low-signal file${lowSignalCount === 1 ? "" : "s"} hidden — show`}
        </button>}
      </>}
  </div>;
}
function EventPanel({ state, workspace }: { state: BridgeState; workspace: Workspace }) { const events = state.events.filter(e => e.entityId === workspace.id || state.sessions.some(s => s.workspaceId === workspace.id && s.id === e.entityId)); return <div className="max-w-[720px] px-8 py-[22px]">{events.length ? events.map(e => <article key={e.id} className="flex gap-3 py-[13px] border-b border-border text-muted-foreground"><CircleDot size={14} aria-hidden="true" /><div><b className="text-foreground text-[11px] font-medium tracking-[0.02em] capitalize">{e.kind.replaceAll(".", " ")}</b><p className="text-[12.5px] my-1 text-foreground">{e.body}</p><small className="font-mono text-[10.5px] text-muted-foreground/65">{new Date(e.createdAt).toLocaleTimeString([], {hour:"2-digit",minute:"2-digit"})}</small></div></article>) : <div className="text-muted-foreground text-[12.5px] p-7">No events for this workspace yet.</div>}</div>; }
function WelcomeModelBadge({ adapters, modelSetup }: { adapters: import("./types").AdapterDescriptor[]; modelSetup: ModelSetupState }) {
  const profile = resolveProfileOption("standard_orchestrator", modelSetup, adapters);
  const preferred = profile?.adapter ?? adapters.find(adapter => adapter.available) ?? adapters[0];
  const model = profile?.model ?? preferred?.models.find(option => option.id === preferred.defaultModel) ?? preferred?.models.find(option => option.defaultForTier) ?? preferred?.models[0];
  const tierLabel = model?.tier === "strong" ? "High" : model?.tier === "standard" ? "Balanced" : "Fast";
  return <span className="inline-flex items-center gap-1 rounded-full px-2.5 py-1.5 text-xs text-muted-foreground">{tierLabel}<ChevronDown size={14} className="text-muted-foreground/70" aria-hidden="true" /></span>;
}

function Welcome({ adapters, modelSetup, busy, canStartChat, onStartChat, onNewWorkspace }: { adapters: import("./types").AdapterDescriptor[]; modelSetup: ModelSetupState; busy: boolean; canStartChat: boolean; onStartChat: (text?: string) => void; onNewWorkspace: () => void }) {
  const greeting = useMemo(() => pickGreeting("welcome"), []);
  const [draft, setDraft] = useState("");
  const submit = () => {
    const text = draft.trim();
    if (text) onStartChat(text);
    else onStartChat();
    setDraft("");
  };
  return <div className="flex flex-1 flex-col items-center justify-center px-4 text-center animate-page-enter">
    <h1 className="mb-8 max-w-xl font-display text-[1.9rem] font-medium leading-[1.15] tracking-[-0.025em] text-foreground sm:mb-10 sm:text-[2.4rem]">{greeting.headline}</h1>
    <ComposerPill
      layout="hero"
      value={draft}
      onChange={setDraft}
      onSubmit={submit}
      onKeyDown={e => { if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) { e.preventDefault(); submit(); } }}
      placeholder={canStartChat ? "Ask Bridge…" : "Install or sign in to a model adapter…"}
      disabled={busy || !canStartChat}
      onPlusClick={onNewWorkspace}
      trailing={<WelcomeModelBadge adapters={adapters} modelSetup={modelSetup} />}
    />
    <p className="mt-6 max-w-md text-[13px] leading-relaxed text-muted-foreground">{greeting.hint}</p>
  </div>;
}
function CommandPalette({ workspaces, onChoose }: { workspaces: Workspace[]; onChoose: (id:string)=>void }) { return <><InputGroup className="border-b border-border rounded-none border-x-0 border-t-0 shadow-none"><InputGroupInput autoFocus placeholder="Search workspaces and actions…" /><InputGroupAddon><Search size={17} aria-hidden="true" /></InputGroupAddon></InputGroup><div className="p-[9px]"><label className="block p-[5px_9px_7px] text-muted-foreground/65 text-[10px] font-semibold tracking-[0.09em]">WORKSPACES</label>{workspaces.map(w => <Button type="button" key={w.id} variant="ghost" className="w-full h-[44px] rounded-md justify-start px-2.5" onClick={() => onChoose(w.id)}><StatusDot status={w.status}/><span className="flex flex-col gap-[3px] flex-1 text-left"><b className="text-[12.5px] font-medium">{w.title}</b><small className="text-[10.5px] text-muted-foreground">{w.city} · {w.branch}</small></span><Kbd className="font-mono text-muted-foreground/65 border border-border rounded px-1 py-[1px] text-[10px]">↵</Kbd></Button>)}</div><div className="h-[32px] border-t border-border flex items-center gap-[14px] px-[13px] text-muted-foreground/65 text-[10.5px]"><span>↑↓ navigate</span><span>esc close</span></div></>; }
