import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { appendFileMention, applyFileMention as insertFileMention, fileMentionQuery } from "./fileMentions";
import { harnessShortcutQuery, parseHarnessShortcut } from "./harnessShortcut";
import { Activity, Archive, Bot, Check, ChevronDown, CircleDot, Clock3, Code2, FileCode2, FileDiff, FileText, GitCommitHorizontal, GitPullRequest, Inbox, LoaderCircle, MessageSquareText, Play, Plus, Search, TerminalSquare, X } from "lucide-react";
import { bridgeApi } from "./api";
import { openExternalUrl } from "./externalLinks";
import { appendAgentEventBatch } from "./agentEvents";
import type { AgentEvent, ApprovalDecision, BridgeState, CapabilitySuggestion, Harness, Health, ModelSetupState, PermissionPolicy, Project, RiskTier, Session, SessionForestSnapshot, SessionStatus, SkillProvider, WorkerRepositoryBinding, Workspace, WorkspaceChangesResult, WorkspaceFileChange } from "./types";
import { AgentConversation } from "./components/AgentConversation";
import { BridgeSidebar } from "./components/BridgeSidebar";
import { HealthWarnings } from "./components/HealthWarnings";
import { ComposerContextStrip } from "./components/ComposerContextStrip";
import { ProjectsScreen } from "./components/ProjectsScreen";
import type { SuggestCompletionResult, SuggestionSettingsSnapshot, WorkBoard, WorkFactAction, WorkTask } from "./protocol/generated/protocol";
import type { WorkActionOutcome } from "./components/WorkView";
import { taskRoute, type TaskAction } from "./components/workTasks";
import { isHiddenSession } from "./components/sidebarChats";
import { SessionToolbar } from "./components/SessionToolbar";
import { SessionRecallSearch } from "./components/SessionRecallSearch";
import { AppTitleBar } from "./components/AppTitleBar";
import { MissionControl } from "./components/MissionControl";
import { BypassBadge } from "./components/BypassBadge";
import type { Section as SettingsSection } from "./components/SettingsScreen";
import { SteerComposer, WorkerDetail } from "./components/WorkerDetail";
import { ComposerPill } from "./components/ComposerPill";
import { activeTurnAction, queuedFollowUps } from "./sessionInput";
import { BrowserSurface } from "./components/BrowserSurface";
import { PatchView } from "./components/DiffView";
import { WorkspaceCreateDialog } from "./components/WorkspaceCreateDialog";
import { OrchestratorCreateDialog } from "./components/OrchestratorCreateDialog";
import { RouterSettingsDialog } from "./components/RouterSettingsDialog";
import { MemoryDialog, rememberAction } from "./components/MemoryDialog";
import { MemoryUsedChip } from "./components/MemoryUsedChip";
import { ModelSetupWizard } from "./components/ModelSetupWizard";
import { UsageWidget } from "./components/UsageWidget";
import { formatElapsed, harnessLabel, slashOwnershipBadge, tierRuntimeLabel } from "./utils";
import { scheduleSuggestion } from "./suggestionTypeahead";
import { projectSessionConversation, reduceConversation } from "./conversation";
import { resolveProfileOption, shouldRequireModelSetup } from "./modelProfiles";
import { pickGreeting } from "./greetings";
import { useThemePreference } from "./theme";
import { recordPlace, type AppPlace, type AppView } from "./navigationHistory";
import { readLastWorkspaceId, resolveNewChatWorkspaceId, writeLastWorkspaceId } from "./lastWorkspace";
import { FLUSH_WINDOW_EVENT, isFlushWindowDocument, notifyLayoutFullscreen, setLayoutFullscreenDocument } from "./windowChrome";
import { cn } from "@/lib/utils";
import { buildCacheDiagnostics, buildUsageHistory, clampPercent, extractUsageSnapshot, type UsageProvider, type UsageRateSample, type UsageSnapshot } from "./usage";
import { describeError, errorMessage } from "./errors";
import { mergeForestSnapshot } from "./forest";
import { queueExplanation, restorationPresentation, turnBudget } from "./observability";
import { startSerialPoll } from "./polling";
import { Alert, AlertAction, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { InputGroup, InputGroupAddon, InputGroupInput } from "@/components/ui/input-group";
import { Kbd } from "@/components/ui/kbd";

const AutomationsPanel = lazy(() => import("./components/AutomationsPanel").then(module => ({ default: module.AutomationsPanel })));
const MarketplaceScreen = lazy(() => import("./components/MarketplaceScreen").then(module => ({ default: module.MarketplaceScreen })));
const SettingsScreen = lazy(() => import("./components/SettingsScreen").then(module => ({ default: module.SettingsScreen })));
const WorkView = lazy(() => import("./components/WorkView").then(module => ({ default: module.WorkView })));
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
  const [view, setView] = useState<AppView>("workspace");
  const [navPlaces, setNavPlaces] = useState<{ stack: AppPlace[]; index: number }>({
    stack: [{ view: "workspace", sessionId: null, paradigm: "single" }],
    index: 0,
  });
  const skipNavRecord = useRef(false);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(() => {
    try { return localStorage.getItem("bridge.sidebar.collapsed") === "1"; }
    catch { return false; }
  });
  // The Work board. Held here rather than inside WorkView so the rail can show a
  // count while a conversation is on screen. Returning to the board does re-read, on
  // purpose: a fact you just acted on may be gone, and showing it again would be
  // worse than a second SQLite read.
  const [workBoard, setWorkBoard] = useState<WorkBoard>();
  const [workError, setWorkError] = useState<string>();
  // A re-read that failed while a board is on screen. Separate from `workError`
  // because it must not replace the board — see `readWorkBoard`.
  const [workRefreshError, setWorkRefreshError] = useState<string>();
  // Which read is the newest. Opening, refreshing, and both mutating actions all
  // read, so a slow earlier call can land after a fast later one; without this it
  // would write its own result over the newer board.
  const workReadGeneration = useRef(0);
  // Mirrors `workBoard` so the catch below can tell "nothing to show" from "a refresh
  // failed" without depending on the state it is setting.
  const workBoardRef = useRef<WorkBoard>();
  const [navOpen, setNavOpen] = useState(false);
  // Two ways to look at the workspace: the classic single-session view, or the
  // Mission Control grid where every live agent is its own window at once.
  const [paradigm, setParadigm] = useState<"single" | "grid">("single");
  const [activeTab, setActiveTab] = useState<"agent" | "changes" | "code" | "events" | "terminal">("agent");
  // Fullscreen only squares the native frame. The sidebar and canvas keep the
  // same side-by-side geometry as windowed mode; native fullscreen and zoom
  // arrive as `data-flush-window` from the shell.
  const [fullscreen, setFullscreen] = useState(false);
  const [flushWindow, setFlushWindow] = useState(isFlushWindowDocument);
  /// The worker whose full activity feed is open over the chat. Owned here, not
  /// in the conversation, because the overlay covers the whole session pane and
  /// has to survive the transcript re-rendering underneath it.
  const [expandedWorkerId, setExpandedWorkerId] = useState<string>();
  // Tabs mount on first visit and then stay mounted. Unmounting the Changes
  // and Code panels on every tab switch would throw away open files, expanded
  // diffs, and — now that both tabs can edit — unsaved text.
  const [visitedTabs, setVisitedTabs] = useState<Set<string>>(() => new Set(["agent"]));
  useEffect(() => { setVisitedTabs(previous => previous.has(activeTab) ? previous : new Set(previous).add(activeTab)); }, [activeTab]);
  const [modal, setModal] = useState<"workspace" | "orchestrator" | "router" | "memory" | null>(null);
  // A too-long "Remember this" lands here so the dialog opens pre-filled for
  // trimming; it is never saved on the user's behalf.
  const [memoryDraft, setMemoryDraft] = useState<string | null>(null);
  const [packetAudit, setPacketAudit] = useState<import("./types").MemoryPacketAudit | null>(null);
  const [memoryDisclosureOpen, setMemoryDisclosureOpen] = useState(false);
  const [pendingWorkspaceId, setPendingWorkspaceId] = useState<string>();
  const worktreeBySessionRef = useRef(new Map<string, boolean>());
  const [title, setTitle] = useState("");
  const [composer, setComposer] = useState("");
  const [slashCommands, setSlashCommands] = useState<import("./types").SlashCommand[]>([]);
  const [slashIndex, setSlashIndex] = useState(0);
  const [slashDismissed, setSlashDismissed] = useState(false);
  const [workspaceFiles, setWorkspaceFiles] = useState<string[]>([]);
  const composerRef = useRef<HTMLTextAreaElement | null>(null);
  const [mentionIndex, setMentionIndex] = useState(0);
  const [mentionDismissed, setMentionDismissed] = useState(false);
  const [harnessShortcutIndex, setHarnessShortcutIndex] = useState(0);
  const [harnessShortcutDismissed, setHarnessShortcutDismissed] = useState(false);
  const [skillSuggestions, setSkillSuggestions] = useState<CapabilitySuggestion[]>([]);
  const [busy, setBusy] = useState(false);
  const [browserOpen, setBrowserOpen] = useState(false);
  const [recallOpen, setRecallOpen] = useState(false);
  const [highlightEntryId, setHighlightEntryId] = useState<string | null>(null);
  const [error, setError] = useState<string>();
  const [forest, setForest] = useState<SessionForestSnapshot>();
  // Completion blocks while a child's changes live only in its own worktree, so
  // the user must be able to see and resolve that here — otherwise the session
  // waits forever with no visible cause.
  const [pendingAdoptions, setPendingAdoptions] = useState<WorkerRepositoryBinding[]>([]);
  const [pending, setPending] = useState<{ key: string; sessionId: string; text: string; delivery?: "steered" | "queued" }[]>([]);
  // The composer's inline typeahead. Loaded once and kept fresh by Settings'
  // own save path (`onSuggestionSettingsChange`) — off by default, so no
  // request fires until the user opts in.
  const [suggestionSettings, setSuggestionSettings] = useState<SuggestionSettingsSnapshot>();
  const [draftSuggestion, setDraftSuggestion] = useState<SuggestCompletionResult>();
  const suggestionGeneration = useRef(0);
  // Shown once per fallback episode, not on every debounce firing while the
  // configured model stays in cooldown.
  const [fallbackNotice, setFallbackNotice] = useState<string>();
  const fallbackNoticeShownRef = useRef(false);
  const [usageByProvider, setUsageByProvider] = useState<Partial<Record<UsageProvider, UsageSnapshot>>>({});
  const [usageSamples, setUsageSamples] = useState<Partial<Record<UsageProvider, UsageRateSample[]>>>({});
  const startedRef = useRef<Set<string>>(new Set());
  const pendingWelcomeMessageRef = useRef<string | null>(null);
  const forestKeyRef = useRef("");
  const agentEventQueueRef = useRef<AgentEvent[]>([]);
  const agentEventTimerRef = useRef<number | undefined>(undefined);
  const browserSessionRef = useRef<string>();

  const reload = useCallback(async () => {
    setState(await bridgeApi.state());
    // Re-read with the state it was published alongside: `save_permission_policy`
    // publishes StateChanged precisely so the badge repaints, and another window
    // flipping the switch has to reach this one too.
    setPermissionPolicy((await bridgeApi.configState()).permissionPolicy);
  }, []);
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
  useEffect(() => { setNavOpen(false); setRecallOpen(false); setHighlightEntryId(null); }, [view, selectedSessionId]);

  useEffect(() => {
    const place: AppPlace = { view, sessionId: selectedSessionId ?? null, paradigm };
    if (skipNavRecord.current) {
      skipNavRecord.current = false;
      return;
    }
    setNavPlaces(current => recordPlace(current.stack, current.index, place));
  }, [view, selectedSessionId, paradigm]);

  const applyPlace = useCallback((place: AppPlace) => {
    skipNavRecord.current = true;
    setView(place.view);
    setSelectedSessionId(place.sessionId ?? undefined);
    setParadigm(place.paradigm);
    if (place.view === "workspace") {
      setActiveTab("agent");
      setExpandedWorkerId(undefined);
    }
  }, []);

  const goBack = useCallback(() => {
    if (navPlaces.index <= 0) return;
    const index = navPlaces.index - 1;
    applyPlace(navPlaces.stack[index]);
    setNavPlaces(current => ({ ...current, index }));
  }, [applyPlace, navPlaces]);

  const goForward = useCallback(() => {
    if (navPlaces.index >= navPlaces.stack.length - 1) return;
    const index = navPlaces.index + 1;
    applyPlace(navPlaces.stack[index]);
    setNavPlaces(current => ({ ...current, index }));
  }, [applyPlace, navPlaces]);

  const canBack = navPlaces.index > 0;
  const canForward = navPlaces.index < navPlaces.stack.length - 1;
  useEffect(() => {
    const previous = browserSessionRef.current;
    browserSessionRef.current = selectedSessionId;
    if (previous && previous !== selectedSessionId) void bridgeApi.browserBridgeState().then(browser => browser.lease ? bridgeApi.detachBrowser() : undefined).catch(() => undefined);
  }, [selectedSessionId]);

  const adapters = health?.adapters ?? [];
  const adaptersReady = adapters.some(adapter => adapter.available);
  // Everything a human may meet. Filtered once, here, because the rail, Mission Control
  // and default selection disagreeing about what exists is how a briefing run ends up
  // on a grid nobody can focus.
  const visibleSessions = useMemo(() => state.sessions.filter(s => !isHiddenSession(s)), [state.sessions]);
  const topSessions = useMemo(() => visibleSessions.filter(s => s.harness !== "shell" && !s.parentSessionId), [visibleSessions]);
  // Resolve across every session, not just top-level ones: a worker can be
  // opened directly (from Mission Control or a blocked-approval link) so its own
  // conversation — and the approval card that lives on it — is reachable.
  const session = state.sessions.find(s => s.id === selectedSessionId && s.harness !== "shell" && !isHiddenSession(s));
  const workspace = session?.workspaceId ? state.workspaces.find(w => w.id === session.workspaceId) : undefined;
  const hasRepo = !!workspace?.path;
  const isDirectChat = session?.kind === "direct";
  // A focused worker is watchable, its approvals are resolvable, and it can be
  // steered — the composer says "steer", not "message", because the worker still
  // answers to the objective its orchestrator gave it.
  const isWorkerView = !!session?.parentSessionId;
  const workerRuntime = useMemo(
    () => forest?.workerRuntimes.find(runtime => runtime.sessionId === session?.id),
    [forest?.workerRuntimes, session?.id],
  );
  // The three durable facts the backend gate reads, mirrored so the composer is
  // not offered for a steer that is going to be refused.
  const workerSteerable = isWorkerView
    && workerRuntime?.resultStatus !== "reported"
    && workerRuntime?.lifecycleState !== "checkpointing"
    && liveStatuses.includes(session?.status ?? "stopped");
  const sessionConnected = !!session && !session.endedAt && liveStatuses.includes(session.status);
  const sessionEvents = useMemo(() => agentEvents.filter(event => event.sessionId === session?.id), [agentEvents, session?.id]);
  // A worker panel reads the worker's own session row, its runtime record, and
  // its slice of the *global* live stream — the parent's slice would show none
  // of the child's frames.
  const workerPanelSource = useMemo(
    () => ({ sessions: state.sessions, runtimes: forest?.workerRuntimes ?? [], events: agentEvents }),
    [agentEvents, forest?.workerRuntimes, state.sessions],
  );
  const expandedWorker = useMemo(
    () => state.sessions.find(candidate => candidate.id === expandedWorkerId),
    [expandedWorkerId, state.sessions],
  );
  const pendingForSession = useMemo(() => pending.filter(p => p.sessionId === session?.id).map(p => p.text), [pending, session?.id]);
  const conversationStarted = useMemo(() => {
    if (!session) return false;
    if (session.activeTurnId) return true;
    if (pendingForSession.length > 0) return true;
    const durable = forest?.entries?.length ? projectSessionConversation(forest.entries, forest.head?.activeEntryId ?? null) : [];
    return durable.some(item => item.type === "message" && item.role === "user");
  }, [forest, pendingForSession.length, session]);
  const [worktreeOn, setWorktreeOn] = useState(false);
  const [welcomeWorkspaceId, setWelcomeWorkspaceId] = useState<string | null>(null);
  const [branchWorkspaceId, setBranchWorkspaceId] = useState<string | null>(null);
  const [workspaceBranches, setWorkspaceBranches] = useState<string[]>([]);
  const [workspaceBranchCurrent, setWorkspaceBranchCurrent] = useState<string | null>(null);
  const [branchBusy, setBranchBusy] = useState(false);
  const [branchError, setBranchError] = useState<string | null>(null);
  const branchRequestGeneration = useRef(0);
  const newChatPendingRef = useRef(false);
  useEffect(() => {
    if (!session) { setWorktreeOn(false); return; }
    const tracked = worktreeBySessionRef.current.get(session.id);
    if (tracked !== undefined) { setWorktreeOn(tracked); return; }
    setWorktreeOn(!!session.cwd && !!workspace?.path && session.cwd !== workspace.path);
  }, [session, workspace?.path]);
  useEffect(() => {
    setWorkspaceBranchCurrent(workspace?.branch ?? null);
  }, [workspace?.id, workspace?.branch]);
  // Read from config rather than held in component state: the badge has to agree
  // with what the host stored, including after another window changed it.
  const [permissionPolicy, setPermissionPolicy] = useState<PermissionPolicy>();
  // Which settings section to open on. The badge is the one entry point that has
  // an opinion: sending someone hunting through Agents for the switch they just
  // clicked "click to change" on is the wrong end of the promise.
  const [settingsSection, setSettingsSection] = useState<SettingsSection>("agents");
  const autoApprovals = useMemo(
    () => state.events.filter(event => event.kind === "approval.auto_allowed"),
    [state.events],
  );
  // What the submit affordance does while this session is working. Read from the
  // harness's advertised capabilities: a provider that cannot take input
  // mid-turn gets its follow-up queued, and the button says Queue, not Steer.
  const activeAction = useMemo(
    () => activeTurnAction(adapters.find(adapter => adapter.id === session?.harness)?.capabilities),
    [adapters, session?.harness],
  );
  // Folded from the durable event feed, so a reconnect reports the same waiting
  // follow-ups the composer showed before it.
  const queuedFollowUpCount = useMemo(
    () => (session ? queuedFollowUps(session.id, state.events).length : 0),
    [session, state.events],
  );
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
  // $harness shortcut: a bare `$token` at the start of the composer with
  // nothing typed after it yet offers the available harnesses to complete to.
  const harnessShortcutQueryValue = harnessShortcutQuery(composer);
  const harnessShortcutMatches = useMemo(() => {
    if (harnessShortcutQueryValue == null) return [];
    const query = harnessShortcutQueryValue.toLowerCase();
    return adapters
      .filter(adapter => adapter.available && adapter.id.toLowerCase().includes(query))
      .sort((a, b) => {
        const aPrefix = Number(a.id.toLowerCase().startsWith(query));
        const bPrefix = Number(b.id.toLowerCase().startsWith(query));
        if (aPrefix !== bPrefix) return bPrefix - aPrefix;
        return a.id.localeCompare(b.id);
      });
  }, [harnessShortcutQueryValue, adapters]);
  const harnessShortcutOpen = harnessShortcutQueryValue != null && harnessShortcutMatches.length > 0 && !harnessShortcutDismissed;
  const harnessShortcutListRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const query = composer.trim();
    if (!session || !(["codex", "claude", "opencode"] as Harness[]).includes(session.harness) || query.length < 8 || query.startsWith("/")) { setSkillSuggestions([]); return; }
    const provider = session.harness as SkillProvider;
    let active = true;
    const timer = window.setTimeout(() => { void bridgeApi.skillSuggestions(query, provider).then(items => { if (active) setSkillSuggestions(items.slice(0, 3)); }).catch(() => { if (active) setSkillSuggestions([]); }); }, 300);
    return () => { active = false; window.clearTimeout(timer); };
  }, [composer, session]);

  // The inline typeahead's own settings — loaded once; Settings' save path
  // keeps this fresh via `onSuggestionSettingsChange`.
  useEffect(() => { void bridgeApi.getSuggestionSettings().then(setSuggestionSettings).catch(() => undefined); }, []);

  // A new configured model/provider earns its own one-time fallback notice.
  useEffect(() => { fallbackNoticeShownRef.current = false; }, [suggestionSettings?.settings.provider, suggestionSettings?.settings.model]);

  // Debounced draft completion: 400ms after the last keystroke, with a
  // generation counter so a stale response from an earlier draft can never
  // overwrite a newer one — the same latest-wins discipline `readWorkBoard`
  // uses. No request fires with the toggle off, no session, or an empty draft.
  useEffect(() => scheduleSuggestion({
    text: composer,
    enabled: !!suggestionSettings?.settings.enabled && !!session,
    request: bridgeApi.suggestCompletion,
    onResult: setDraftSuggestion,
    generation: suggestionGeneration,
  }), [composer, session, suggestionSettings?.settings.enabled, suggestionSettings?.settings.provider, suggestionSettings?.settings.model]);

  // The fallback chip: shown once per episode, not re-shown on every debounce
  // firing while the configured model stays in its cooldown window.
  useEffect(() => {
    if (!draftSuggestion?.usedFallback || fallbackNoticeShownRef.current) return;
    fallbackNoticeShownRef.current = true;
    const reason = draftSuggestion.fallbackReason?.replace(/_/g, " ");
    setFallbackNotice(`Suggestions switched to a fallback model${reason ? ` (${reason})` : ""} while yours is unavailable.`);
    const timer = window.setTimeout(() => setFallbackNotice(undefined), 6000);
    return () => window.clearTimeout(timer);
  }, [draftSuggestion]);

  const acceptSuggestion = useCallback(() => {
    if (!draftSuggestion?.suggestion) return;
    setComposer(current => current + draftSuggestion.suggestion);
    setDraftSuggestion(undefined);
  }, [draftSuggestion]);

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

  useEffect(() => {
    if (!harnessShortcutOpen) return;
    setHarnessShortcutIndex(index => Math.min(index, Math.max(0, harnessShortcutMatches.length - 1)));
  }, [harnessShortcutOpen, harnessShortcutMatches.length]);

  useEffect(() => {
    if (!harnessShortcutOpen) return;
    const root = harnessShortcutListRef.current;
    if (!root) return;
    const active = root.querySelector<HTMLElement>(`[data-harness-shortcut-index="${harnessShortcutIndex}"]`);
    active?.scrollIntoView({ block: "nearest" });
  }, [harnessShortcutOpen, harnessShortcutIndex]);

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
    let pollsSinceFullFetch = 0;
    const refresh = async () => {
      // The digest is tens of bytes; the snapshot is the entire history. Only
      // fetch the snapshot when the digest moves, with a periodic forced
      // fetch as the safety net for state the store cannot see (repository
      // divergence above all).
      const digest = await bridgeApi.sessionForestDigest(session.id).catch(() => undefined);
      const force = pollsSinceFullFetch >= 9 || digest === undefined;
      if (!active) return;
      if (!force && digest === forestKeyRef.current) {
        pollsSinceFullFetch += 1;
        return;
      }
      const [value, adoptions] = await Promise.all([
        bridgeApi.sessionForest(session.id).catch(() => undefined),
        bridgeApi.pendingWorkerAdoptions(session.id).catch(() => []),
      ]);
      if (!active) return;
      pollsSinceFullFetch = 0;
      setPendingAdoptions(adoptions);
      if (!value) return;
      forestKeyRef.current = digest ?? "";
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
  function openSession(id: string) {
    setView("workspace");
    setParadigm("single");
    setActiveTab("agent");
    setSelectedSessionId(id);
    setExpandedWorkerId(undefined);
    const opened = state.sessions.find(candidate => candidate.id === id);
    if (opened?.workspaceId) writeLastWorkspaceId(opened.workspaceId);
  }

  // Reading the board is the whole of what opening Work does: one call, no session
  // selected, no model, no git, no network.
  const readWorkBoard = useCallback(async () => {
    const generation = ++workReadGeneration.current;
    try {
      const board = await bridgeApi.workBoard();
      if (generation !== workReadGeneration.current) return;
      workBoardRef.current = board;
      setWorkBoard(board);
      setWorkError(undefined);
      setWorkRefreshError(undefined);
    } catch (error) {
      if (generation !== workReadGeneration.current) return;
      const reason = errorMessage(error);
      // A failed re-read never throws away a board that is on screen. The numbers
      // are a snapshot either way, and replacing them with an error panel loses
      // what the reader had without telling them anything they can act on.
      if (workBoardRef.current === undefined) setWorkError(reason);
      else setWorkRefreshError(reason);
    }
  }, []);

  const openWorkBoard = useCallback(() => {
    setView("work");
    setParadigm("single");
    void readWorkBoard();
  }, [readWorkBoard]);

  // Local state on a suggested task. Nothing here reaches a connector — see
  // bridge_core::work_actions — so a failure is Bridge's own and shows on the row.
  const runWorkTaskAction = useCallback(async (task: WorkTask, action: TaskAction): Promise<WorkActionOutcome> => {
    try {
      if (action === "start") {
        const preferred = adapters.find(adapter => adapter.available);
        const prepared = await bridgeApi.workTaskPrepareSession(
          task.id,
          (preferred?.id as Harness) ?? "codex",
          preferred?.defaultModel ?? null,
        );
        // Open the session with the draft in the composer. Nothing is sent: the user edits
        // and presses Send, which is the whole point of preparing rather than starting.
        setState(await bridgeApi.state());
        setComposer(prepared.draft);
        openSession(prepared.sessionId);
        return { ok: true };
      }
      const snoozedUntil = action === "snooze"
        ? new Date(Date.now() + 24 * 60 * 60 * 1000).toISOString()
        : null;
      await bridgeApi.workTaskAction(task.id, action, snoozedUntil);
      await readWorkBoard();
      return { ok: true };
    } catch (error) {
      return { ok: false, reason: errorMessage(error) };
    }
  }, [adapters, readWorkBoard]);

  const toggleWorkTaskPin = useCallback(async (task: WorkTask): Promise<WorkActionOutcome> => {
    try {
      await bridgeApi.workTaskPin(task.id, !task.pinned);
      await readWorkBoard();
      return { ok: true };
    } catch (error) {
      return { ok: false, reason: errorMessage(error) };
    }
  }, [readWorkBoard]);

  const openWorkTaskEvidence = useCallback(async (task: WorkTask): Promise<void> => {
    try {
      const target = await bridgeApi.workTaskOpenEvidence(task.id);
      if (target.kind === "session") {
        openSession(target.sessionId);
        return;
      }
      await openExternalUrl(target.url);
    } catch (error) {
      setError(errorMessage(error));
    }
  }, []);

  // A row routes by what the task is: a workspace-bound task opens Code on the
  // most recent session in that workspace; everything else belongs to Work,
  // which the reader is already on. The route comes from the task's own fields
  // (taskRoute) — never from a model-authored URL.
  const openWorkTask = useCallback((task: WorkTask): void => {
    const route = taskRoute(task);
    if (route.kind !== "code") return;
    const inWorkspace = visibleSessions.find(item => item.workspaceId === route.workspaceId);
    if (inWorkspace) {
      openSession(inWorkspace.id);
      return;
    }
    setView("workspace");
    setParadigm("single");
  }, [visibleSessions]);

  // Ask for a fresh briefing, then follow the board while the run lands. The
  // receipt is not the result — the run settles on its own thread — so all this
  // does is surface a refusal and re-read.
  const runWorkBriefing = useCallback(async (trigger: "manual" | "focus"): Promise<void> => {
    try {
      const receipt = await bridgeApi.runWorkBriefing(trigger);
      if (receipt.outcome === "refused" && trigger === "manual") {
        setWorkRefreshError(receipt.detail ?? receipt.code ?? "the briefing was refused");
      }
    } catch (error) {
      if (trigger === "manual") setWorkRefreshError(errorMessage(error));
    }
    void readWorkBoard();
  }, [readWorkBoard]);

  // The opt-in focus trigger. Gated on the stored settings the board carries, so
  // a user who never opted in gets no background model run from switching apps.
  useEffect(() => {
    const onFocus = () => {
      if (!workBoardRef.current?.settings.refreshOnFocus) return;
      void runWorkBriefing("focus");
    };
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [runWorkBriefing]);

  // Two of the four actions are navigation and two are calls. A board button must
  // never answer an approval on the user's behalf — it takes them to where the
  // decision is made — while a fast-forward and a re-measure are Bridge's own work
  // and report their own failure.
  const runWorkAction = useCallback(async (action: WorkFactAction): Promise<WorkActionOutcome> => {
    try {
      switch (action.kind) {
        case "reviewCompletionCheck":
        case "answerApproval":
          openSession(action.sessionId);
          return { ok: true };
        case "refreshWorkspaceBase":
          await bridgeApi.refreshWorkspaceBase(action.sessionId);
          await readWorkBoard();
          return { ok: true };
        case "refreshBaseObservation":
          // A user-triggered reading is written through to the cache the board reads,
          // so re-measuring is the same call the warning offers.
          await bridgeApi.workspaceBaseDivergence(action.sessionId, false);
          await readWorkBoard();
          return { ok: true };
      }
    } catch (error) {
      return { ok: false, reason: errorMessage(error) };
    }
  }, [readWorkBoard]);
  // New chat opens instantly (no picker up front). Preserve the current direct
  // chat's harness/model so switching to OpenCode also changes the next-chat
  // default; otherwise fall back to the configured standard profile.
  // `carryFromSessionId` optionally seeds the new chat with a durable handoff
  // brief projecting that session's stored context, so its first turn knows
  // what "this" refers to ($harness shortcut).
  async function openNewChat(initialMessage?: string, harnessOverride?: import("./types").AdapterDescriptor, alreadyLocked = false, carryFromSessionId?: string): Promise<string | undefined> {
    if (!alreadyLocked) {
      if (newChatPendingRef.current) return;
      newChatPendingRef.current = true;
    }
    try {
      if (!adaptersReady) { setError("No model adapter is available. Install or sign in to Codex, Claude, or OpenCode, then retry model setup."); return; }
      setView("workspace");
      const currentAdapter = session?.kind === "direct"
        ? adapters.find(adapter => adapter.id === session.harness && adapter.available)
        : undefined;
      const profile = modelSetup ? resolveProfileOption("standard_orchestrator", modelSetup, adapters) : undefined;
      const preferred = harnessOverride ?? currentAdapter ?? profile?.adapter ?? adapters.find(adapter => adapter.available) ?? adapters[0];
      const harness = (preferred?.id as Harness) ?? "codex";
      const model = harnessOverride
        ? harnessOverride.defaultModel ?? harnessOverride.models[0]?.id ?? null
        : currentAdapter
        ? session?.model ?? currentAdapter.defaultModel ?? currentAdapter.models[0]?.id ?? null
        : profile?.model.id ?? preferred?.defaultModel ?? preferred?.models[0]?.id ?? null;
      const draft = initialMessage?.trim() ?? "";
      if (draft) pendingWelcomeMessageRef.current = draft;
      setBusy(true); setError(undefined);
      try {
        const next = await bridgeApi.createChat(harness, model, null);
        const created = [...next.sessions].reverse().find(s => !s.parentSessionId && !s.workspaceId);
        // Carry before selecting: the welcome-message effect fires on
        // session-id change, and the brief must be in the forest before the
        // first cold start compiles its prompt.
        if (created && carryFromSessionId && carryFromSessionId !== created.id) {
          try { await bridgeApi.carrySessionHandoff(created.id, carryFromSessionId); }
          catch { /* context carry is best-effort; the chat starts regardless */ }
        }
        setState(next);
        if (created) setSelectedSessionId(created.id);
        return created?.id;
      } catch (e) {
        pendingWelcomeMessageRef.current = null;
        setError(errorMessage(e));
      }
      finally { setBusy(false); }
    } finally {
      if (!alreadyLocked) newChatPendingRef.current = false;
    }
  }

  // A `$harness` prefix (e.g. `$codex are we right?`) bypasses whatever
  // session is open and starts a fresh direct chat pinned to that harness,
  // handing it the rest of the text as its first message — plus a projected
  // handoff brief of this conversation, so the question has its context.
  // Returns whether the text was a shortcut at all, so the caller knows
  // whether to fall back to its own normal send path.
  async function openHarnessShortcut(text: string, alreadyLocked = false): Promise<boolean> {
    const shortcut = parseHarnessShortcut(text);
    if (!shortcut) return false;
    const adapter = adapters.find(item => item.id.toLowerCase() === shortcut.harnessId.toLowerCase());
    if (!adapter) return false;
    if (!adapter.available) {
      setError(`${adapter.label} isn't available${adapter.unavailableReason ? ` — ${adapter.unavailableReason}` : ""}.`);
      return true;
    }
    const carryFromSessionId = session?.id;
    await openNewChat(shortcut.rest, adapter, alreadyLocked, carryFromSessionId);
    return true;
  }
  // Entry point for the Welcome screen's own composer, which has no session
  // to skip past — a `$harness` prefix there is the only branch either way.
  async function startChatOrShortcut(text?: string) {
    if (newChatPendingRef.current) return;
    newChatPendingRef.current = true;
    try {
      if (text && await openHarnessShortcut(text, true)) return;
      const workspaceId = resolveNewChatWorkspaceId({
        activeWorkspaceId: welcomeWorkspaceId,
        lastWorkspaceId: readLastWorkspaceId(),
        workspaces: state.workspaces,
      });
      if (!workspaceId) {
        await openNewChat(text, undefined, true);
        return;
      }
      const draft = text?.trim() ?? "";
      if (draft) pendingWelcomeMessageRef.current = draft;
      const createdId = await newWorkspaceSession(false, workspaceId, true);
      if (!createdId) pendingWelcomeMessageRef.current = null;
    } finally {
      newChatPendingRef.current = false;
    }
  }

  async function startChatInCurrentRepo() {
    if (newChatPendingRef.current) return;
    newChatPendingRef.current = true;
    try {
      if (!adaptersReady) { setError("No model adapter is available. Install or sign in to Codex, Claude, or OpenCode, then retry model setup."); return; }
      const workspaceId = resolveNewChatWorkspaceId({
        activeWorkspaceId: session?.workspaceId,
        lastWorkspaceId: readLastWorkspaceId(),
        workspaces: state.workspaces,
      });
      if (!workspaceId) {
        await openNewChat(undefined, undefined, true);
        return;
      }
      await newWorkspaceSession(false, workspaceId, true);
    } finally {
      newChatPendingRef.current = false;
    }
  }

  async function retargetWorkspace(workspaceId: string, createWorktree: boolean) {
    const previousId = session && forest !== undefined && !conversationStarted ? session.id : undefined;
    const createdId = await newWorkspaceSession(createWorktree, workspaceId);
    if (previousId && createdId && previousId !== createdId) {
      try { setState(await bridgeApi.stopSession(previousId)); } catch { /* the empty chat we replaced */ }
    }
  }

  async function requestWorkspaceBranches(workspaceId: string) {
    const generation = ++branchRequestGeneration.current;
    setBranchWorkspaceId(workspaceId);
    setBranchBusy(true);
    setBranchError(null);
    try {
      const result = await bridgeApi.listWorkspaceBranches(workspaceId);
      if (branchRequestGeneration.current !== generation) return;
      setWorkspaceBranches(result.branches);
      setWorkspaceBranchCurrent(result.current ?? null);
    } catch (error) {
      if (branchRequestGeneration.current !== generation) return;
      setWorkspaceBranches([]);
      setBranchError(errorMessage(error));
    } finally {
      if (branchRequestGeneration.current === generation) setBranchBusy(false);
    }
  }

  async function switchWorkspaceBranch(workspaceId: string, branch: string) {
    const generation = ++branchRequestGeneration.current;
    setBranchWorkspaceId(workspaceId);
    setBranchBusy(true);
    setBranchError(null);
    setError(undefined);
    try {
      const next = await bridgeApi.checkoutWorkspaceBranch(workspaceId, branch);
      if (branchRequestGeneration.current !== generation) return;
      setState(next);
      const switched = next.workspaces.find(item => item.id === workspaceId)?.branch ?? branch;
      setWorkspaceBranchCurrent(switched);
      try {
        const result = await bridgeApi.listWorkspaceBranches(workspaceId);
        if (branchRequestGeneration.current !== generation) return;
        setWorkspaceBranches(result.branches);
        setWorkspaceBranchCurrent(result.current ?? null);
      } catch {
        // Checkout already committed; a stale menu is not a failed switch.
      }
    } catch (error) {
      if (branchRequestGeneration.current !== generation) return;
      const message = errorMessage(error);
      setBranchError(message);
      setError(message);
    } finally {
      if (branchRequestGeneration.current === generation) setBranchBusy(false);
    }
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
  async function newWorkspaceSession(createWorktree: boolean, explicitWorkspaceId?: string, alreadyLocked = false) {
    if (!alreadyLocked) {
      if (newChatPendingRef.current) return;
      newChatPendingRef.current = true;
    }
    try {
      const workspaceId = explicitWorkspaceId ?? pendingWorkspaceId;
      if (!workspaceId) return;
      setBusy(true); setError(undefined);
      try {
        const next = await bridgeApi.createWorkspaceSession(workspaceId, createWorktree);
        const created = [...next.sessions].reverse().find(s => !s.parentSessionId && s.workspaceId === workspaceId);
        writeLastWorkspaceId(workspaceId);
        setState(next);
        // Land in the new agent's chat rather than leaving the user looking at the
        // card or dialog they came from.
        if (created) {
          worktreeBySessionRef.current.set(created.id, createWorktree);
          openSession(created.id);
        }
        setModal(null); setPendingWorkspaceId(undefined);
        return created?.id;
      } catch (e) { setError(errorMessage(e)); }
      finally { setBusy(false); }
    } finally {
      if (!alreadyLocked) newChatPendingRef.current = false;
    }
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
  // another provider auto-switch the direct-chat harness first. A `$harness`
  // prefix skips this session entirely — see `openHarnessShortcut`.
  async function sendPrompt(forcedText?: string) {
    const submittedText = (forcedText ?? composer).trim();
    if (!submittedText) return;
    if (await openHarnessShortcut(submittedText)) { setComposer(""); return; }
    if (!session) return;
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
      const localOnly = /^\/(usage|cost|stats|clear|new|reset|compact|recall|pins|unpin|pin)(\s|$)/i.test(text);
      if (!localOnly && !liveStatuses.includes(target.status)) {
        startedRef.current.add(target.id);
        setState(await bridgeApi.startChat(target.id));
      }
      // One call whatever the session is doing. The backend decides between
      // starting a turn, steering the live one, and durably queueing, and says
      // which — so the message can be shown in the state it is actually in.
      const outcome = await bridgeApi.submitInput(target.id, text);
      if (outcome.disposition !== "startedNewTurn") {
        const delivery = outcome.disposition === "steeredActiveTurn" ? "steered" as const : "queued" as const;
        setPending(current => current.map(item => item.key === key ? { ...item, delivery } : item));
      }
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
  // The "Memory used" chip is audit-backed: what this session's prompt actually
  // received, re-read on every memory change.
  useEffect(() => {
    setPacketAudit(null);
    setMemoryDisclosureOpen(false);
    const id = session?.id;
    if (!id) return;
    let active = true;
    let off: (() => void) | undefined;
    const load = () => {
      bridgeApi.getPacketAudit(id).then(audit => { if (active) setPacketAudit(audit); }).catch(() => {});
    };
    load();
    void bridgeApi.onMemoryChanged(() => load()).then(fn => {
      if (!active) { fn(); return; }
      off = fn;
    }).catch(() => {});
    return () => { active = false; off?.(); };
  }, [session?.id]);

  // "Remember this" on an assistant message. Over the cap the dialog opens with
  // the full text for the user to trim — never a clip, never a truncated save.
  const rememberMessage = useCallback(async (text: string) => {
    if (rememberAction(text) === "open-dialog") { setMemoryDraft(text); setModal("memory"); return; }
    try { await bridgeApi.saveMemoryRecord(text, undefined, session?.id ?? undefined); }
    catch (e) { setError(errorMessage(e)); }
  }, [session?.id]);

  /// Send guidance into a running worker.
  ///
  /// The same `submitInput` every other session uses: the backend decides
  /// between steering the live turn and queueing for the next boundary, and it
  /// is what tells the orchestrator a human redirected its worker. Deliberately
  /// not `startChat` first — a worker with no process is refused, because
  /// launching one is the worker pool's decision.
  const steerWorker = useCallback(async (childSessionId: string, text: string) => {
    await bridgeApi.submitInput(childSessionId, text);
  }, []);
  // Re-run a failed worker's objective because the user asked. The reason it
  // failed is on the card next to this action, which is the point: Bridge no
  // longer spends this turn on a cause it cannot show has changed.
  const retryWorkerTask = useCallback(async (childSessionId: string) => {
    await bridgeApi.retryWorkerTask(childSessionId);
    await reload();
  }, [reload]);
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
    // Out-of-band fetch: reset the digest so the next poll reconciles.
    forestKeyRef.current = "";
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
  // The `+` control: the system file dialog, so any file on the machine can be
  // attached to any chat — including one with no folder connected. The chosen
  // paths become `@path` mentions, which the backend reads as bounded,
  // secret-sanitized, untrusted context at submit time. The draft is never
  // touched, only added to.
  async function attachFile() {
    if (!("__TAURI_INTERNALS__" in window)) {
      // No system dialog outside the desktop shell; fall back to the workspace
      // picker `@` drives rather than doing nothing.
      setMentionDismissed(false);
      setMentionIndex(0);
      setComposer(current => (current.length === 0 || /\s$/.test(current) ? `${current}@` : `${current} @`));
      composerRef.current?.focus();
      return;
    }
    try {
      const picked = await open({ multiple: true, title: "Attach files" });
      if (picked == null) return;
      const paths = (Array.isArray(picked) ? picked : [picked]).filter(path => typeof path === "string");
      if (paths.length === 0) return;
      setComposer(current => paths.reduce(appendFileMention, current));
    } catch (e) { setError(errorMessage(e)); }
    finally { composerRef.current?.focus(); }
  }
  // Replace the @token being typed at the end of the composer with the picked
  // path, preserving any leading whitespace the mention started after.
  function applyFileMention(path: string) {
    setComposer(current => insertFileMention(current, path));
    setMentionIndex(0);
    setMentionDismissed(true);
  }
  // Complete the `$token` being typed to `$id `, ready for the message that
  // follows — picking one doesn't send anything by itself.
  function applyHarnessShortcut(adapter: import("./types").AdapterDescriptor) {
    setComposer(`$${adapter.id} `);
    setHarnessShortcutIndex(0);
    setHarnessShortcutDismissed(true);
  }
  function onComposerKeyDown(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    if (e.nativeEvent.isComposing) return;
    if (mentionOpen) {
      if (e.key === "ArrowDown") { e.preventDefault(); setMentionIndex(index => Math.min(index + 1, fileMatches.length - 1)); return; }
      if (e.key === "ArrowUp") { e.preventDefault(); setMentionIndex(index => Math.max(index - 1, 0)); return; }
      if (e.key === "Escape") { e.preventDefault(); setMentionDismissed(true); return; }
      if ((e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) || e.key === "Tab") { e.preventDefault(); applyFileMention(fileMatches[Math.min(mentionIndex, fileMatches.length - 1)]); return; }
    }
    if (harnessShortcutOpen) {
      if (e.key === "ArrowDown") { e.preventDefault(); setHarnessShortcutIndex(index => Math.min(index + 1, harnessShortcutMatches.length - 1)); return; }
      if (e.key === "ArrowUp") { e.preventDefault(); setHarnessShortcutIndex(index => Math.max(index - 1, 0)); return; }
      if (e.key === "Escape") { e.preventDefault(); setHarnessShortcutDismissed(true); return; }
      if ((e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) || e.key === "Tab") { e.preventDefault(); applyHarnessShortcut(harnessShortcutMatches[Math.min(harnessShortcutIndex, harnessShortcutMatches.length - 1)]); return; }
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

  useEffect(() => {
    setLayoutFullscreenDocument(fullscreen || flushWindow);
  }, [fullscreen, flushWindow]);

  useEffect(() => {
    notifyLayoutFullscreen(fullscreen);
  }, [fullscreen]);

  useEffect(() => {
    const sync = () => setFlushWindow(isFlushWindowDocument());
    sync();
    document.documentElement.addEventListener(FLUSH_WINDOW_EVENT, sync);
    return () => document.documentElement.removeEventListener(FLUSH_WINDOW_EVENT, sync);
  }, []);

  const toggleLayoutFullscreen = useCallback(() => {
    setFullscreen(value => !value);
  }, []);

  const chromeFullscreen = fullscreen || flushWindow;
  const turnActive = !!session?.activeTurnId || pendingForSession.length > 0;
  if (!health || !modelSetup) return <div className="relative grid h-[100dvh] place-items-center overflow-hidden bg-background text-muted-foreground"><div className="relative z-10 flex max-w-md items-center gap-2 px-6 text-center text-xs">{error ? <><X size={14} className="text-destructive" aria-hidden="true" />{error}</> : <><LoaderCircle className="animate-spin" size={14} aria-hidden="true" />Loading Bridge…</>}</div></div>;
  if (shouldRequireModelSetup(modelSetup, health.adapters)) return <div className="relative h-[100dvh] overflow-hidden bg-background"><ModelSetupWizard adapters={health.adapters} onComplete={setModelSetup} onError={setError} />{error && <Alert variant="error" className="fixed bottom-5 right-5 z-[60] max-w-md"><AlertTitle>Model setup failed</AlertTitle><AlertDescription>{error}</AlertDescription></Alert>}</div>;
  const chromeTitle = view === "work" ? "Work" : view === "projects" ? "Projects" : view === "marketplace" ? "Marketplace" : view === "automations" ? "Automations" : view === "settings" ? "Settings" : session?.title || session?.label || "Bridge";
  const titleBarActions = <>
    <BypassBadge bypassing={!!permissionPolicy?.bypassAll} onOpenSettings={() => { setSettingsSection("permissions"); setView("settings"); }} />
    <UsageWidget usage={usageByProvider} samples={usageSamples} history={usageHistory} cacheDiagnostics={cacheDiagnostics} contextPercent={latestContext ?? undefined} contextSource={latestContextSource} focusedSessionId={session?.id ?? null} onOpenPromptStudio={() => { setSettingsSection("prompts"); setView("settings"); }} />
  </>;
  const sidebar = (
    <BridgeSidebar
      mobileOpen={navOpen}
      onCloseMobile={() => setNavOpen(false)}
      chats={topSessions}
      workspaces={state.workspaces}
      activeSessionId={session?.id}
      projectsActive={view === "projects"}
      automationsActive={view === "automations"}
      missionControlActive={view === "workspace" && paradigm === "grid"}
      workActive={view === "work"}
      settingsActive={view === "settings"}
      accountName={localAccountName(health.database, workspace?.path)}
      newChatBusy={busy}
      onOpenNewChat={() => void startChatInCurrentRepo()}
      onOpenProjects={() => setView("projects")}
      onOpenAutomations={() => setView("automations")}
      onOpenMissionControl={() => { setView("workspace"); setParadigm("grid"); }}
      onOpenWorkBoard={openWorkBoard}
      onOpenMemory={() => setModal("memory")}
      onOpenSettings={() => setView("settings")}
      onOpenSession={openSession}
      collapsed={sidebarCollapsed}
      onCollapsedChange={setSidebarCollapsed}
      showWindowNav
      canBack={canBack}
      canForward={canForward}
      onBack={goBack}
      onForward={goForward}
    />
  );
  const resolvedWelcomeWorkspaceId = resolveNewChatWorkspaceId({
    activeWorkspaceId: welcomeWorkspaceId,
    lastWorkspaceId: readLastWorkspaceId(),
    workspaces: state.workspaces,
  });
  const welcomeWorkspace = state.workspaces.find(item => item.id === resolvedWelcomeWorkspaceId) ?? null;

  return <div data-fullscreen={chromeFullscreen ? "" : undefined} className="u-app-shell relative flex h-[100dvh] flex-row overflow-hidden text-foreground">
    {sidebar}
    <div className="u-vibrancy-canvas relative z-10 flex min-h-0 min-w-0 flex-1 flex-col bg-background">
    <AppTitleBar
      flush
      hideBrand
      title={chromeTitle}
      navOpen={navOpen}
      onOpenNav={() => setNavOpen(true)}
      actions={titleBarActions}
    />
    <main className="relative z-10 min-w-0 flex-1 overflow-hidden flex flex-col animate-page-mount">
      {!adaptersReady && <Alert variant="warning" className="mx-auto mt-4 w-[calc(100%-2rem)] max-w-2xl"><AlertTitle>No model adapters available</AlertTitle><AlertDescription>Bridge remains accessible, but chats and orchestrators are disabled until Codex, Claude, or OpenCode is installed and signed in.</AlertDescription></Alert>}
      <HealthWarnings warnings={health.warnings ?? []} className="mx-auto mt-4 w-[calc(100%-2rem)] max-w-2xl" />
      {view === "work" ? <Suspense fallback={<PanelLoading label="Opening work…"/>}><WorkView
        board={workBoard}
        error={workError}
        refreshError={workRefreshError}
        onRefresh={() => void readWorkBoard()}
        onAction={runWorkAction}
        onTaskAction={runWorkTaskAction}
        onTogglePin={toggleWorkTaskPin}
        onOpenEvidence={task => void openWorkTaskEvidence(task)}
        onOpenTask={openWorkTask}
        onRunBriefing={() => void runWorkBriefing("manual")}
      /></Suspense> : view === "projects" ? <ProjectsScreen
        workspaces={state.workspaces}
        chats={topSessions}
        activeSessionId={session?.id}
        busy={busy}
        onOpenSession={openSession}
        onNewWorkspace={() => { setTitle(""); setModal("workspace"); }}
        onNewWorkspaceSession={requestWorkspaceSession}
        onConnectFolder={workspaceId => void connectFolder(workspaceId)}
      /> : view === "automations" ? <Suspense fallback={<PanelLoading label="Opening automations…"/>}><AutomationsPanel onBrowseCatalog={() => setView("marketplace")} /></Suspense> : view === "marketplace" ? <Suspense fallback={<PanelLoading label="Opening marketplace…"/>}><MarketplaceScreen /></Suspense> : view === "settings" ? <Suspense fallback={<PanelLoading label="Opening settings…"/>}><SettingsScreen adapters={adapters} autoApprovals={autoApprovals} initialSection={settingsSection} onModelSetupChange={setModelSetup} onSuggestionSettingsChange={setSuggestionSettings} onError={setError} /></Suspense> : paradigm === "grid" ? <MissionControl
        sessions={visibleSessions}
        runtimes={forest?.workerRuntimes ?? []}
        reasons={forest?.reasons ?? []}
        events={agentEvents}
        activeSessionId={session?.id}
        fullscreen={fullscreen}
        onToggleFullscreen={toggleLayoutFullscreen}
        onFocusSession={openSession}
        onSteer={steerWorker}
      /> : session ? <>
        <SessionToolbar
          title={session.title || session.label}
          tabs={hasRepo ? [
            { id: "agent", label: "Agent", icon: MessageSquareText },
            { id: "changes", label: "Changes", icon: FileCode2, badge: workspace?.dirtyFiles || undefined },
            { id: "code", label: "Code", icon: Code2 },
            { id: "terminal", label: "Terminal", icon: TerminalSquare },
          ] : [{ id: "agent", label: "Agent", icon: MessageSquareText }]}
          activeTab={activeTab}
          onTabChange={id => setActiveTab(id as typeof activeTab)}
          model={isDirectChat ? undefined : modelDisplayName(adapters, session.harness, session.model)}
          browserOpen={browserOpen}
          onToggleBrowser={() => setBrowserOpen(value => !value)}
          fullscreen={fullscreen}
          onToggleFullscreen={toggleLayoutFullscreen}
          onOpenRouterSettings={!isDirectChat && workspace ? () => setModal("router") : undefined}
          onToggleRecall={() => {
            setActiveTab("agent");
            setRecallOpen(open => !open);
          }}
          recallOpen={recallOpen}
          onEnd={sessionConnected ? () => void endChat() : undefined}
          busy={busy}
        />
        <section className="flex-1 min-h-0 overflow-hidden flex relative">
          {/* The chat's own panel, expanded. Rendered over the session pane
              rather than navigating away, because the reason to look at a
              worker's full feed is usually to decide something in the
              conversation you are still in. */}
          {expandedWorker && <div className="absolute inset-0 z-30 flex min-h-0 flex-col bg-background">
            <WorkerDetail
              session={expandedWorker}
              runtime={forest?.workerRuntimes.find(runtime => runtime.sessionId === expandedWorker.id)}
              liveEvents={agentEvents}
              onClose={() => setExpandedWorkerId(undefined)}
              onFocusSession={openSession}
              onSteer={steerWorker}
            />
          </div>}
          <div className="flex-1 min-w-0 flex flex-col relative">
            {(activeTab === "agent" || !hasRepo) && <>
              {recallOpen && (
                <SessionRecallSearch
                  sessionId={session.id}
                  onClose={() => { setRecallOpen(false); setHighlightEntryId(null); }}
                  onJump={entryId => {
                    setHighlightEntryId(entryId);
                    requestAnimationFrame(() => {
                      document.getElementById(`forest-entry-${entryId}`)?.scrollIntoView({ behavior: "smooth", block: "center" });
                    });
                  }}
                />
              )}
              <div className="flex-1 min-h-0 relative">
                <AgentConversation
                  session={session}
                  onOpenSession={openSession}
                  workers={workerPanelSource}
                  onExpandWorker={setExpandedWorkerId}
                  events={sessionEvents}
                  forestEntries={forest?.entries}
                  activeLeafId={forest?.head?.activeEntryId}
                  repositoryDivergence={forest?.repositoryDivergence.status}
                  completion={forest?.completion}
                  onWaiveCompletion={waiveCompletion}
                  onRefreshBase={refreshWorkspaceBase}
                  onRetryWorker={retryWorkerTask}
                  pendingAdoptions={pendingAdoptions}
                  onResolveAdoption={resolveAdoption}
                  continuationFidelity={session?.continuationFidelity}
                  preview={false}
                  working={turnActive}
                  pendingMessages={pendingForSession}
                  onResolve={resolveApproval}
                  highlightEntryId={highlightEntryId}
                  onRemember={rememberMessage}
                />
              </div>
              <div className="pointer-events-none absolute bottom-0 left-0 right-0 h-16 bg-gradient-to-t from-background to-transparent sm:h-20" />
              <div className="relative z-10 flex-none safe-bottom">
                {/* A follow-up the provider cannot take mid-turn is held, not
                    dropped. Saying so is the difference between a considered
                    queue and an agent that ignored you. */}
                <MemoryUsedChip audit={packetAudit} open={memoryDisclosureOpen} onToggle={() => setMemoryDisclosureOpen(current => !current)} />
                {queuedFollowUpCount > 0 && <div className="mx-auto mb-2 flex max-w-2xl justify-center px-4 sm:px-6">
                  <div className="u-glass-soft inline-flex items-center gap-2 h-[30px] px-3.5 rounded-full text-muted-foreground text-xs" role="status">
                    <Clock3 size={12} aria-hidden="true" />
                    <span>{`${queuedFollowUpCount} follow-up${queuedFollowUpCount === 1 ? "" : "s"} queued — sent when this step finishes`}</span>
                  </div>
                </div>}
                {hasRepo && workspace && workspace.dirtyFiles > 0 && <div className="mx-auto mb-2 flex max-w-2xl justify-center px-4 sm:px-6">
                  <div className="u-glass-soft inline-flex items-center gap-2 h-[30px] px-3.5 rounded-full text-muted-foreground text-xs">
                    <FileDiff size={12} aria-hidden="true" />
                    <span>{`${workspace.dirtyFiles} file${workspace.dirtyFiles === 1 ? "" : "s"}`}</span>
                    <em className="not-italic font-mono text-[11px]"><b className="text-success">+{workspace.additions}</b> <b className="text-destructive">−{workspace.deletions}</b></em>
                  </div>
                </div>}
                {fallbackNotice && <div className="mx-auto mb-2 flex max-w-2xl justify-center px-4 sm:px-6">
                  <div className="u-glass-soft inline-flex items-center gap-2 h-[30px] px-3.5 rounded-full text-muted-foreground text-xs" role="status">
                    <span>{fallbackNotice}</span>
                  </div>
                </div>}
                {/* A worker gets a steering composer, not the chat composer: what
                    you type amends the objective its orchestrator gave it, and
                    the orchestrator is told so it does not fight the change. */}
                {isWorkerView ? <div className="mx-auto max-w-2xl px-4 sm:px-6">
                  <div className="u-glass-soft flex items-center gap-2.5 rounded-2xl px-4 py-2.5 text-[12px] text-muted-foreground"><Bot size={14} className="shrink-0 text-muted-foreground" aria-hidden="true" /><span>This is a background worker. It takes its objective from its orchestrator — steer it here to amend that objective.</span></div>
                  <SteerComposer sessionId={session.id} steerable={!!workerSteerable} onSteer={steerWorker} className="pt-2"/>
                </div> : <div className="relative mx-auto max-w-2xl">
                  {!slashOpen && !mentionOpen && !harnessShortcutOpen && skillSuggestions.length > 0 && <div className="u-glass-popover absolute bottom-full left-4 right-4 z-20 mb-2 overflow-hidden rounded-2xl sm:left-6 sm:right-6"><div className="border-b border-border px-3 py-1.5 text-[9px] uppercase tracking-[0.12em] text-muted-foreground/70">Available skills for this task</div>{skillSuggestions.map(suggestion => <button key={suggestion.id} type="button" onMouseDown={event => { event.preventDefault(); setComposer(current => `/${suggestion.command} ${current}`); setSkillSuggestions([]); }} className="flex w-full items-start gap-3 border-b border-border px-3 py-2 text-left last:border-0 hover:bg-accent"><span className="mt-0.5 rounded border border-success/25 bg-success/10 px-1.5 py-0.5 text-[8.5px] uppercase text-success">installed</span><span className="min-w-0 flex-1"><b className="block truncate text-[11px] font-medium text-foreground">{suggestion.name}</b><small className="mt-0.5 block text-[9.5px] leading-4 text-muted-foreground">{suggestion.relevance} · {suggestion.source} · {suggestion.risk} risk · {suggestion.permissions.join(", ")}</small></span></button>)}</div>}
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
                        <span title={command.harness === "bridge" ? "Runs locally in Bridge" : "Provider-owned command"} className="shrink-0 text-[8.5px] uppercase tracking-[0.06em] text-muted-foreground border border-border rounded px-1 py-[1px]">{slashOwnershipBadge(command.harness)}</span>
                      </button>)}
                    </div>
                  </div>}
                  <ComposerContextStrip
                    workspaces={state.workspaces}
                    workspace={workspace ?? null}
                    worktree={worktreeOn}
                    locked={conversationStarted || forest === undefined}
                    branches={branchWorkspaceId === workspace?.id ? workspaceBranches : []}
                    currentBranch={branchWorkspaceId === workspace?.id ? workspaceBranchCurrent : workspace?.branch ?? null}
                    branchBusy={branchWorkspaceId === workspace?.id && branchBusy}
                    branchError={branchWorkspaceId === workspace?.id ? branchError : null}
                    onSelectWorkspace={id => { if (id === workspace?.id) return; void retargetWorkspace(id, worktreeOn); }}
                    onRequestBranches={() => { if (workspace) void requestWorkspaceBranches(workspace.id); }}
                    onSelectBranch={branch => { if (workspace) void switchWorkspaceBranch(workspace.id, branch); }}
                    onToggleWorktree={() => { if (!workspace) return; void retargetWorkspace(workspace.id, !worktreeOn); }}
                  />
                  {harnessShortcutOpen && <div className="u-glass-popover absolute left-4 right-4 sm:left-6 sm:right-6 bottom-full mb-2 z-20 rounded-2xl overflow-hidden flex flex-col max-h-[min(420px,55vh)]">
                    <div className="shrink-0 px-3 py-1.5 text-[9px] uppercase tracking-[0.12em] text-muted-foreground/70 border-b border-border flex items-center gap-2">
                      <span>Talk to a harness directly</span>
                      <span className="normal-case tracking-normal text-muted-foreground/50">{harnessShortcutMatches.length}</span>
                    </div>
                    <div ref={harnessShortcutListRef} className="min-h-0 flex-1 overflow-y-auto overscroll-contain" onWheel={e => e.stopPropagation()}>
                      {harnessShortcutMatches.map((adapter, index) => <button key={adapter.id} type="button" data-harness-shortcut-index={index} onMouseEnter={() => setHarnessShortcutIndex(index)} onMouseDown={e => { e.preventDefault(); applyHarnessShortcut(adapter); }} className={`w-full flex items-center gap-2 px-3 py-2 text-left transition-colors ${index === harnessShortcutIndex ? "bg-accent" : "hover:bg-accent"}`}>
                        <span className="font-mono text-[12px] text-foreground whitespace-nowrap">${adapter.id}</span>
                        <span className="flex-1 min-w-0 text-[11px] text-muted-foreground whitespace-nowrap overflow-hidden text-ellipsis">Starts a new {adapter.label} chat with what follows</span>
                      </button>)}
                    </div>
                  </div>}
                  <ComposerPill
                    layout="dock"
                    value={composer}
                    onChange={value => { setComposer(value); setSlashDismissed(false); setSlashIndex(0); setMentionDismissed(false); setMentionIndex(0); setHarnessShortcutDismissed(false); setHarnessShortcutIndex(0); }}
                    onSubmit={() => void sendPrompt()}
                    onKeyDown={onComposerKeyDown}
                    autocomplete={mentionOpen ? {
                      controls: "file-mention-listbox",
                      activeDescendant: `file-mention-option-${mentionIndex}`,
                    } : undefined}
                    suggestion={draftSuggestion?.suggestion}
                    onAcceptSuggestion={acceptSuggestion}
                    placeholder={isDirectChat ? "Ask Bridge…" : sessionConnected ? "Message…" : "Message…  (starts the agent)"}
                    disabled={!session}
                    working={!!session?.activeTurnId}
                    activeAction={activeAction}
                    onStop={session ? () => void bridgeApi.interruptTurn(session.id) : undefined}
                    inputRef={composerRef}
                    onPlusClick={() => void attachFile()}
                    trailing={session.kind === "direct" || session.kind === "orchestrator"
                      ? <ChatModelControl adapters={adapters} harness={session.harness} model={session.model ?? null} disabled={busy || turnActive} disabledReason={turnActive ? "Wait for the current response before switching models" : undefined} onChange={(harness, model) => void changeChatModel(harness, model)} compact roleLabel={session.kind === "orchestrator" ? "Orchestrator" : "Chat"} />
                      : <span className="inline-flex items-center gap-1 h-8 px-2.5 text-foreground/75 text-[13px] rounded-full">{harnessLabel(session.harness)}</span>}
                  />
                </div>}
              </div>
            </>}
            {hasRepo && workspace && visitedTabs.has("changes") && <div className={cn("absolute inset-0", activeTab !== "changes" && "hidden")}><ChangesPanel key={workspace.id} workspace={workspace}/></div>}
            {hasRepo && workspace && visitedTabs.has("code") && <div className={cn("absolute inset-0", activeTab !== "code" && "hidden")}>{/* Keyed on the workspace: these panels hold open buffers and relative
                  paths, and neither survives a change of tree. Without it a save
                  would aim the old path at the new workspace. */}
              <Suspense fallback={<PanelLoading label="Opening editor…"/>}><CodePanel key={workspace.id} workspaceId={workspace.id} visible={activeTab === "code"} onSaved={() => void refreshWorkspaceStats(workspace.id)}/></Suspense></div>}
            {hasRepo && workspace && activeTab === "terminal" && <div className="absolute inset-0"><Suspense fallback={<PanelLoading label="Opening terminal…"/>}><TerminalPane workspaceId={workspace.id}/></Suspense></div>}
          </div>
          {browserOpen && <BrowserSurface onClose={() => setBrowserOpen(false)} onError={setError} />}
        </section>
      </> : <Welcome
        adapters={adapters}
        modelSetup={modelSetup}
        canStartChat={adaptersReady}
        busy={busy}
        workspaces={state.workspaces}
        workspace={welcomeWorkspace}
        worktree={false}
        branches={branchWorkspaceId === welcomeWorkspace?.id ? workspaceBranches : []}
        currentBranch={branchWorkspaceId === welcomeWorkspace?.id ? workspaceBranchCurrent : welcomeWorkspace?.branch ?? null}
        branchBusy={branchWorkspaceId === welcomeWorkspace?.id && branchBusy}
        branchError={branchWorkspaceId === welcomeWorkspace?.id ? branchError : null}
        onSelectWorkspace={id => { writeLastWorkspaceId(id); setWelcomeWorkspaceId(id); }}
        onRequestBranches={() => { if (welcomeWorkspace) void requestWorkspaceBranches(welcomeWorkspace.id); }}
        onSelectBranch={branch => { if (welcomeWorkspace) void switchWorkspaceBranch(welcomeWorkspace.id, branch); }}
        onToggleWorktree={draft => {
          if (draft) pendingWelcomeMessageRef.current = draft;
          if (resolvedWelcomeWorkspaceId) void newWorkspaceSession(true, resolvedWelcomeWorkspaceId);
        }}
        onStartChat={text => void startChatOrShortcut(text)}
        onNewWorkspace={() => { setTitle(""); setModal("workspace"); }}
      />}
    </main>
    </div>
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
    <MemoryDialog open={modal === "memory"} initialBody={memoryDraft} adapters={adapters} onClose={() => { setModal(null); setMemoryDraft(null); }} onError={setError} />
  </div>;
}

function PanelLoading({ label }: { label: string }) {
  return <div role="status" className="absolute inset-0 grid place-items-center text-xs text-muted-foreground">{label}</div>;
}

function localAccountName(...paths: Array<string | null | undefined>): string {
  for (const path of paths) {
    if (!path) continue;
    const macOrLinux = path.match(/^\/(?:Users|home)\/([^/]+)/);
    if (macOrLinux?.[1]) return macOrLinux[1];
    const windows = path.match(/^[A-Za-z]:\\Users\\([^\\]+)/i);
    if (windows?.[1]) return windows[1];
  }
  return "Local user";
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
  // Once a file has been edited the editor stays mounted — hidden behind the
  // diff, and kept alive through a collapse while it still holds unsaved text.
  // Unmounting it was the same data loss the tab switch used to cause.
  const [everEdited, setEverEdited] = useState(false);
  const [dirty, setDirty] = useState(false);
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
      {dirty && <span className="shrink-0 text-[10.5px] text-warning" title="This file has unsaved edits in the inline editor">unsaved</span>}
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
    {(expanded || dirty) && <div className={cn("border-t border-border bg-code", !expanded && "hidden")}>
      {!file.binary && <div className="flex items-center gap-1 border-b border-border px-2 py-1">
        {(["diff", "edit"] as const).map(option => <button
          key={option}
          type="button"
          onClick={() => { setMode(option); if (option === "edit") setEverEdited(true); }}
          aria-pressed={mode === option}
          className={cn(
            "h-[20px] rounded-[5px] px-2 text-[10.5px] capitalize transition-colors",
            mode === option ? "bg-accent text-foreground" : "text-muted-foreground hover:text-foreground",
          )}
        >{option}</button>)}
      </div>}
      <div className={cn(mode === "edit" && !file.binary && "hidden")}>
        <FileDiffView patch={file.patch} binary={file.binary} path={file.path} />
      </div>
      {everEdited && !file.binary && <div className={cn(mode !== "edit" && "hidden")}>
        <Suspense fallback={<div className="px-3.5 py-4 text-[11.5px] text-muted-foreground">Opening editor…</div>}>
          <InlineFileEditor workspaceId={workspaceId} path={file.path} onDirtyChange={setDirty} onSaved={onSaved} />
        </Suspense>
      </div>}
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

function Welcome({ adapters, modelSetup, busy, canStartChat, onStartChat, onNewWorkspace, workspaces, workspace, worktree, branches, currentBranch, branchBusy, branchError, onSelectWorkspace, onRequestBranches, onSelectBranch, onToggleWorktree }: {
  adapters: import("./types").AdapterDescriptor[];
  modelSetup: ModelSetupState;
  busy: boolean;
  canStartChat: boolean;
  onStartChat: (text?: string) => void;
  onNewWorkspace: () => void;
  workspaces: Workspace[];
  workspace: Workspace | null;
  worktree: boolean;
  branches: string[];
  currentBranch: string | null;
  branchBusy: boolean;
  branchError: string | null;
  onSelectWorkspace: (id: string) => void;
  onRequestBranches: () => void;
  onSelectBranch: (branch: string) => void;
  onToggleWorktree: (draft?: string) => void;
}) {
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
    {workspaces.length > 0 && <ComposerContextStrip
      workspaces={workspaces}
      workspace={workspace}
      worktree={worktree}
      locked={busy}
      branches={branches}
      currentBranch={currentBranch}
      branchBusy={branchBusy}
      branchError={branchError}
      onSelectWorkspace={onSelectWorkspace}
      onRequestBranches={onRequestBranches}
      onSelectBranch={onSelectBranch}
      onToggleWorktree={() => onToggleWorktree(draft.trim() || undefined)}
    />}
    <ComposerPill
      layout="hero"
      value={draft}
      onChange={setDraft}
      onSubmit={submit}
      onKeyDown={e => { if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) { e.preventDefault(); submit(); } }}
      placeholder={canStartChat ? "Ask Bridge…" : "Install or sign in to a model adapter…"}
      disabled={busy || !canStartChat}
      // There is no conversation or folder here yet, so there is nothing to
      // attach to. This surface keeps the structural action — and says so.
      plusLabel="New workspace"
      onPlusClick={onNewWorkspace}
      trailing={<WelcomeModelBadge adapters={adapters} modelSetup={modelSetup} />}
    />
    <p className="mt-6 max-w-md text-[13px] leading-relaxed text-muted-foreground">{greeting.hint}</p>
  </div>;
}
function CommandPalette({ workspaces, onChoose }: { workspaces: Workspace[]; onChoose: (id:string)=>void }) { return <><InputGroup className="border-b border-border rounded-none border-x-0 border-t-0 shadow-none"><InputGroupInput autoFocus placeholder="Search workspaces and actions…" /><InputGroupAddon><Search size={17} aria-hidden="true" /></InputGroupAddon></InputGroup><div className="p-[9px]"><label className="block p-[5px_9px_7px] text-muted-foreground/65 text-[10px] font-semibold tracking-[0.09em]">WORKSPACES</label>{workspaces.map(w => <Button type="button" key={w.id} variant="ghost" className="w-full h-[44px] rounded-md justify-start px-2.5" onClick={() => onChoose(w.id)}><StatusDot status={w.status}/><span className="flex flex-col gap-[3px] flex-1 text-left"><b className="text-[12.5px] font-medium">{w.title}</b><small className="text-[10.5px] text-muted-foreground">{w.city} · {w.branch}</small></span><Kbd className="font-mono text-muted-foreground/65 border border-border rounded px-1 py-[1px] text-[10px]">↵</Kbd></Button>)}</div><div className="h-[32px] border-t border-border flex items-center gap-[14px] px-[13px] text-muted-foreground/65 text-[10.5px]"><span>↑↓ navigate</span><span>esc close</span></div></>; }
