import { type ClipboardEvent, lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { QueryClientProvider } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";
import { appendFileMention, applyFileMention as insertFileMention, fileMentionQuery } from "./fileMentions";
import { agentMentionQuery, agentShortcutCandidates, parseAgentMention, type AgentShortcutCandidate } from "./agentMention";
import { harnessShortcutQuery, parseHarnessShortcut } from "./harnessShortcut";
import { Activity, Archive, Bot, Braces, CircleDot, Clock3, Code2, FileCode2, FileDiff, FileText, GitCommitHorizontal, GitPullRequest, Inbox, LoaderCircle, MessageSquareText, Monitor, Play, Plus, Search, TerminalSquare, X } from "lucide-react";
import { bridgeApi } from "./api";
import { type ComposerAttachment, imageFilesFromClipboard, isPasteTooLarge, mediaTypeOf, readAsDataUri } from "./pasteAttachments";
import { openExternalUrl } from "./externalLinks";
import { appendAgentEventBatch } from "./agentEvents";
import type { AgentDefinition, AgentEvent, ApprovalDecision, BridgeState, CapabilitySuggestion, Harness, PermissionPolicy, Project, Session, SessionForestSnapshot, SessionStatus, SkillProvider, WorkerRepositoryBinding, Workspace } from "./types";
import { AgentConversation } from "./components/AgentConversation";
import { BridgeSidebar } from "./components/BridgeSidebar";
import { HealthWarnings } from "./components/HealthWarnings";
import { ComposerContextStrip } from "./components/ComposerContextStrip";
import { ProjectsScreen } from "./components/ProjectsScreen";
import type { QuestionAction, SuggestCompletionResult, SuggestionSettingsSnapshot, WorkFactAction, WorkTask } from "./protocol/generated/protocol";
import type { WorkActionOutcome } from "./components/WorkView";
import { taskRoute, type TaskAction } from "./components/workTasks";
import { isHiddenSession } from "./components/sidebarChats";
import { SessionToolbar } from "./components/SessionToolbar";
import { ChatModelControl, modelDisplayName } from "./components/ChatModelControl";
export { ChatModelControl };
import { SessionDock, type DockPaneDescriptor } from "./components/SessionDock";
import { AsideChat } from "./components/AsideChat";
import { ChangesPanel } from "./components/ChangesPanel";
import { GitHubPane } from "./components/GitHubPane";
import { GithubToasts, type CiToast } from "./components/GithubToasts";
import { ciToastKey, jumpFallbackHint } from "./githubSurface";
import { TranscriptPane, TRANSCRIPT_PAGE_SIZE } from "./components/TranscriptPane";
import type { BrowserSupervision } from "./components/BrowserSurface";
import type { TerminalActivity } from "./components/TerminalPane";
import { TasksPane } from "./components/TasksPane";
import { workerStatus } from "./components/workerStatus";
import type { HunkRange } from "./components/DiffView";
import { DOCK_PANES, DOCK_SHEET_THRESHOLD, useDockLayout } from "./dockLayout";
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
import { projectSessionConversation, reduceConversation, undeliveredPending } from "./conversation";
import { resolveProfileOption, shouldRequireModelSetup } from "./modelProfiles";
import { resolveAsideModel } from "./asideModel";
import { pickGreeting } from "./greetings";
import { useThemePreference } from "./theme";
import { recordPlace, type AppPlace, type AppView } from "./navigationHistory";
import { readLastWorkspaceId, resolveNewChatWorkspaceId, writeLastWorkspaceId } from "./lastWorkspace";
import { FLUSH_WINDOW_EVENT, isFlushWindowDocument, notifyLayoutFullscreen, setLayoutFullscreenDocument } from "./windowChrome";
import { isTypingTarget, matchShortcut, MENU_COMMAND_EVENT, type CommandId } from "./keymap";
import { ShortcutsSheet } from "./components/ShortcutsSheet";
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
import { createBridgeQueryClient } from "./queryClient";
import { useUiStore } from "./uiStore";
import { useBridgeServerState } from "./serverState";

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

// #350: New chat opens an *unstarted* draft — no session row, no scratch dir, no
// adapter process — until the first message is submitted. The draft captures the
// choices made before sending. Harness/model are snapshotted at open time so the
// carry-over from the current direct chat survives deselecting it.
type NewChatDraft = {
  harness: Harness;
  model: string | null;
  workspaceId: string | null;
  createWorktree: boolean;
  carryFromSessionId?: string;
};

type AsidePhase = "creating" | "handing_off" | "ready" | "switching" | "failed" | "cancelled" | "closed";
type AsideLifecycle = {
  sourceSessionId: string;
  sessionId?: string;
  phase: AsidePhase;
  handoffStatus?: string;
  fidelity?: string;
  error?: string;
  recoveryDraft?: string;
};

export function App() {
  const [queryClient] = useState(createBridgeQueryClient);
  return <QueryClientProvider client={queryClient}><AppContent /></QueryClientProvider>;
}

function AppContent() {
  const modal = useUiStore(state => state.modal);
  const openModal = useUiStore(state => state.openModal);
  const closeModal = useUiStore(state => state.closeModal);
  const {
    health, healthError, modelSetup, modelSetupError, workBoard,
    workBoardQueryError, refetchWorkBoard, acceptModelSetup, invalidateHealth,
  } = useBridgeServerState();
  const [state, setState] = useState<BridgeState>(emptyState);
  const [agentEvents, setAgentEvents] = useState<AgentEvent[]>([]);
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
  const [workBriefingError, setWorkBriefingError] = useState<string>();
  const [navOpen, setNavOpen] = useState(false);
  // Two ways to look at the workspace: the classic single-session view, or the
  // Mission Control grid where every live agent is its own window at once.
  const [paradigm, setParadigm] = useState<"single" | "grid">("single");
  const [dockSectionWidth, setDockSectionWidth] = useState(1280);
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
  const [agentShortcutIndex, setAgentShortcutIndex] = useState(0);
  const [agentShortcutDismissed, setAgentShortcutDismissed] = useState(false);
  const [configuredAgents, setConfiguredAgents] = useState<AgentDefinition[]>([]);
  const [skillSuggestions, setSkillSuggestions] = useState<CapabilitySuggestion[]>([]);
  const [busy, setBusy] = useState(false);
  const [browserSupervision, setBrowserSupervision] = useState<BrowserSupervision>();
  const [terminalActivity, setTerminalActivity] = useState<TerminalActivity>();
  const [acknowledgedTasks, setAcknowledgedTasks] = useState<Set<string>>(() => new Set());
  const [recallOpen, setRecallOpen] = useState(false);
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  const [highlightEntryId, setHighlightEntryId] = useState<string | null>(null);
  const [error, setError] = useState<string>();
  const [forest, setForest] = useState<SessionForestSnapshot>();
  // Completion blocks while a child's changes live only in its own worktree, so
  // the user must be able to see and resolve that here — otherwise the session
  // waits forever with no visible cause.
  const [pendingAdoptions, setPendingAdoptions] = useState<WorkerRepositoryBinding[]>([]);
  const [pending, setPending] = useState<{ key: string; sessionId: string; text: string; delivery?: "steered" | "queued"; attachment?: string }[]>([]);
  // Image attachments pasted into the composer, waiting to ride the next send.
  // Cleared on success, restored on failure — a refused send must not eat the
  // user's clipboard work.
  const [attachments, setAttachments] = useState<ComposerAttachment[]>([]);
  /** A model switch in flight, so the conversation can narrate it honestly. */
  const [modelSwitch, setModelSwitch] = useState<{ sessionId: string; harness: string; label: string } | null>(null);
  /** Exact source/aside ownership and lifecycle - see `openHarnessShortcut`. */
  const [asideLifecycle, setAsideLifecycle] = useState<AsideLifecycle>();
  // The composer's inline typeahead. Loaded once and kept fresh by Settings'
  // own save path (`onSuggestionSettingsChange`) — off by default, so no
  // request fires until the user opts in.
  const [suggestionSettings, setSuggestionSettings] = useState<SuggestionSettingsSnapshot>();
  const [draftSuggestion, setDraftSuggestion] = useState<SuggestCompletionResult>();
  const suggestionGeneration = useRef(0);
  // Shown once per fallback episode, not on every debounce firing while the
  // configured model stays in cooldown.
  const [fallbackNotice, setFallbackNotice] = useState<string>();
  const [agentDispatchNotice, setAgentDispatchNotice] = useState<string>();
  const fallbackNoticeShownRef = useRef(false);
  const [usageByProvider, setUsageByProvider] = useState<Partial<Record<UsageProvider, UsageSnapshot>>>({});
  const [usageSamples, setUsageSamples] = useState<Partial<Record<UsageProvider, UsageRateSample[]>>>({});
  const startedRef = useRef<Set<string>>(new Set());
  // The first message of a just-created chat, tagged with its target session id so
  // the delivery effect can only ever hand it to that chat — never to a session that
  // became active while the create awaited (#350).
  const pendingWelcomeMessageRef = useRef<{ sessionId: string; text: string; attachments: ComposerAttachment[] } | null>(null);
  const forestKeyRef = useRef("");
  const agentEventQueueRef = useRef<AgentEvent[]>([]);
  const agentEventTimerRef = useRef<number | undefined>(undefined);
  const browserSessionRef = useRef<string>();
  const workQueryError = workBoardQueryError ? errorMessage(workBoardQueryError) : undefined;
  const workError = workBoard === undefined ? workQueryError : undefined;
  const workRefreshError = workBoard === undefined ? undefined : workBriefingError ?? workQueryError;

  const reload = useCallback(async () => {
    const [nextState, config] = await Promise.all([bridgeApi.state(), bridgeApi.configState()]);
    setState(nextState);
    // Re-read with the state it was published alongside: `save_permission_policy`
    // publishes StateChanged precisely so the badge repaints, and another window
    // flipping the switch has to reach this one too.
    setPermissionPolicy(config.permissionPolicy);
    const enabledHarnesses = new Set(config.harnesses.filter(harness => harness.enabled).map(harness => harness.id));
    setConfiguredAgents(config.agents.filter(agent => enabledHarnesses.has(agent.harness)));
  }, []);
  useEffect(() => {
    void reload().catch(value => setError(errorMessage(value)));
    let offState: (() => void) | undefined;
    let offAgent: (() => void) | undefined;
    let offUsage: (() => void) | undefined;
    let offAdapters: (() => void) | undefined;
    let offProviderLogin: (() => void) | undefined;
    let active = true;
    const reloadHealth = invalidateHealth;
    void bridgeApi.onStateChanged(reload).then(fn => offState = fn);
    // The provider-login flow runs as an ordinary PTY under the "provider-login"
    // pseudo-workspace; when the vendor process exits, re-read health so a
    // completed sign-in populates the widget without a restart.
    void bridgeApi.onTerminalExited(exit => {
      if (exit.sessionId === "provider-login") reloadHealth();
    }).then(fn => offProviderLogin = fn);
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
      offState?.(); offAgent?.(); offUsage?.(); offAdapters?.(); offProviderLogin?.();
      if (agentEventTimerRef.current !== undefined) window.clearTimeout(agentEventTimerRef.current);
      agentEventTimerRef.current = undefined;
      agentEventQueueRef.current = [];
    };
  }, [invalidateHealth, reload]);
  useThemePreference();
  useEffect(() => { setNavOpen(false); setRecallOpen(false); setHighlightEntryId(null); }, [view, selectedSessionId]);
  // Navigating away from an unstarted draft discards it silently — nothing was
  // created, so there is nothing to clean up (#350). A draft only lives on the
  // single-chat empty surface (workspace view, no session selected).
  useEffect(() => {
    if (selectedSessionId || view !== "workspace" || paradigm !== "single") setNewChatDraft(null);
  }, [selectedSessionId, view, paradigm]);

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
  useEffect(() => { setAgentDispatchNotice(undefined); }, [session?.id]);

  // The dock is a workspace possession: width, active pane, and expand state
  // belong to the tree being worked on, so direct chats key by session instead.
  const dockKey = workspace?.id ?? session?.id;
  const [dock, dispatchDock] = useDockLayout(dockKey);
  const dockRef = useRef(dock);
  dockRef.current = dock;
  const dockSheet = dockSectionWidth < DOCK_SHEET_THRESHOLD;
  // A callback ref, so the observer is keyed to the element itself: the
  // section unmounts and remounts on view and paradigm switches while the
  // session id stays put, and an effect keyed on the id would keep watching
  // the detached node — freezing the width and with it the sheet threshold.
  const dockSectionObserver = useRef<ResizeObserver | null>(null);
  const dockSectionRef = useCallback((element: HTMLElement | null) => {
    dockSectionObserver.current?.disconnect();
    dockSectionObserver.current = null;
    if (!element || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(entries => {
      const width = entries[0]?.contentRect.width;
      if (typeof width === "number" && width > 0) setDockSectionWidth(width);
    });
    observer.observe(element);
    dockSectionObserver.current = observer;
  }, []);
  const dockTaskBadge = useMemo(() => {
    const statuses = (forest?.workerRuntimes ?? []).flatMap(runtime => {
      const workerSession = visibleSessions.find(item => item.id === runtime.sessionId);
      return workerSession ? [{ id: runtime.sessionId, status: workerStatus(workerSession, runtime) }] : [];
    });
    const running = statuses.filter(item => item.status.tone === "working").length + (terminalActivity?.running ?? 0);
    const attention = statuses.some(item => (item.status.tone === "failed" || item.status.tone === "stalled") && !acknowledgedTasks.has(item.id));
    return { running, attention };
  }, [forest?.workerRuntimes, visibleSessions, terminalActivity?.running, acknowledgedTasks]);
  const dockPanes: DockPaneDescriptor[] = [
    { id: "changes", label: "Changes", icon: FileCode2, available: hasRepo && !!workspace, unavailableReason: "Changes needs a repository. This chat has no worktree to diff.", badge: workspace?.dirtyFiles || undefined },
    { id: "code", label: "Code", icon: Code2, available: hasRepo && !!workspace, unavailableReason: "Code needs a repository. This chat has no worktree to read files from." },
    { id: "terminal", label: "Terminal", icon: TerminalSquare, available: hasRepo && !!workspace, unavailableReason: "The terminal needs a repository. This chat has no worktree to run a shell in.", badge: terminalActivity && terminalActivity.running > 1 ? terminalActivity.running : undefined, alert: terminalActivity?.attention || undefined },
    { id: "browser", label: "Browser", icon: Monitor, available: true, alert: browserSupervision?.attention || undefined },
    { id: "transcript", label: "Transcript", icon: Braces, available: true },
    { id: "tasks", label: "Tasks", icon: Activity, available: true, badge: dockTaskBadge.running || undefined, alert: dockTaskBadge.attention || undefined },
    { id: "github", label: "GitHub", icon: GitPullRequest, available: hasRepo && !!workspace, unavailableReason: "GitHub needs a repository. This chat has no worktree with a remote." },
  ];
  const dockExpandedVisible = dock.open && dock.expanded && !fullscreen;

  // Cross-pane intents. Quoting names what a message is about instead of
  // describing it; revealing hands a file from the diff to the editor. The
  // nonce distinguishes "open it again" from a re-render.
  const [codeReveal, setCodeReveal] = useState<{ path: string; line?: number; nonce: number }>();
  const revealNonce = useRef(0);
  // The intent names a path in one worktree; a remounted CodePanel in another
  // workspace resets its nonce guard and would honour it against the wrong
  // tree. Changing workspaces retires the request.
  useEffect(() => {
    setCodeReveal(undefined);
  }, [workspace?.id]);
  function quoteToComposer(path: string, range?: HunkRange) {
    setComposer(current => {
      const mentioned = appendFileMention(current, path);
      return range ? `${mentioned}lines ${range.start}-${range.end} ` : mentioned;
    });
    composerRef.current?.focus();
  }
  function revealEntryInConversation(entryId: string) {
    // The conversation sits beside the dock, so reveal scrolls and highlights
    // rather than navigates — the same jump recall search uses. An expanded
    // pane steps aside first: scrollIntoView on a hidden column is a no-op.
    if (dockRef.current.expanded) dispatchDock({ type: "toggle-expanded" });
    setHighlightEntryId(entryId);
    requestAnimationFrame(() => {
      document.getElementById(`forest-entry-${entryId}`)?.scrollIntoView({ behavior: "smooth", block: "center" });
    });
    // The highlight is a pointer, not a state: it fades once it has done its
    // job, instead of marking the entry until the next navigation.
    window.setTimeout(() => {
      setHighlightEntryId(current => current === entryId ? null : current);
    }, 3000);
  }
  function openFileInDock(path: string, line?: number) {
    revealNonce.current += 1;
    setCodeReveal({ path, line, nonce: revealNonce.current });
    dispatchDock({ type: "open-pane", pane: "code" });
  }

  // ── GitHub surface glue ────────────────────────────────────────────────────
  // Deep links into the GitHub dock pane (sidebar rows, CI toasts), the
  // CI-finished notification stack, and jump-to-diff from a review comment.
  const githubIntentNonce = useRef(0);
  const [githubIntent, setGithubIntent] = useState<{ number: number; nonce: number }>();
  const [githubToasts, setGithubToasts] = useState<CiToast[]>([]);
  const [githubJumpHint, setGithubJumpHint] = useState<string>();

  function openPullRequestPane(number: number) {
    githubIntentNonce.current += 1;
    setGithubIntent({ number, nonce: githubIntentNonce.current });
    setView("workspace");
    setParadigm("single");
    dispatchDock({ type: "open-pane", pane: "github" });
  }

  function openCiToast(toast: CiToast) {
    setGithubToasts(current => current.filter(item => item.key !== toast.key));
    const payload = toast.payload;
    if (workspace?.id !== payload.workspaceId) {
      // The PR lives in another workspace; land in one of its chats first so
      // the pane reads the right repo.
      const target = state.sessions.find(candidate => candidate.workspaceId === payload.workspaceId && !candidate.parentSessionId);
      if (!target) {
        setError(`CI finished on ${payload.headBranch}, but its workspace has no open chat to show it in.`);
        return;
      }
      openSession(target.id);
    }
    openPullRequestPane(payload.number);
  }

  useEffect(() => {
    let active = true;
    let off: (() => void) | undefined;
    void bridgeApi.onGithubCiFinished(payload => {
      if (!active) return;
      const key = ciToastKey(payload);
      // The poller dedups per terminal check set; this guard only keeps a
      // re-delivered payload from stacking the same card twice.
      setGithubToasts(current => current.some(item => item.key === key) ? current : [...current.slice(-3), { key, payload }]);
    }).then(unlisten => { if (active) off = unlisten; else unlisten(); });
    return () => { active = false; off?.(); };
  }, []);

  // The fallback hint is a pointer, not a state — it fades on its own.
  useEffect(() => {
    if (!githubJumpHint) return;
    const timer = window.setTimeout(() => setGithubJumpHint(undefined), 8000);
    return () => window.clearTimeout(timer);
  }, [githubJumpHint]);

  /** Jump-to-diff from a review comment: open the editor at the commented
   * file/line. When the PR head branch is not what this workspace has checked
   * out, the file still opens (read it, don't edit it) with a hint saying so. */
  function jumpToReviewComment(path: string, line: number | undefined, headBranch: string) {
    openFileInDock(path, line);
    setGithubJumpHint(jumpFallbackHint(workspace?.branch ?? null, headBranch) ?? undefined);
  }
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
  const asideSession = useMemo(() => {
    if (!asideLifecycle || asideLifecycle.sourceSessionId !== session?.id) return undefined;
    return state.sessions.find(candidate => candidate.id === asideLifecycle.sessionId);
  }, [asideLifecycle, session?.id, state.sessions]);
  const asidePending = useMemo(
    () => pending.filter(item => item.sessionId === asideLifecycle?.sessionId).map(item => item.text),
    [pending, asideLifecycle?.sessionId],
  );
  const pendingForSession = useMemo(() => pending.filter(p => p.sessionId === session?.id).map(p => p.text), [pending, session?.id]);
  const pendingForSessionAttachments = useMemo(
    () => pending.filter(p => p.sessionId === session?.id && p.attachment).map(p => p.attachment as string),
    [pending, session?.id],
  );
  const conversationStarted = useMemo(() => {
    if (!session) return false;
    if (session.activeTurnId) return true;
    if (pendingForSession.length > 0) return true;
    const durable = forest?.entries?.length ? projectSessionConversation(forest.entries, forest.head?.activeEntryId ?? null) : [];
    return durable.some(item => item.type === "message" && item.role === "user");
  }, [forest, pendingForSession.length, session]);
  const [worktreeOn, setWorktreeOn] = useState(false);
  const [welcomeWorkspaceId, setWelcomeWorkspaceId] = useState<string | null>(null);
  // The pending unstarted new chat, if any. Non-null ⇒ the empty-state surface is a
  // draft: the choices are held here and the session is created on first submit.
  const [newChatDraft, setNewChatDraft] = useState<NewChatDraft | null>(null);
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
  // #agent shortcut: leading-only, and only while the target token is being
  // typed. The host resolves the selected token again from persisted config;
  // these rows are discovery, never execution authority.
  const agentShortcutQueryValue = agentMentionQuery(composer);
  const agentShortcutMatches = useMemo(() => {
    if (agentShortcutQueryValue == null || !session?.workspaceId) return [];
    return agentShortcutCandidates(configuredAgents, agentShortcutQueryValue);
  }, [agentShortcutQueryValue, configuredAgents, session?.workspaceId]);
  const agentShortcutOpen = agentShortcutQueryValue != null && agentShortcutMatches.length > 0 && !agentShortcutDismissed;
  const agentShortcutListRef = useRef<HTMLDivElement>(null);
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
  // overwrite a newer one. No request fires with the toggle off, no session,
  // or an empty draft.
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
    if (!agentShortcutOpen) return;
    setAgentShortcutIndex(index => Math.min(index, Math.max(0, agentShortcutMatches.length - 1)));
  }, [agentShortcutOpen, agentShortcutMatches.length]);

  useEffect(() => {
    if (!agentShortcutOpen) return;
    const root = agentShortcutListRef.current;
    if (!root) return;
    const active = root.querySelector<HTMLElement>(`[data-agent-shortcut-index="${agentShortcutIndex}"]`);
    active?.scrollIntoView({ block: "nearest" });
  }, [agentShortcutOpen, agentShortcutIndex]);

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

  // Drop an optimistic message once its real user turn arrives from the
  // backend. Judged per row in the row's own session and re-run on the global
  // stream: an aside's pending rows used to be checked against the *selected*
  // session's slice, so they never reconciled and the aside's startup row
  // counted forever under an already-answered reply.
  useEffect(() => {
    setPending(current => {
      const durable = forest?.entries?.length ? projectSessionConversation(forest.entries, forest.head?.activeEntryId ?? null) : [];
      const durableUserTexts = new Set(durable.filter(item => item.type === "message" && item.role === "user").map(item => item.text.trim()));
      return undeliveredPending(current, agentEvents, { sessionId: session?.id, durableUserTexts });
    });
  }, [agentEvents, forest, session?.id]);

  // Load available slash commands + skills from signed-in providers.
  useEffect(() => { void bridgeApi.listSlashCommands().then(setSlashCommands).catch(() => undefined); }, [adaptersReady]);

  // Always land on the Agent tab: focusing a session (especially a blocked
  // worker from Mission Control) must reveal its conversation and approval card,
  // not whatever tab — Changes/Terminal — happened to be open before.
  function openSession(id: string) {
    setView("workspace");
    setParadigm("single");
    setNewChatDraft(null);
    setSelectedSessionId(id);
    setExpandedWorkerId(undefined);
    setAsideLifecycle(current => current?.sourceSessionId === id ? current : undefined);
    const opened = state.sessions.find(candidate => candidate.id === id);
    if (opened?.workspaceId) writeLastWorkspaceId(opened.workspaceId);
  }

  const openWorkBoard = useCallback(() => {
    setView("work");
    setParadigm("single");
    setWorkBriefingError(undefined);
    void refetchWorkBoard();
  }, [refetchWorkBoard]);

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
      setWorkBriefingError(undefined);
      await refetchWorkBoard();
      return { ok: true };
    } catch (error) {
      return { ok: false, reason: errorMessage(error) };
    }
  }, [adapters, refetchWorkBoard]);

  const toggleWorkTaskPin = useCallback(async (task: WorkTask): Promise<WorkActionOutcome> => {
    try {
      await bridgeApi.workTaskPin(task.id, !task.pinned);
      setWorkBriefingError(undefined);
      await refetchWorkBoard();
      return { ok: true };
    } catch (error) {
      return { ok: false, reason: errorMessage(error) };
    }
  }, [refetchWorkBoard]);

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
        setWorkBriefingError(receipt.detail ?? receipt.code ?? "the briefing was refused");
      } else if (trigger === "manual") {
        setWorkBriefingError(undefined);
      }
    } catch (error) {
      if (trigger === "manual") setWorkBriefingError(errorMessage(error));
    }
    void refetchWorkBoard();
  }, [refetchWorkBoard]);

  // The opt-in focus trigger. Gated on the stored settings the board carries, so
  // a user who never opted in gets no background model run from switching apps.
  useEffect(() => {
    const onFocus = () => {
      if (!workBoard?.settings.refreshOnFocus) return;
      void runWorkBriefing("focus");
    };
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [runWorkBriefing, workBoard?.settings.refreshOnFocus]);

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
          setWorkBriefingError(undefined);
          await refetchWorkBoard();
          return { ok: true };
        case "refreshBaseObservation":
          // A user-triggered reading is written through to the cache the board reads,
          // so re-measuring is the same call the warning offers.
          await bridgeApi.workspaceBaseDivergence(action.sessionId, false);
          setWorkBriefingError(undefined);
          await refetchWorkBoard();
          return { ok: true };
      }
    } catch (error) {
      return { ok: false, reason: errorMessage(error) };
    }
  }, [refetchWorkBoard]);
  // Preserve the current direct chat's harness/model so switching to OpenCode also
  // changes the next-chat default; otherwise fall back to the configured standard
  // profile. Snapshotted at draft-open time, while the session being left is still
  // selected, so the carry-over survives deselecting it.
  function resolveDraftHarnessModel(harnessOverride?: import("./types").AdapterDescriptor): { harness: Harness; model: string | null } {
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
    return { harness, model };
  }

  // The single create-on-submit path (#350). Turns a draft into a real session —
  // workspace orchestrator or non-workspace direct chat — carries the handoff brief,
  // then hands the first message to the pending-message effect on selection. Guarded
  // by `newChatPendingRef` so a fast second submit cannot produce two sessions.
  async function submitNewChatDraft(
    draft: NewChatDraft,
    message?: string,
    alreadyLocked = false,
    initialAttachments: ComposerAttachment[] = [],
  ): Promise<string | undefined> {
    if (!alreadyLocked) {
      if (newChatPendingRef.current) return;
      newChatPendingRef.current = true;
    }
    try {
      if (!adaptersReady) { setError("No model adapter is available. Install or sign in to Codex, Claude, or OpenCode, then retry model setup."); return; }
      const text = message?.trim() ?? "";
      // A chat comes into existence only when it has something in it (#350): an empty
      // submit is a no-op, never a zero-entry placeholder row. The draft stays open.
      if (!text && initialAttachments.length === 0) return undefined;
      setBusy(true); setError(undefined);
      try {
        // create_chat takes harness/model directly; create_workspace_session doesn't,
        // so a workspace orchestrator is aligned to the draft's chosen model right
        // after creation — the model picked on the draft is the model it starts with.
        let next = draft.workspaceId
          ? await bridgeApi.createWorkspaceSession(draft.workspaceId, draft.createWorktree)
          : await bridgeApi.createChat(draft.harness, draft.model, null);
        let created = draft.workspaceId
          ? [...next.sessions].reverse().find(s => !s.parentSessionId && s.workspaceId === draft.workspaceId)
          : [...next.sessions].reverse().find(s => !s.parentSessionId && !s.workspaceId);
        if (created && draft.workspaceId && (created.harness !== draft.harness || (created.model ?? null) !== draft.model)) {
          try {
            next = await bridgeApi.updateChatModel(created.id, draft.harness, draft.model);
            created = next.sessions.find(s => s.id === created!.id) ?? created;
          } catch { /* keep the orchestrator's default model rather than fail the chat */ }
        }
        // Carry before selecting: the welcome-message effect fires on session-id
        // change, and the brief must be in the forest before the first cold start
        // compiles its prompt.
        if (created && draft.carryFromSessionId && draft.carryFromSessionId !== created.id) {
          try { await bridgeApi.carrySessionHandoff(created.id, draft.carryFromSessionId); }
          catch { /* context carry is best-effort; the chat starts regardless */ }
        }
        if (draft.workspaceId) writeLastWorkspaceId(draft.workspaceId);
        // Tag the message with the created session so it can only land there, even if
        // another session became active while the create awaited.
        if (created) pendingWelcomeMessageRef.current = { sessionId: created.id, text, attachments: initialAttachments };
        setState(next);
        setNewChatDraft(null);
        if (created) {
          if (draft.workspaceId) worktreeBySessionRef.current.set(created.id, draft.createWorktree);
          setSelectedSessionId(created.id);
        }
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

  // Open an unstarted draft — no session row is created (#350). This is the
  // non-workspace path: the `$harness` shortcut and the no-workspace fallback.
  // A pinned harness ($harness) always carries an `initialMessage`, so it creates on
  // send immediately, exactly like a submit; otherwise the draft surface just opens.
  async function openNewChat(initialMessage?: string, harnessOverride?: import("./types").AdapterDescriptor, alreadyLocked = false, carryFromSessionId?: string): Promise<string | undefined> {
    if (!adaptersReady) { setError("No model adapter is available. Install or sign in to Codex, Claude, or OpenCode, then retry model setup."); return; }
    const { harness, model } = resolveDraftHarnessModel(harnessOverride);
    const draft: NewChatDraft = { harness, model, workspaceId: null, createWorktree: false, carryFromSessionId };
    const message = initialMessage?.trim();
    if (message) return submitNewChatDraft(draft, message, alreadyLocked);
    setView("workspace"); setParadigm("single");
    setNewChatDraft(draft);
    setSelectedSessionId(undefined);
    return undefined;
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
      setError(`${adapter.label} isn't available${adapter.unavailableReason ? `: ${adapter.unavailableReason}` : ""}.`);
      return true;
    }
    // Inside a conversation the shortcut is a delegation the user makes, not a
    // navigation: the new agent opens as an aside floating over this chat, and
    // this chat stays put. Only the Welcome surface, with nothing to stay in,
    // still becomes the new chat.
    if (session) {
      await openAside(adapter, shortcut.rest, session.id);
      return true;
    }
    await openNewChat(shortcut.rest, adapter, alreadyLocked);
    return true;
  }

  // Create the aside session, hand it the projected brief of the conversation
  // it was asked from, send it the question, and float it over the chat. The
  // aside is a real standalone chat: it lives in the sidebar afterwards, and
  // closing the panel never ends it.
  async function openAside(adapter: import("./types").AdapterDescriptor, text: string, carryFromSessionId: string): Promise<void> {
    // The same double-submit lock every create path takes: a second Enter
    // while the create awaits must not make a second aside.
    if (newChatPendingRef.current) return;
    // The model the side chat begins on: carry the model of the chat it was
    // asked from when the harness matches, else that harness's Standard model —
    // never the bare adapter default (Codex's is a Fast model; OpenCode's is
    // null until a provider loads, which is what made codex/opencode asides
    // start on the wrong model or fail to cold start). See `resolveAsideModel`.
    const source = state.sessions.find(item => item.id === carryFromSessionId);
    const model = resolveAsideModel(adapter, source ? { harness: source.harness, model: source.model ?? null } : null);
    if (!model && adapter.models.length === 0) {
      setError(`${adapter.label} has no model available to start a side chat. Connect a provider model, then try again.`);
      return;
    }
    newChatPendingRef.current = true;
    setError(undefined);
    setAsideLifecycle({ sourceSessionId: carryFromSessionId, phase: "creating" });
    try {
      const title = text.length > 64 ? `${text.slice(0, 63).trimEnd()}…` : text;
      const result = await bridgeApi.createAsideChat(carryFromSessionId, adapter.id as Harness, model, title);
      setAsideLifecycle({
        sourceSessionId: result.sourceSessionId,
        sessionId: result.sessionId,
        phase: "handing_off",
        handoffStatus: result.handoffStatus,
        fidelity: result.fidelity,
      });
      setState(result.state);
      const created = result.state.sessions.find(item => item.id === result.sessionId);
      if (!created) throw new Error("Bridge created the aside but did not return its session");
      setAsideLifecycle({
        sourceSessionId: result.sourceSessionId,
        sessionId: result.sessionId,
        phase: "ready",
        handoffStatus: result.handoffStatus,
        fidelity: result.fidelity,
      });
      try {
        await deliverPrompt(created, text);
      } catch (e) {
        const message = errorMessage(e);
        setAsideLifecycle(current => current && {
          ...current,
          phase: "failed",
          error: message,
          recoveryDraft: text,
        });
        throw e;
      }
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      setAsideLifecycle(current => current && { ...current, phase: "failed", error: message });
      throw e;
    }
    finally { newChatPendingRef.current = false; }
  }
  // Entry point for the Welcome screen's own composer, which has no session
  // to skip past — a `$harness` prefix there is the only branch either way.
  async function startChatOrShortcut(text?: string, initialAttachments: ComposerAttachment[] = []) {
    if (newChatPendingRef.current) return;
    newChatPendingRef.current = true;
    try {
      if (text && initialAttachments.length === 0 && await openHarnessShortcut(text, true)) return;
      // Submit the open draft's choices; on the fresh welcome surface (no draft yet)
      // resolve them from the welcome workspace, just as this path used to.
      const draft: NewChatDraft = newChatDraft ?? {
        ...resolveDraftHarnessModel(),
        workspaceId: resolveNewChatWorkspaceId({
          activeWorkspaceId: welcomeWorkspaceId,
          lastWorkspaceId: readLastWorkspaceId(),
          workspaces: state.workspaces,
        }),
        createWorktree: false,
      };
      await submitNewChatDraft(draft, text, true, initialAttachments);
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
      // Open an unstarted workspace draft (#350) — no row until the first submit.
      // Snapshot harness/model before deselecting so the carry-over survives.
      const { harness, model } = resolveDraftHarnessModel();
      setView("workspace"); setParadigm("single");
      setWelcomeWorkspaceId(workspaceId);
      setNewChatDraft({ harness, model, workspaceId, createWorktree: false });
      setSelectedSessionId(undefined);
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
    const pending = pendingWelcomeMessageRef.current;
    // Deliver only to the session the message was created for — never to whatever
    // session happened to become active in the meantime (#350).
    if (!pending || session?.id !== pending.sessionId) return;
    pendingWelcomeMessageRef.current = null;
    void sendPrompt(pending.text, pending.attachments);
  }, [session?.id]);
  // Workspace "+": ask whether this orchestrator should get an isolated worktree.
  function requestWorkspaceSession(workspaceId: string) {
    if (!adaptersReady) { setError("No model adapter is available. Install or sign in to Codex, Claude, or OpenCode before starting an orchestrator."); return; }
    setPendingWorkspaceId(workspaceId);
    openModal("orchestrator");
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
        closeModal(); setPendingWorkspaceId(undefined);
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
    // The switch can take seconds (the outgoing provider may be asked for a
    // handoff summary); the narration row wears this instead of claiming the
    // old model is reading a message that does not exist.
    // A null model means the adapter's default; "Switching to Automatic…"
    // names nothing, so the harness carries the label instead.
    setModelSwitch({ sessionId: session.id, harness, label: model ? modelDisplayName(adapters, harness, model) : harnessLabel(harness) });
    try { setState(await bridgeApi.updateChatModel(session.id, harness, model)); }
    catch (e) { setError(errorMessage(e)); }
    finally { setBusy(false); setModelSwitch(null); }
  }
  async function submitNewWorkspace() {
    const name = title.trim(); if (!name) return;
    setBusy(true); setError(undefined);
    try {
      const next = await bridgeApi.createWorkspace(name);
      setState(next); closeModal(); setTitle("");
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
  // The composer's paste policy: clipboard images become removable preview
  // attachments (and the paste is intercepted before text insertion); every
  // other paste — text, file-less markup — falls through untouched. Sized
  // before decode: a silent multi-second paste for a huge screenshot reads as
  // broken, so the user hears why instead.
  const handleComposerPaste = (event: ClipboardEvent<HTMLTextAreaElement>) => {
    const items = event.clipboardData?.items;
    if (!items) return;
    const files = imageFilesFromClipboard(items);
    if (files.length === 0) return;
    event.preventDefault();
    if (files.some(isPasteTooLarge)) {
      setError("That image is too large to paste (over 8 MB). Save it to the repo and reference it with @ instead.");
      return;
    }
    void Promise.all(files.map(async file => ({
      id: crypto.randomUUID(),
      mediaType: mediaTypeOf(file),
      dataUri: await readAsDataUri(file),
    })))
      .then(pasted => setAttachments(current => [...current, ...pasted]))
      .catch(error => setError(errorMessage(error)));
    composerRef.current?.focus();
  };

  /// The delivery core both composers share: prepare, cold-start if needed,
  /// submit, with optimistic pending bookkeeping. Slash-command handling stays
  /// in `sendPrompt` - an aside is pinned to its harness on purpose.
  async function deliverPrompt(target: Session, submittedText: string, sentAttachments?: ComposerAttachment[]): Promise<void> {
    const key = crypto.randomUUID();
    const prepared = await bridgeApi.prepareTurn(target.id, submittedText);
    const text = prepared.text;
    try {
      setPending(current => [...current, { key, sessionId: target.id, text, attachment: sentAttachments?.[0]?.dataUri }]);
      if (!liveStatuses.includes(target.status)) {
        startedRef.current.add(target.id);
        setState(await bridgeApi.startChat(target.id));
      }
      // One call whatever the session is doing. The backend decides between
      // starting a turn, steering the live one, and durably queueing, and says
      // which — so the message can be shown in the state it is actually in.
      const outcome = await bridgeApi.submitInput(target.id, text, sentAttachments);
      if (outcome.disposition !== "startedNewTurn") {
        const delivery = outcome.disposition === "steeredActiveTurn" ? "steered" as const : "queued" as const;
        setPending(current => current.map(item => item.key === key ? { ...item, delivery } : item));
      }
    } catch (e) {
      setPending(current => current.filter(item => item.key !== key));
      throw e;
    }
  }
  async function sendPrompt(forcedText?: string, forcedAttachments?: ComposerAttachment[]) {
    const submittedText = (forcedText ?? composer).trim();
    const sentAttachments = forcedAttachments ?? attachments;
    if (!submittedText && sentAttachments.length === 0) return;
    // A harness shortcut is a chat launcher, not a turn — `$codex fix the lint`
    // opens a chat whose first message is that text. An image has nowhere to
    // go in that handoff, so with attachments in hand the words route into the
    // session like any other message.
    if (submittedText && sentAttachments.length === 0) {
      try {
        if (await openHarnessShortcut(submittedText)) { setComposer(""); return; }
      } catch {
        // The aside lifecycle owns the inline recovery state. Keep the source
        // draft untouched so Enter is also a valid retry path.
        return;
      }
    }
    if (!session) return;
    const agentMention = submittedText ? parseAgentMention(submittedText) : null;
    if (agentMention) {
      if (sentAttachments.length > 0) {
        setError("Agent shortcuts do not accept image attachments. Put the file in the workspace and reference it with @ instead.");
        return;
      }
      setComposer("");
      setAgentShortcutIndex(0);
      setAgentDispatchNotice(undefined);
      setError(undefined);
      try {
        const outcome = await bridgeApi.dispatchAgentShortcut(session.id, agentMention.token, agentMention.objective);
        const action = outcome.disposition === "awaitingApproval"
          ? "is waiting for write-scope approval"
          : outcome.disposition === "queued"
            ? "is queued for the next worker slot"
            : "was launched";
        setAgentDispatchNotice(`${outcome.agentName} (${outcome.role}) ${action}.`);
        await reload();
      } catch (e) {
        setComposer(submittedText);
        setError(errorMessage(e));
      }
      return;
    }
    const key = crypto.randomUUID();
    let target = session;
    let retryText = submittedText;
    setComposer("");
    setSlashIndex(0);
    setAttachments([]);
    try {
      const prepared = await bridgeApi.prepareTurn(target.id, submittedText);
      const text = prepared.text;
      retryText = text;
      // The optimistic row shows the image immediately; the durable row the
      // backend persists carries the same attachment data, so a reload
      // replays it identically.
      setPending(current => [...current, { key, sessionId: target.id, text, attachment: sentAttachments[0]?.dataUri }]);
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
      const outcome = await bridgeApi.submitInput(target.id, text, sentAttachments);
      if (outcome.disposition !== "startedNewTurn") {
        const delivery = outcome.disposition === "steeredActiveTurn" ? "steered" as const : "queued" as const;
        setPending(current => current.map(item => item.key === key ? { ...item, delivery } : item));
      }
      if (localOnly) {
        setPending(current => current.filter(item => item.key !== key));
        await reload();
      }
    }
    catch (e) { setComposer(retryText); setAttachments(sentAttachments); setPending(current => current.filter(item => item.key !== key)); setError(errorMessage(e)); }
  }
  const resolveApproval = useCallback(async (eventId: number, decision: ApprovalDecision, optionId?: string) => {
    if (!session?.id) return;
    try { const result = await bridgeApi.resolveApproval(session.id, eventId, decision, optionId); await reload(); return result; }
    catch (e) { setError(errorMessage(e)); throw e; }
  }, [reload, session?.id]);
  const resolveQuestion = useCallback(async (eventId: number, action: QuestionAction, answers: Record<string, string[]>) => {
    if (!session?.id) return;
    try { const result = await bridgeApi.resolveQuestion(session.id, eventId, action, answers); await reload(); return result; }
    catch (e) { setError(errorMessage(e)); throw e; }
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
    if (rememberAction(text) === "open-dialog") { setMemoryDraft(text); openModal("memory"); return; }
    try { await bridgeApi.saveMemoryRecord(text, undefined, session?.id ?? undefined); }
    catch (e) { setError(errorMessage(e)); }
  }, [openModal, session?.id]);

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
  // Selecting a worker only completes the directive. Sending remains an
  // explicit second action, so the user can write and review the objective.
  function applyAgentShortcut(candidate: AgentShortcutCandidate) {
    setComposer(`#${candidate.token} `);
    setAgentShortcutIndex(0);
    setAgentShortcutDismissed(true);
    composerRef.current?.focus();
  }
  function onComposerKeyDown(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    if (e.nativeEvent.isComposing) return;
    if (agentShortcutOpen) {
      if (e.key === "ArrowDown") { e.preventDefault(); setAgentShortcutIndex(index => Math.min(index + 1, agentShortcutMatches.length - 1)); return; }
      if (e.key === "ArrowUp") { e.preventDefault(); setAgentShortcutIndex(index => Math.max(index - 1, 0)); return; }
      if (e.key === "Escape") { e.preventDefault(); setAgentShortcutDismissed(true); return; }
      if ((e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) || e.key === "Tab") { e.preventDefault(); applyAgentShortcut(agentShortcutMatches[Math.min(agentShortcutIndex, agentShortcutMatches.length - 1)]); return; }
    }
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

  // Every chord in the app resolves through `src/keymap.ts`: this dispatcher,
  // the shell's menu items, and the shortcuts sheet all read the one table, so
  // a binding cannot mean different things in three places. Held in a ref
  // rather than a callback because the commands close over half the component
  // and the window listener must never run yesterday's copy of them.
  const commandRef = useRef<(id: CommandId, index?: number) => void>(() => {});
  commandRef.current = (id, index) => {
    switch (id) {
      case "new-chat":
        void startChatInCurrentRepo();
        return;
      case "new-project":
        setTitle("");
        openModal("workspace");
        return;
      case "interrupt-turn":
        // Reachable mid-sentence, so it has to be inert when nothing is running.
        if (session?.activeTurnId) void bridgeApi.interruptTurn(session.id);
        return;
      case "open-recall":
        if (!session) return;
        // Recall reads the conversation, so an expanded pane steps aside first.
        if (dockRef.current.expanded) dispatchDock({ type: "toggle-expanded" });
        setView("workspace");
        setRecallOpen(open => !open);
        return;
      case "jump-to-chat": {
        const target = topSessions[index ?? 0];
        if (target) openSession(target.id);
        return;
      }
      case "next-chat":
      case "previous-chat": {
        if (!topSessions.length) return;
        const step = id === "next-chat" ? 1 : -1;
        const current = topSessions.findIndex(chat => chat.id === session?.id);
        // Nothing open yet: step onto the end the direction came from.
        const target = current < 0
          ? topSessions[step > 0 ? 0 : topSessions.length - 1]
          : topSessions[(current + step + topSessions.length) % topSessions.length];
        openSession(target.id);
        return;
      }
      case "open-projects":
        setView("projects");
        return;
      case "open-settings":
        setView("settings");
        return;
      case "toggle-sidebar":
        setSidebarCollapsed(value => !value);
        return;
      case "toggle-fullscreen":
        setFullscreen(value => !value);
        return;
      case "toggle-dock":
        dispatchDock({ type: "toggle" });
        return;
      case "expand-dock":
        dispatchDock({ type: "toggle-expanded" });
        return;
      case "open-dock-pane": {
        const pane = DOCK_PANES[index ?? 0];
        if (pane) dispatchDock({ type: "open-pane", pane });
        return;
      }
      case "show-shortcuts":
        setShortcutsOpen(open => !open);
        return;
    }
  };

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const match = matchShortcut(event, isTypingTarget(event.target));
      if (match) {
        event.preventDefault();
        commandRef.current(match.shortcut.id, match.index);
        return;
      }
      if (event.key === "Escape") {
        // An expanded dock is the nearer layer: the first Escape restores it,
        // the next one leaves fullscreen.
        if (dockRef.current.open && dockRef.current.expanded) dispatchDock({ type: "toggle-expanded" });
        else setFullscreen(false);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [dispatchDock]);

  // A menu pick carries the same command id a chord does, so both land on the
  // same handler. Outside the desktop shell there is no menu and no listener.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void bridgeApi.onMenuCommand(id => commandRef.current(id)).then(dispose => {
      if (cancelled) dispose();
      else unlisten = dispose;
    });
    return () => { cancelled = true; unlisten?.(); };
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
  const startupError = error ?? (healthError ? errorMessage(healthError) : modelSetupError ? errorMessage(modelSetupError) : undefined);
  if (!health || !modelSetup) return <div className="relative grid h-[100dvh] place-items-center overflow-hidden bg-background text-muted-foreground"><div className="relative z-10 flex max-w-md items-center gap-2 px-6 text-center text-xs">{startupError ? <><X size={14} className="text-destructive" aria-hidden="true" />{startupError}</> : <><LoaderCircle className="animate-spin" size={14} aria-hidden="true" />Loading Bridge…</>}</div></div>;
  if (shouldRequireModelSetup(modelSetup, health.adapters)) return <div className="relative h-[100dvh] overflow-hidden bg-background"><ModelSetupWizard adapters={health.adapters} onComplete={acceptModelSetup} onError={setError} />{error && <Alert variant="error" className="fixed bottom-5 right-5 z-[60] max-w-md"><AlertTitle>Model setup failed</AlertTitle><AlertDescription>{error}</AlertDescription></Alert>}</div>;
  const chromeTitle = view === "work" ? "Work" : view === "projects" ? "Projects" : view === "marketplace" ? "Marketplace" : view === "settings" ? "Settings" : session?.title || session?.label || "Bridge";
  // A session view mounts SessionToolbar as its one chrome row instead of
  // AppTitleBar; every other view (including the pre-session Welcome screen)
  // keeps the title bar.
  const isSessionChrome = view === "workspace" && paradigm !== "grid" && !!session;
  const bypassBadge = <BypassBadge bypassing={!!permissionPolicy?.autoApproveProviderPermissions} onOpenSettings={() => { setSettingsSection("permissions"); setView("settings"); }} />;
  const usageWidget = <UsageWidget usage={usageByProvider} adapters={health?.adapters} samples={usageSamples} history={usageHistory} cacheDiagnostics={cacheDiagnostics} contextPercent={latestContext ?? undefined} contextSource={latestContextSource} focusedSessionId={session?.id ?? null} onOpenPromptStudio={() => { setSettingsSection("prompts"); setView("settings"); }} />;
  const titleBarActions = <>{bypassBadge}{usageWidget}</>;
  const sidebar = (
    <BridgeSidebar
      mobileOpen={navOpen}
      onCloseMobile={() => setNavOpen(false)}
      chats={topSessions}
      workspaces={state.workspaces}
      activeSessionId={session?.id}
      projectsActive={view === "projects"}
      marketplaceActive={view === "marketplace"}
      missionControlActive={view === "workspace" && paradigm === "grid"}
      workActive={view === "work"}
      settingsActive={view === "settings"}
      accountName={localAccountName(health.database, workspace?.path)}
      newChatBusy={busy}
      onOpenNewChat={() => void startChatInCurrentRepo()}
      onOpenProjects={() => setView("projects")}
      onOpenMarketplace={() => setView("marketplace")}
      onOpenMissionControl={() => { setView("workspace"); setParadigm("grid"); }}
      onOpenWorkBoard={openWorkBoard}
      onOpenMemory={() => openModal("memory")}
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
    {!isSessionChrome && <AppTitleBar
      flush
      hideBrand
      title={chromeTitle}
      navOpen={navOpen}
      onOpenNav={() => setNavOpen(true)}
      actions={titleBarActions}
    />}
    <main className="relative z-10 min-w-0 flex-1 overflow-hidden flex flex-col animate-page-mount">
      {!adaptersReady && <Alert variant="warning" className="mx-auto mt-4 w-[calc(100%-2rem)] max-w-2xl"><AlertTitle>No model adapters available</AlertTitle><AlertDescription>Bridge remains accessible, but chats and orchestrators are disabled until Codex, Claude, or OpenCode is installed and signed in.</AlertDescription></Alert>}
      <HealthWarnings warnings={health.warnings ?? []} className="mx-auto mt-4 w-[calc(100%-2rem)] max-w-2xl" />
      {view === "work" ? <Suspense fallback={<PanelLoading label="Opening work…"/>}><WorkView
        board={workBoard}
        error={workError}
        refreshError={workRefreshError}
        onRefresh={() => { setWorkBriefingError(undefined); void refetchWorkBoard(); }}
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
        onNewWorkspace={() => { setTitle(""); openModal("workspace"); }}
        onNewWorkspaceSession={requestWorkspaceSession}
        onConnectFolder={workspaceId => void connectFolder(workspaceId)}
      /> : view === "marketplace" ? <Suspense fallback={<PanelLoading label="Opening marketplace…"/>}><MarketplaceScreen /></Suspense> : view === "settings" ? <Suspense fallback={<PanelLoading label="Opening settings…"/>}><SettingsScreen adapters={adapters} autoApprovals={autoApprovals} initialSection={settingsSection} onModelSetupChange={acceptModelSetup} onSuggestionSettingsChange={setSuggestionSettings} onError={setError} /></Suspense> : paradigm === "grid" ? <MissionControl
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
          modelControl={isDirectChat ? undefined : <ChatModelControl
            adapters={adapters}
            harness={session.harness}
            model={session.model ?? null}
            disabled={busy || turnActive}
            disabledReason={turnActive ? "Wait for the current response before switching models" : undefined}
            onChange={(harness, model) => void changeChatModel(harness, model)}
            compact
            maxWidthClassName="max-w-[190px]"
            // The toolbar is the top row of an overflow-hidden container, so
            // the composer's upward panel would open above the viewport.
            placement="down"
            roleLabel={session.kind === "orchestrator" ? "Orchestrator" : "Chat"}
          />}
          bypassBadge={bypassBadge}
          actions={usageWidget}
          navOpen={navOpen}
          onOpenNav={() => setNavOpen(true)}
          dockOpen={dock.open}
          onToggleDock={() => dispatchDock({ type: "toggle" })}
          browserOpen={dock.open && dock.pane === "browser"}
          onToggleBrowser={() => dispatchDock(
            // The menu item is a checkbox, so it has to close what it opened:
            // a second activation collapses the dock instead of re-opening the
            // pane that is already showing.
            dock.open && dock.pane === "browser" ? { type: "toggle" } : { type: "open-pane", pane: "browser" },
          )}
          fullscreen={fullscreen}
          onToggleFullscreen={toggleLayoutFullscreen}
          onOpenRouterSettings={!isDirectChat && workspace ? () => openModal("router") : undefined}
          onToggleRecall={() => {
            // Recall reads the conversation, so an expanded pane steps aside first.
            if (dockRef.current.expanded) dispatchDock({ type: "toggle-expanded" });
            setRecallOpen(open => !open);
          }}
          recallOpen={recallOpen}
          onEnd={sessionConnected ? () => void endChat() : undefined}
          busy={busy}
        />
        <section ref={dockSectionRef} className="flex-1 min-h-0 overflow-hidden flex relative">
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
          {/* A user-made delegation floats over the chat it was asked from;
              the chat underneath never moves. See `openAside`. */}
          {asideSession && asideSession.id !== session.id && <AsideChat
            session={asideSession}
            adapters={adapters}
            events={agentEvents}
            pendingMessages={asidePending}
            working={!!asideSession.activeTurnId || asideSession.status === "working"}
            modelSwitch={modelSwitch?.sessionId === asideSession.id ? modelSwitch : null}
            lifecycle={asideLifecycle}
            initialDraft={asideLifecycle?.recoveryDraft}
            onSend={async (text, attachments) => {
              try {
                await deliverPrompt(asideSession, text, attachments);
                setAsideLifecycle(current => current && { ...current, phase: "ready", error: undefined, recoveryDraft: undefined });
              } catch (e) {
                setError(errorMessage(e));
                setAsideLifecycle(current => current && { ...current, phase: "failed", error: errorMessage(e) });
                throw e;
              }
            }}
            onChangeModel={async (harness, model) => {
              // Same path the main chat's control uses, bound to the aside
              // session so the switch never touches the chat underneath —
              // and narrated the same way, inside the aside's conversation.
              // Rethrows so the panel can wear the failure itself.
              setAsideLifecycle(current => current && { ...current, phase: "switching", error: undefined });
              setModelSwitch({ sessionId: asideSession.id, harness, label: model ? modelDisplayName(adapters, harness, model) : harnessLabel(harness) });
              try {
                setState(await bridgeApi.updateChatModel(asideSession.id, harness, model));
                setAsideLifecycle(current => current && { ...current, phase: "ready", error: undefined });
              } catch (e) {
                setAsideLifecycle(current => current && { ...current, phase: "failed", error: errorMessage(e) });
                throw e;
              }
              finally { setModelSwitch(null); }
            }}
            onResolve={async (eventId, decision, optionId) => {
              try { const result = await bridgeApi.resolveApproval(asideSession.id, eventId, decision, optionId); await reload(); return result; }
              catch (e) { setError(errorMessage(e)); throw e; }
            }}
            onAnswerQuestion={async (eventId, action, answers) => {
              try { const result = await bridgeApi.resolveQuestion(asideSession.id, eventId, action, answers); await reload(); return result; }
              catch (e) { setError(errorMessage(e)); throw e; }
            }}
            onPromote={() => { setAsideLifecycle(undefined); openSession(asideSession.id); }}
            onClose={() => setAsideLifecycle(undefined)}
          />}
          <div className={cn("flex-1 min-w-0 flex flex-col relative", dockExpandedVisible && "hidden")}>
            <>
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
                  modelSwitch={modelSwitch?.sessionId === session?.id ? modelSwitch : null}
                   pendingMessages={pendingForSession}
                   pendingAttachments={pendingForSessionAttachments}
                  onResolve={resolveApproval}
                  onAnswerQuestion={resolveQuestion}
                  workspaceFiles={hasRepo ? workspaceFiles : undefined}
                  onOpenFile={hasRepo && workspace ? openFileInDock : undefined}
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
                {agentDispatchNotice && <div className="mx-auto mb-2 flex max-w-2xl justify-center px-4 sm:px-6">
                  <div className="u-glass-soft inline-flex items-center gap-2 min-h-[30px] px-3.5 rounded-full text-muted-foreground text-xs" role="status">
                    <Bot size={12} aria-hidden="true" />
                    <span>{agentDispatchNotice}</span>
                  </div>
                </div>}
                {/* A worker gets a steering composer, not the chat composer: what
                    you type amends the objective its orchestrator gave it, and
                    the orchestrator is told so it does not fight the change. */}
                {isWorkerView ? <div className="mx-auto max-w-2xl px-4 sm:px-6">
                  <div className="u-glass-soft flex items-center gap-2.5 rounded-2xl px-4 py-2.5 text-[12px] text-muted-foreground"><Bot size={14} className="shrink-0 text-muted-foreground" aria-hidden="true" /><span>This is a background worker. It takes its objective from its orchestrator — steer it here to amend that objective.</span></div>
                  <SteerComposer sessionId={session.id} steerable={!!workerSteerable} onSteer={steerWorker} className="pt-2"/>
                </div> : <div className="relative mx-auto max-w-2xl">
                  {!slashOpen && !mentionOpen && !agentShortcutOpen && !harnessShortcutOpen && skillSuggestions.length > 0 && <div className="u-glass-popover absolute bottom-full left-4 right-4 z-20 mb-2 overflow-hidden rounded-2xl sm:left-6 sm:right-6"><div className="border-b border-border px-3 py-1.5 text-[9px] uppercase tracking-[0.12em] text-muted-foreground/70">Available skills for this task</div>{skillSuggestions.map(suggestion => <button key={suggestion.id} type="button" onMouseDown={event => { event.preventDefault(); setComposer(current => `/${suggestion.command} ${current}`); setSkillSuggestions([]); }} className="flex w-full items-start gap-3 border-b border-border px-3 py-2 text-left last:border-0 hover:bg-accent"><span className="mt-0.5 rounded border border-success/25 bg-success/10 px-1.5 py-0.5 text-[8.5px] uppercase text-success">installed</span><span className="min-w-0 flex-1"><b className="block truncate text-[11px] font-medium text-foreground">{suggestion.name}</b><small className="mt-0.5 block text-[9.5px] leading-4 text-muted-foreground">{suggestion.relevance} · {suggestion.source} · {suggestion.risk} risk · {suggestion.permissions.join(", ")}</small></span></button>)}</div>}
                  {agentShortcutOpen && <div id="agent-shortcut-listbox" role="listbox" aria-label="Specialist agents" className="u-glass-popover absolute left-4 right-4 sm:left-6 sm:right-6 bottom-full mb-2 z-20 rounded-2xl overflow-hidden flex flex-col max-h-[min(420px,55vh)]">
                    <div className="shrink-0 px-3 py-1.5 text-[9px] uppercase tracking-[0.12em] text-muted-foreground/70 border-b border-border flex items-center gap-2">
                      <span>Dispatch a specialist</span>
                      <span className="normal-case tracking-normal text-muted-foreground/50">{agentShortcutMatches.length}</span>
                    </div>
                    <div ref={agentShortcutListRef} className="min-h-0 flex-1 overflow-y-auto overscroll-contain" onWheel={e => e.stopPropagation()}>
                      {agentShortcutMatches.map((candidate, index) => {
                        const agent = candidate.agent;
                        const model = agent.model ?? (agent.harness === "bridge" ? "automatic model" : "default model");
                        const access = agent.role === "implementation" ? "isolated write" : "read only";
                        return <button id={`agent-shortcut-option-${index}`} role="option" aria-selected={index === agentShortcutIndex} key={agent.id ?? `${agent.name}:${agent.role}`} type="button" data-agent-shortcut-index={index} onMouseEnter={() => setAgentShortcutIndex(index)} onMouseDown={e => { e.preventDefault(); applyAgentShortcut(candidate); }} className={`min-h-12 w-full flex items-start gap-3 px-3 py-2 text-left transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-ring ${index === agentShortcutIndex ? "bg-accent" : "hover:bg-accent"}`}>
                          <Bot size={13} className="mt-0.5 shrink-0 text-muted-foreground" aria-hidden="true" />
                          <span className="min-w-0 flex-1">
                            <span className="flex items-baseline gap-2"><b className="font-mono text-[12px] font-medium text-foreground">#{candidate.token}</b><span className="truncate text-[11px] text-muted-foreground">{agent.name}</span></span>
                            <small className="mt-0.5 block truncate text-[9.5px] leading-4 text-muted-foreground">{agent.role} · {agent.harness} / {model} · {access} · {agent.effort} effort</small>
                          </span>
                        </button>;
                      })}
                    </div>
                  </div>}
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
                    onChange={value => { setComposer(value); setSlashDismissed(false); setSlashIndex(0); setMentionDismissed(false); setMentionIndex(0); setAgentShortcutDismissed(false); setAgentShortcutIndex(0); setHarnessShortcutDismissed(false); setHarnessShortcutIndex(0); }}
                    onSubmit={() => void sendPrompt()}
                    onKeyDown={onComposerKeyDown}
                    onPaste={handleComposerPaste}
                    attachments={attachments}
                    onRemoveAttachment={id => setAttachments(current => current.filter(attachment => attachment.id !== id))}
                    autocomplete={agentShortcutOpen ? {
                      controls: "agent-shortcut-listbox",
                      activeDescendant: `agent-shortcut-option-${agentShortcutIndex}`,
                    } : mentionOpen ? {
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
            </>
          </div>
          <SessionDock
            state={dock}
            panes={dockPanes}
            availableWidth={dockSectionWidth}
            sheet={dockSheet}
            concealed={fullscreen}
            onAction={dispatchDock}
            onConnectFolder={workspace && !hasRepo ? () => void connectFolder(workspace.id) : undefined}
          >
            {pane => {
              if (pane === "tasks") return <TasksPane
                key={session.id}
                sessions={visibleSessions}
                runtimes={forest?.workerRuntimes}
                queue={forest?.workerQueue}
                terminalActivity={terminalActivity}
                acknowledged={acknowledgedTasks}
                onAcknowledge={id => setAcknowledgedTasks(previous => new Set(previous).add(id))}
                onOpenSession={openSession}
                onExpandWorker={setExpandedWorkerId}
                onRetryWorker={id => void retryWorkerTask(id)}
                onOpenTerminal={() => dispatchDock({ type: "open-pane", pane: "terminal" })}
              />;
              if (pane === "browser") return <BrowserSurface
                visible={dock.open && dock.pane === "browser" && !fullscreen}
                onError={setError}
                onSupervisionChange={setBrowserSupervision}
              />;
              if (pane === "transcript") return <TranscriptPane
                key={session.id}
                sessionId={session.id}
                events={sessionEvents}
                entries={forest?.entries}
                head={forest?.head}
                leaves={forest?.leaves}
                loadOlder={request => bridgeApi.replaySessionEvents(
                  session.id,
                  request.tail ? 0 : Math.max(0, (request.beforeSequence ?? 1) - 1 - TRANSCRIPT_PAGE_SIZE),
                  TRANSCRIPT_PAGE_SIZE,
                  request.tail,
                )}
                onRevealEntry={revealEntryInConversation}
              />;
              if (!workspace) return null;
              /* Keyed on the workspace: these panes hold open buffers, shells,
                 and relative paths, and none of that survives a change of tree.
                 Without the key a save would aim the old path at the new
                 workspace. */
              if (pane === "github") return <GitHubPane key={workspace.id} workspaceId={workspace.id} workspaceBranch={workspace.branch ?? null} sessionId={session?.id} intent={githubIntent} onJumpToFile={jumpToReviewComment} />;
              if (pane === "changes") return <ChangesPanel key={workspace.id} workspace={workspace} onQuote={quoteToComposer} onOpenFile={openFileInDock} />;
              if (pane === "code") return <Suspense fallback={<PanelLoading label="Opening editor…"/>}><CodePanel key={workspace.id} workspaceId={workspace.id} visible={dock.open && dock.pane === "code"} reveal={codeReveal} driftSignal={`${workspace.dirtyFiles}:${workspace.additions}:${workspace.deletions}`} onSaved={() => void refreshWorkspaceStats(workspace.id)}/></Suspense>;
              return <Suspense fallback={<PanelLoading label="Opening terminal…"/>}><TerminalPane key={workspace.id} workspaceId={workspace.id} workspacePath={workspace.path ?? undefined} visible={dock.open && dock.pane === "terminal" && !fullscreen} onActivity={setTerminalActivity}/></Suspense>;
            }}
          </SessionDock>
        </section>
      </> : <Welcome
        adapters={adapters}
        harness={(newChatDraft ?? resolveDraftHarnessModel()).harness}
        model={(newChatDraft ?? resolveDraftHarnessModel()).model}
        onSelectModel={(harness, model) => setNewChatDraft(current => ({
          ...(current ?? { workspaceId: resolvedWelcomeWorkspaceId, createWorktree: false }),
          harness,
          model,
        }))}
        canStartChat={adaptersReady}
        busy={busy}
        workspaces={state.workspaces}
        workspace={welcomeWorkspace}
        worktree={newChatDraft?.createWorktree ?? false}
        branches={branchWorkspaceId === welcomeWorkspace?.id ? workspaceBranches : []}
        currentBranch={branchWorkspaceId === welcomeWorkspace?.id ? workspaceBranchCurrent : welcomeWorkspace?.branch ?? null}
        branchBusy={branchWorkspaceId === welcomeWorkspace?.id && branchBusy}
        branchError={branchWorkspaceId === welcomeWorkspace?.id ? branchError : null}
        onSelectWorkspace={id => { writeLastWorkspaceId(id); setWelcomeWorkspaceId(id); setNewChatDraft(current => current ? { ...current, workspaceId: id } : current); }}
        onRequestBranches={() => { if (welcomeWorkspace) void requestWorkspaceBranches(welcomeWorkspace.id); }}
        onSelectBranch={branch => { if (welcomeWorkspace) void switchWorkspaceBranch(welcomeWorkspace.id, branch); }}
        onToggleWorktree={() => setNewChatDraft(current => current
          ? { ...current, createWorktree: !current.createWorktree }
          // Fresh welcome surface with no draft yet: the worktree decision is now
          // held on the draft (#350), created on submit — not started immediately.
          : { ...resolveDraftHarnessModel(), workspaceId: resolvedWelcomeWorkspaceId, createWorktree: true })}
        onStartChat={(text, initialAttachments) => void startChatOrShortcut(text, initialAttachments)}
        onNewWorkspace={() => { setTitle(""); openModal("workspace"); }}
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
    {/* Behind the error alert on purpose: a failure to act outranks CI news. */}
    <GithubToasts
      toasts={githubToasts}
      hint={githubJumpHint}
      onOpen={openCiToast}
      onDismiss={key => setGithubToasts(current => current.filter(toast => toast.key !== key))}
      onDismissHint={() => setGithubJumpHint(undefined)}
    />

    <WorkspaceCreateDialog
      open={modal === "workspace"}
      title={title}
      busy={busy}
      onTitleChange={setTitle}
      onClose={closeModal}
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
    <RouterSettingsDialog open={modal === "router"} workspaceId={workspace?.id} adapters={adapters} databasePath={health.database} onModelSetupChange={acceptModelSetup} onClose={closeModal} onError={setError} />
    <ShortcutsSheet open={shortcutsOpen} onClose={() => setShortcutsOpen(false)} />
    <MemoryDialog open={modal === "memory"} initialBody={memoryDraft} adapters={adapters} onClose={() => { closeModal(); setMemoryDraft(null); }} onError={setError} />
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

function EnvPanel({ workspace, project, session, sessions, forest, onChanges, onSelectLeaf, onCompact }: { workspace: Workspace; project?: Project; session?: Session; sessions: Session[]; forest?: SessionForestSnapshot; onChanges: () => void; onSelectLeaf: (entryId: string) => void; onCompact: () => void }) {
  const workers = sessions.filter(s => s.parentSessionId && s.parentSessionId === session?.id);
  const doneWorkers = workers.filter(s => s.status === "stopped" || s.status === "ready").length;
  const budget = turnBudget(forest, session?.activeTurnId);
  const restoration = restorationPresentation(session?.restorationMode ?? "fresh");
  return <aside className="hidden">
  </aside>;
}

function Welcome({ adapters, harness, model, onSelectModel, busy, canStartChat, onStartChat, onNewWorkspace, workspaces, workspace, worktree, branches, currentBranch, branchBusy, branchError, onSelectWorkspace, onRequestBranches, onSelectBranch, onToggleWorktree }: {
  adapters: import("./types").AdapterDescriptor[];
  harness: Harness;
  model: string | null;
  onSelectModel: (harness: Harness, model: string | null) => void;
  busy: boolean;
  canStartChat: boolean;
  onStartChat: (text?: string, attachments?: ComposerAttachment[]) => void;
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
  const [attachments, setAttachments] = useState<ComposerAttachment[]>([]);
  const [composerError, setComposerError] = useState<string>();
  const submit = () => {
    const text = draft.trim();
    if (text || attachments.length > 0) onStartChat(text, attachments);
    else onStartChat();
  };
  const handlePaste = (event: ClipboardEvent<HTMLTextAreaElement>) => {
    const items = event.clipboardData?.items;
    if (!items) return;
    const files = imageFilesFromClipboard(items);
    if (files.length === 0) return;
    event.preventDefault();
    if (files.some(isPasteTooLarge)) {
      setComposerError("That image is too large to paste (over 8 MB). Save it to the repo and reference it with @ instead.");
      return;
    }
    setComposerError(undefined);
    void Promise.all(files.map(async file => ({
      id: crypto.randomUUID(),
      mediaType: mediaTypeOf(file),
      dataUri: await readAsDataUri(file),
    })))
      .then(pasted => setAttachments(current => [...current, ...pasted]))
      .catch(error => setComposerError(errorMessage(error)));
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
      onPaste={handlePaste}
      attachments={attachments}
      onRemoveAttachment={id => setAttachments(current => current.filter(attachment => attachment.id !== id))}
      placeholder={canStartChat ? "Ask Bridge…" : "Install or sign in to a model adapter…"}
      disabled={busy || !canStartChat}
      // There is no conversation or folder here yet, so the structural `+`
      // still creates a workspace. Clipboard images are first-turn content and
      // use the paste path above instead of pretending to be repository files.
      plusLabel="New workspace"
      onPlusClick={onNewWorkspace}
      // The unstarted draft is a real chat-in-waiting: let the model be chosen
      // before the first message, the same picker the session composer uses.
      trailing={<ChatModelControl adapters={adapters} harness={harness} model={model} disabled={busy || !canStartChat} onChange={onSelectModel} compact roleLabel="Chat" />}
    />
    {composerError && <p className="mt-2 max-w-2xl text-left text-[11px] text-destructive">{composerError}</p>}
    <p className="mt-6 max-w-md text-[13px] leading-relaxed text-muted-foreground">{greeting.hint}</p>
  </div>;
}
function CommandPalette({ workspaces, onChoose }: { workspaces: Workspace[]; onChoose: (id:string)=>void }) { return <><InputGroup className="border-b border-border rounded-none border-x-0 border-t-0 shadow-none"><InputGroupInput autoFocus placeholder="Search workspaces and actions…" /><InputGroupAddon><Search size={17} aria-hidden="true" /></InputGroupAddon></InputGroup><div className="p-[9px]"><label className="block p-[5px_9px_7px] text-muted-foreground/65 text-[10px] font-semibold tracking-[0.09em]">WORKSPACES</label>{workspaces.map(w => <Button type="button" key={w.id} variant="ghost" className="w-full h-[44px] rounded-md justify-start px-2.5" onClick={() => onChoose(w.id)}><StatusDot status={w.status}/><span className="flex flex-col gap-[3px] flex-1 text-left"><b className="text-[12.5px] font-medium">{w.title}</b><small className="text-[10.5px] text-muted-foreground">{w.city} · {w.branch}</small></span><Kbd className="font-mono text-muted-foreground/65 border border-border rounded px-1 py-[1px] text-[10px]">↵</Kbd></Button>)}</div><div className="h-[32px] border-t border-border flex items-center gap-[14px] px-[13px] text-muted-foreground/65 text-[10.5px]"><span>↑↓ navigate</span><span>esc close</span></div></>; }
