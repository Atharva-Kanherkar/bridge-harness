import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Activity, Archive, Bot, Check, ChevronDown, CircleDot, Clock3, FileCode2, FileDiff, FileText, GitBranch, GitCommitHorizontal, GitPullRequest, Inbox, LayoutGrid, LoaderCircle, MessageSquareText, Monitor, Play, Plus, Search, Settings2, Square, TerminalSquare, X } from "lucide-react";
import { bridgeApi } from "./api";
import { appendAgentEventBatch } from "./agentEvents";
import type { AgentEvent, BridgeState, CapabilitySuggestion, Harness, Health, Project, Session, SessionForestSnapshot, SessionStatus, Workspace } from "./types";
import { AgentConversation } from "./components/AgentConversation";
import { BridgeSidebar } from "./components/BridgeSidebar";
import { ComposerPill } from "./components/ComposerPill";
import { SpaceBackground } from "./components/SpaceBackground";
import { WorkspaceCreateDialog } from "./components/WorkspaceCreateDialog";
import { RouterSettingsDialog } from "./components/RouterSettingsDialog";
import { UsageWidget } from "./components/UsageWidget";
import { formatElapsed, tierRuntimeLabel } from "./utils";
import { projectSessionConversation, reduceConversation } from "./conversation";
import { pickGreeting } from "./greetings";
import { buildUsageHistory, clampPercent, extractUsageSnapshot, type UsageProvider, type UsageRateSample, type UsageSnapshot } from "./usage";
import { describeError } from "./errors";
import { forestSnapshotKey } from "./forest";
import { queueExplanation, restorationPresentation, turnBudget } from "./observability";
import { startSerialPoll } from "./polling";
import { Alert, AlertAction, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { InputGroup, InputGroupAddon, InputGroupInput } from "@/components/ui/input-group";
import { Kbd } from "@/components/ui/kbd";
import { Tabs, TabsList, TabsTab } from "@/components/ui/tabs";

const MarketplaceScreen = lazy(() => import("./components/MarketplaceScreen").then(module => ({ default: module.MarketplaceScreen })));
const TerminalPane = lazy(() => import("./components/TerminalPane").then(module => ({ default: module.TerminalPane })));

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

function harnessLabel(harness?: string | null): string {
  if (harness === "claude") return "Claude";
  if (harness === "codex") return "Codex";
  return harness ? harness[0].toUpperCase() + harness.slice(1) : "Agent";
}

function errorMessage(value: unknown): string {
  return value instanceof Error ? value.message : String(value);
}

export function App() {
  const [state, setState] = useState<BridgeState>(emptyState);
  const [agentEvents, setAgentEvents] = useState<AgentEvent[]>([]);
  const [health, setHealth] = useState<Health>();
  const [selectedSessionId, setSelectedSessionId] = useState<string>();
  const [view, setView] = useState<"workspace" | "marketplace">("workspace");
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [activeTab, setActiveTab] = useState<"agent" | "changes" | "events" | "terminal">("agent");
  const [modal, setModal] = useState<"chat" | "workspace" | "router" | null>(null);
  const [title, setTitle] = useState("");
  const [composer, setComposer] = useState("");
  const [slashCommands, setSlashCommands] = useState<import("./types").SlashCommand[]>([]);
  const [slashIndex, setSlashIndex] = useState(0);
  const [slashDismissed, setSlashDismissed] = useState(false);
  const [skillSuggestions, setSkillSuggestions] = useState<CapabilitySuggestion[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [clock, setClock] = useState(Date.now());
  const [forest, setForest] = useState<SessionForestSnapshot>();
  const [pending, setPending] = useState<{ key: string; sessionId: string; text: string }[]>([]);
  const [usageByProvider, setUsageByProvider] = useState<Partial<Record<UsageProvider, UsageSnapshot>>>({});
  const [usageSamples, setUsageSamples] = useState<Partial<Record<UsageProvider, UsageRateSample[]>>>({});
  const startedRef = useRef<Set<string>>(new Set());
  const pendingWelcomeMessageRef = useRef<string | null>(null);
  const forestKeyRef = useRef("");
  const agentEventQueueRef = useRef<AgentEvent[]>([]);
  const agentEventTimerRef = useRef<number | undefined>(undefined);

  const reload = useCallback(async () => { setState(await bridgeApi.state()); }, []);
  useEffect(() => {
    void Promise.all([reload(), bridgeApi.health().then(setHealth)]);
    let offState: (() => void) | undefined;
    let offAgent: (() => void) | undefined;
    let offUsage: (() => void) | undefined;
    void bridgeApi.onStateChanged(reload).then(fn => offState = fn);
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
      offState?.(); offAgent?.(); offUsage?.();
      if (agentEventTimerRef.current !== undefined) window.clearTimeout(agentEventTimerRef.current);
      agentEventTimerRef.current = undefined;
      agentEventQueueRef.current = [];
    };
  }, [reload]);
  useEffect(() => { const timer = window.setInterval(() => setClock(Date.now()), 30_000); return () => window.clearInterval(timer); }, []);
  useEffect(() => {
    const key = (e: KeyboardEvent) => { if (e.key === "Escape") setModal(null); };
    window.addEventListener("keydown", key); return () => window.removeEventListener("keydown", key);
  }, []);
  useEffect(() => { document.documentElement.classList.add("dark"); }, []);

  const adapters = health?.adapters ?? [];
  const adaptersReady = adapters.some(adapter => adapter.available);
  const topSessions = useMemo(() => state.sessions.filter(s => s.harness !== "shell" && !s.parentSessionId), [state.sessions]);
  const standaloneChats = useMemo(() => topSessions.filter(s => !s.workspaceId), [topSessions]);
  const session = topSessions.find(s => s.id === selectedSessionId);
  const workspace = session?.workspaceId ? state.workspaces.find(w => w.id === session.workspaceId) : undefined;
  const hasRepo = !!workspace?.path;
  const isDirectChat = session?.kind === "direct";
  const sessionConnected = !!session && !session.endedAt && liveStatuses.includes(session.status);
  const sessionEvents = useMemo(() => agentEvents.filter(event => event.sessionId === session?.id), [agentEvents, session?.id]);
  const pendingForSession = useMemo(() => pending.filter(p => p.sessionId === session?.id).map(p => p.text), [pending, session?.id]);
  const usageHistory = useMemo(() => buildUsageHistory(forest?.usage ?? [], state.sessions), [forest?.usage, state.sessions]);
  const latestContext = session?.contextPercent ?? usageHistory.find(entry => entry.contextPercent != null)?.contextPercent;
  const latestContextSource = session?.contextPercent != null ? session.metricSource : usageHistory.find(entry => entry.contextPercent != null)?.source;
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

  useEffect(() => {
    const query = composer.trim();
    if (!session || query.length < 8 || query.startsWith("/")) { setSkillSuggestions([]); return; }
    let active = true;
    const timer = window.setTimeout(() => { void bridgeApi.skillSuggestions(query, session.harness === "claude" ? "claude" : "codex").then(items => { if (active) setSkillSuggestions(items.slice(0, 3)); }).catch(() => { if (active) setSkillSuggestions([]); }); }, 300);
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

  useEffect(() => {
    forestKeyRef.current = "";
    setForest(undefined);
    if (!session?.id) return;
    let active = true;
    const refresh = async () => {
      const value = await bridgeApi.sessionForest(session.id).catch(() => undefined);
      if (!active || !value) return;
      const key = forestSnapshotKey(value);
      if (key === forestKeyRef.current) return;
      forestKeyRef.current = key;
      setForest(value);
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

  function openSession(id: string) { setView("workspace"); setSelectedSessionId(id); }
  // New chat opens instantly (no picker up front): create a direct chat with the
  // default model and select it. The model can be changed inside the chat.
  async function openNewChat(initialMessage?: string) {
    setView("workspace");
    const preferred = adapters.find(adapter => adapter.available) ?? adapters[0];
    const harness = (preferred?.id as Harness) ?? "codex";
    const model = preferred?.defaultModel ?? preferred?.models[0]?.id ?? null;
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
  // Workspace "+": start a classic orchestrator session tied to the workspace.
  async function newWorkspaceSession(workspaceId: string) {
    setBusy(true); setError(undefined);
    try {
      const next = await bridgeApi.createWorkspaceSession(workspaceId);
      const created = [...next.sessions].reverse().find(s => !s.parentSessionId && s.workspaceId === workspaceId);
      setState(next);
      setExpanded(current => new Set(current).add(workspaceId));
      if (created) setSelectedSessionId(created.id);
    } catch (e) { setError(errorMessage(e)); }
    finally { setBusy(false); }
  }
  async function changeChatModel(harness: Harness, model: string | null) {
    if (!session) return;
    try { setState(await bridgeApi.updateChatModel(session.id, harness, model)); }
    catch (e) { setError(errorMessage(e)); }
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
    try { setState(await bridgeApi.stopSession(session.id)); }
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
        const next = await bridgeApi.updateChatModel(target.id, resolved.harness, adapter?.defaultModel ?? null);
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
  const resolveApproval = useCallback(async (eventId: number, decision: string) => {
    if (!session?.id) return;
    try { await bridgeApi.resolveApproval(session.id, eventId, decision); await reload(); }
    catch (e) { setError(errorMessage(e)); }
  }, [reload, session?.id]);
  const waiveCompletion = useCallback(async (attemptId: string, checkIds: string[], reason: string) => {
    const completion = await bridgeApi.waiveCompletion(attemptId, checkIds, reason);
    setForest(current => current ? { ...current, completion } : current);
  }, []);
  async function applySlash(command: import("./types").SlashCommand) {
    if (session?.kind === "direct" && command.harness !== session.harness) {
      const adapter = adapters.find(item => item.id === command.harness);
      try { setState(await bridgeApi.updateChatModel(session.id, command.harness, adapter?.defaultModel ?? null)); }
      catch (e) { setError(errorMessage(e)); return; }
    }
    setComposer(`/${command.name} `);
    setSlashIndex(0);
    setSlashDismissed(true);
  }
  function onComposerKeyDown(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    if (slashOpen) {
      if (e.key === "ArrowDown") { e.preventDefault(); setSlashIndex(index => Math.min(index + 1, slashMatches.length - 1)); return; }
      if (e.key === "ArrowUp") { e.preventDefault(); setSlashIndex(index => Math.max(index - 1, 0)); return; }
      if (e.key === "Escape") { e.preventDefault(); setSlashDismissed(true); return; }
      if ((e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) || e.key === "Tab") { e.preventDefault(); void applySlash(slashMatches[Math.min(slashIndex, slashMatches.length - 1)]); return; }
    }
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) { e.preventDefault(); void sendPrompt(); }
  }

  const toggleExpanded = (id: string) => setExpanded(current => { const next = new Set(current); if (next.has(id)) next.delete(id); else next.add(id); return next; });

  const turnActive = !!session?.activeTurnId || pendingForSession.length > 0;
  return <div className="space-dark relative flex h-[100dvh] overflow-hidden text-neutral-200">
    <SpaceBackground paused={turnActive} />

    <div className="fixed right-3 top-3 z-30 flex items-center gap-1.5 sm:right-5 sm:top-5">
      <UsageWidget usage={usageByProvider} samples={usageSamples} history={usageHistory} contextPercent={latestContext ?? undefined} contextSource={latestContextSource} />
    </div>

    <BridgeSidebar
      standaloneChats={standaloneChats}
      workspaces={state.workspaces}
      workspaceChats={workspaceId => topSessions.filter(s => s.workspaceId === workspaceId)}
      activeSessionId={session?.id}
      marketplaceActive={view === "marketplace"}
      expanded={expanded}
      busy={busy}
      onOpenNewChat={() => void openNewChat()}
      onOpenMarketplace={() => setView("marketplace")}
      onOpenSession={openSession}
      onToggleWorkspace={toggleExpanded}
      onNewWorkspace={() => { setTitle(""); setModal("workspace"); }}
      onNewWorkspaceSession={workspaceId => void newWorkspaceSession(workspaceId)}
      onConnectFolder={workspaceId => void connectFolder(workspaceId)}
    />
    <main className="relative z-10 min-w-0 flex-1 overflow-hidden flex flex-col animate-page-mount">
      {view === "marketplace" ? <Suspense fallback={<PanelLoading label="Opening marketplace…"/>}><MarketplaceScreen /></Suspense> : session ? <>
        <div className={`shrink-0 px-4 sm:px-6 flex items-center border-b border-white/[0.04] ${isDirectChat ? "h-[48px]" : "min-h-[52px] py-2"}`}>
          <div className="min-w-0 flex-1">
            <h1 className="m-0 font-display text-sm sm:text-[15px] leading-tight text-white font-semibold tracking-tight whitespace-nowrap overflow-hidden text-ellipsis">{session.title || session.label}</h1>
            {!isDirectChat && <div className="mt-1 flex items-center gap-1.5 text-neutral-500 font-mono text-[10px]">
              <Bot size={12} aria-hidden="true" />{session.kind === "orchestrator" ? "Orchestrator" : harnessLabel(session.harness)}
              {hasRepo && workspace && <><span>·</span><GitBranch size={12} aria-hidden="true" />{workspace.branch ?? "folder"}<span>·</span>{workspace.dirtyFiles ? <span className="text-warning">{workspace.dirtyFiles} changed</span> : <span>clean</span>}</>}
            </div>}
          </div>
          <div className="ml-auto flex items-center gap-[7px]">
            {!isDirectChat && workspace && <Button type="button" variant="ghost" size="icon-sm" className="text-muted-foreground" onClick={() => setModal("router")} aria-label="Learning router settings"><Settings2 size={14} aria-hidden="true" /></Button>}
            {sessionConnected && <Button type="button" variant="ghost" size="sm" className="text-muted-foreground hover:text-destructive" disabled={busy} onClick={() => void endChat()}>{busy ? <LoaderCircle className="animate-spin" size={14} aria-hidden="true" /> : <Square size={13} aria-hidden="true" />} End</Button>}
          </div>
        </div>
        {hasRepo && <Tabs value={activeTab} onValueChange={v => setActiveTab(v as typeof activeTab)} className="shrink-0">
          <div className="border-b-0 px-[14px] pt-1">
            <TabsList variant="underline" className="w-full justify-start gap-[2px] bg-transparent p-0">
              <TabsTab value="agent" className="h-[28px] px-2 text-[10.5px] text-muted-foreground rounded-none"><MessageSquareText size={14} aria-hidden="true" /> Agent</TabsTab>
              <TabsTab value="changes" className="h-[28px] px-2 text-[10.5px] text-muted-foreground rounded-none"><FileCode2 size={14} aria-hidden="true" /> Changes {workspace && workspace.dirtyFiles > 0 && <Badge variant="secondary" size="sm">{workspace.dirtyFiles}</Badge>}</TabsTab>
              <TabsTab value="terminal" className="h-[28px] px-2 text-[10.5px] text-muted-foreground rounded-none"><TerminalSquare size={14} aria-hidden="true" /> Terminal</TabsTab>
            </TabsList>
          </div>
        </Tabs>}
        <section className="flex-1 min-h-0 overflow-hidden flex relative">
          <div className="flex-1 min-w-0 flex flex-col relative">
            {(activeTab === "agent" || !hasRepo) && <>
              <div className="flex-1 min-h-0 relative">
                <AgentConversation
                  session={session}
                  events={sessionEvents}
                  forestEntries={forest?.entries}
                  activeLeafId={forest?.head?.activeEntryId}
                  repositoryDivergence={forest?.repositoryDivergence.status}
                  completion={forest?.completion}
                  onWaiveCompletion={waiveCompletion}
                  continuationFidelity={session?.continuationFidelity}
                  preview={false}
                  working={turnActive}
                  pendingMessages={pendingForSession}
                  onResolve={resolveApproval}
                />
              </div>
              <div className="pointer-events-none absolute bottom-0 left-0 right-0 h-16 bg-gradient-to-t from-[#0a0a0c] to-transparent sm:h-20" />
              <div className="relative z-10 flex-none safe-bottom">
                {hasRepo && workspace && workspace.dirtyFiles > 0 && <div className="mx-auto mb-2 flex max-w-2xl justify-center px-4 sm:px-6">
                  <div className="inline-flex items-center gap-2 h-[30px] px-3 rounded-full border border-white/[0.08] bg-white/[0.03] text-neutral-400 text-xs">
                    <FileDiff size={12} aria-hidden="true" />
                    <span>{`${workspace.dirtyFiles} file${workspace.dirtyFiles === 1 ? "" : "s"}`}</span>
                    <em className="not-italic font-mono text-[11px]"><b className="text-emerald-400">+{workspace.additions}</b> <b className="text-red-400">−{workspace.deletions}</b></em>
                  </div>
                </div>}
                <div className="relative mx-auto max-w-2xl">
                  {!slashOpen && skillSuggestions.length > 0 && <div className="absolute bottom-full left-4 right-4 z-20 mb-2 overflow-hidden rounded-2xl border border-white/[0.08] bg-[#0c0c10]/95 shadow-2xl shadow-black/40 backdrop-blur-xl sm:left-6 sm:right-6"><div className="border-b border-white/[0.06] px-3 py-1.5 text-[9px] uppercase tracking-[0.12em] text-neutral-600">Available skills for this task</div>{skillSuggestions.map(suggestion => <button key={suggestion.id} type="button" onMouseDown={event => { event.preventDefault(); setComposer(current => `/${suggestion.command} ${current}`); setSkillSuggestions([]); }} className="flex w-full items-start gap-3 border-b border-white/[0.045] px-3 py-2 text-left last:border-0 hover:bg-white/[0.05]"><span className="mt-0.5 rounded border border-emerald-400/15 bg-emerald-400/[0.05] px-1.5 py-0.5 text-[8.5px] uppercase text-emerald-300">installed</span><span className="min-w-0 flex-1"><b className="block truncate text-[11px] font-medium text-neutral-200">{suggestion.name}</b><small className="mt-0.5 block text-[9.5px] leading-4 text-neutral-500">{suggestion.relevance} · {suggestion.source} · {suggestion.risk} risk · {suggestion.permissions.join(", ")}</small></span></button>)}</div>}
                  {slashOpen && <div className="absolute left-4 right-4 sm:left-6 sm:right-6 bottom-full mb-2 z-20 rounded-2xl border border-white/[0.08] bg-[#0c0c10]/95 backdrop-blur-xl shadow-2xl shadow-black/40 overflow-hidden flex flex-col max-h-[min(420px,55vh)]">
                    <div className="shrink-0 px-3 py-1.5 text-[9px] uppercase tracking-[0.12em] text-neutral-600 border-b border-white/[0.06] flex items-center gap-2">
                      <span>Commands & skills</span>
                      <span className="normal-case tracking-normal text-neutral-700">{slashMatches.length}</span>
                    </div>
                    <div ref={slashListRef} className="min-h-0 flex-1 overflow-y-auto overscroll-contain scrollbar-thin scrollbar-thumb-white/10" onWheel={e => e.stopPropagation()}>
                      {slashMatches.map((command, index) => <button key={`${command.harness}:${command.kind}:${command.name}`} type="button" data-slash-index={index} onMouseEnter={() => setSlashIndex(index)} onMouseDown={e => { e.preventDefault(); void applySlash(command); }} className={`w-full flex items-center gap-2 px-3 py-2 text-left transition-colors ${index === slashIndex ? "bg-white/[0.08]" : "hover:bg-white/[0.05]"}`}>
                        <span className="font-mono text-[12px] text-neutral-100 whitespace-nowrap">/{command.name}</span>
                        <span className="flex-1 min-w-0 text-[11px] text-neutral-500 whitespace-nowrap overflow-hidden text-ellipsis">{command.description}</span>
                        <span className="shrink-0 text-[8.5px] uppercase tracking-[0.06em] text-neutral-500 border border-white/[0.08] rounded px-1 py-[1px]">{harnessLabel(command.harness)}</span>
                      </button>)}
                    </div>
                  </div>}
                  <ComposerPill
                    layout="dock"
                    value={composer}
                    onChange={value => { setComposer(value); setSlashDismissed(false); setSlashIndex(0); }}
                    onSubmit={() => void sendPrompt()}
                    onKeyDown={onComposerKeyDown}
                    placeholder={isDirectChat ? "Ask Bridge…" : sessionConnected ? "Message…" : "Message…  (starts the agent)"}
                    disabled={!session}
                    working={!!session?.activeTurnId}
                    onStop={session ? () => void bridgeApi.interruptTurn(session.id) : undefined}
                    onPlusClick={() => { setComposer(""); setSlashDismissed(false); }}
                    trailing={isDirectChat
                      ? <ChatModelControl adapters={adapters} harness={session.harness} model={session.model ?? null} disabled={busy} onChange={(harness, model) => void changeChatModel(harness, model)} compact />
                      : <span className="inline-flex items-center gap-1 h-8 px-2.5 text-foreground/75 text-[13px] rounded-full">{session.kind === "orchestrator" ? "Orchestrator" : harnessLabel(session.harness)}</span>}
                  />
                </div>
              </div>
            </>}
            {hasRepo && workspace && activeTab === "changes" && <ChangesPanel workspace={workspace}/>}
            {hasRepo && workspace && activeTab === "terminal" && <div className="absolute inset-0"><Suspense fallback={<PanelLoading label="Opening terminal…"/>}><TerminalPane workspaceId={workspace.id}/></Suspense></div>}
          </div>
        </section>
      </> : <Welcome adapters={adapters} busy={busy} onStartChat={text => void openNewChat(text)} onNewWorkspace={() => { setTitle(""); setModal("workspace"); }}/>}
    </main>
    {error && (() => {
      const described = describeError(error, {
        provider: session ? harnessLabel(session.harness) : undefined,
        snapshot: session ? usageByProvider[session.harness as UsageProvider] : undefined,
      });
      return (
        <Alert variant={described.kind === "usage-limit" ? "warning" : "error"} className="fixed right-[18px] bottom-[18px] z-40 max-w-[520px] bg-card/70 backdrop-blur-2xl backdrop-saturate-150 border-foreground/10 shadow-[0_24px_70px_-20px_rgba(0,0,0,0.65)]">
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
    <RouterSettingsDialog open={modal === "router"} workspaceId={workspace?.id} adapters={adapters} onClose={() => setModal(null)} onError={setError} />
  </div>;
}

function PanelLoading({ label }: { label: string }) {
  return <div role="status" className="absolute inset-0 grid place-items-center text-xs text-muted-foreground">{label}</div>;
}


function ChatModelControl({ adapters, harness, model, disabled, onChange, compact }: { adapters: import("./types").AdapterDescriptor[]; harness: Harness; model: string | null; disabled?: boolean; onChange: (harness: Harness, model: string | null) => void; compact?: boolean }) {
  const [open, setOpen] = useState(false);
  const chatAdapters = adapters.filter(adapter => adapter.id === "codex" || adapter.id === "claude");
  const current = chatAdapters.find(adapter => adapter.id === harness);
  const currentModel = current?.models.find(option => option.id === model) ?? current?.models.find(option => option.defaultForTier) ?? current?.models[0];
  const modelLabel = currentModel?.label ?? model ?? "Default";
  const tierLabel = currentModel?.tier === "strong" ? "High" : currentModel?.tier === "standard" ? "Balanced" : "Fast";
  const compactLabel = compact ? tierLabel : `${harnessLabel(harness)} · ${modelLabel}`;
  return <div className="relative">
    <button type="button" disabled={disabled} onClick={() => setOpen(value => !value)} className={`flex items-center gap-1 rounded-full transition-colors disabled:opacity-45 ${compact ? "h-8 px-2 text-xs text-neutral-400 hover:bg-white/[0.08]" : "h-[28px] max-w-[220px] px-2 text-[11.5px] text-neutral-300 hover:bg-white/[0.06]"}`} title={disabled ? "End the chat to switch models" : "Choose model"}>
      <span className="whitespace-nowrap overflow-hidden text-ellipsis">{compactLabel}</span>
      <ChevronDown size={compact ? 14 : 12} className={`shrink-0 text-muted-foreground/55 transition-transform ${open ? "rotate-180" : ""}`} aria-hidden="true" />
    </button>
    {open && <>
      <div className="fixed inset-0 z-30" onClick={() => setOpen(false)} />
      <div className="absolute left-0 bottom-full mb-2 z-40 w-[280px] py-1.5 rounded-2xl border border-white/[0.1] bg-[#0c0c10]/95 backdrop-blur-xl shadow-2xl shadow-black/40 max-h-[340px] overflow-y-auto scrollbar-thin scrollbar-thumb-white/10">
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

function ChangesPanel({ workspace }: { workspace: Workspace }) { return <div className="p-[38px_44px] max-w-[780px]"><div className="text-muted-foreground/65 text-[10.5px] font-semibold tracking-[0.1em]">CHANGE STORY</div><h2 className="font-heading text-foreground text-[20px] my-2.5 tracking-[-0.015em]">{workspace.dirtyFiles ? `${workspace.dirtyFiles} files changed` : "Workspace is clean"}</h2><p className="text-muted-foreground text-[13px] leading-relaxed max-w-[560px]">Behavior-grouped review will live here. High-risk authentication, migrations, test weakening, and evaluation thresholds are always expanded.</p><div className="mt-6 h-[44px] border border-border flex items-center gap-3 px-3.5 rounded-lg font-mono text-[11.5px]"><b className="text-success font-medium">+{workspace.additions}</b><b className="text-destructive font-medium">−{workspace.deletions}</b><span className="h-[3px] flex-1 rounded-[2px] bg-[linear-gradient(90deg,color-mix(in_srgb,var(--color-success)_55%,transparent)_0_72%,color-mix(in_srgb,var(--color-destructive)_55%,transparent)_72%)]"/><small className="text-muted-foreground">{workspace.branch}</small></div><div className="mt-5 flex flex-col gap-2.5">{[78,92,64,85,51,70].map((n,i)=><i key={i} className="block h-[7px] bg-muted rounded-[3px]" style={{width:`${n}%`}}/>)}</div></div>; }
function EventPanel({ state, workspace }: { state: BridgeState; workspace: Workspace }) { const events = state.events.filter(e => e.entityId === workspace.id || state.sessions.some(s => s.workspaceId === workspace.id && s.id === e.entityId)); return <div className="max-w-[720px] px-8 py-[22px]">{events.length ? events.map(e => <article key={e.id} className="flex gap-3 py-[13px] border-b border-border text-muted-foreground"><CircleDot size={14} aria-hidden="true" /><div><b className="text-foreground text-[11px] font-medium tracking-[0.02em] capitalize">{e.kind.replaceAll(".", " ")}</b><p className="text-[12.5px] my-1 text-foreground">{e.body}</p><small className="font-mono text-[10.5px] text-muted-foreground/65">{new Date(e.createdAt).toLocaleTimeString([], {hour:"2-digit",minute:"2-digit"})}</small></div></article>) : <div className="text-muted-foreground text-[12.5px] p-7">No events for this workspace yet.</div>}</div>; }
function WelcomeModelBadge({ adapters }: { adapters: import("./types").AdapterDescriptor[] }) {
  const preferred = adapters.find(adapter => adapter.available) ?? adapters[0];
  const model = preferred?.models.find(option => option.id === preferred.defaultModel) ?? preferred?.models.find(option => option.defaultForTier) ?? preferred?.models[0];
  const tierLabel = model?.tier === "strong" ? "High" : model?.tier === "standard" ? "Balanced" : "Fast";
  return <span className="inline-flex items-center gap-1 rounded-full px-2.5 py-1.5 text-xs text-neutral-400">{tierLabel}<ChevronDown size={14} className="text-neutral-600" aria-hidden="true" /></span>;
}

function Welcome({ adapters, busy, onStartChat, onNewWorkspace }: { adapters: import("./types").AdapterDescriptor[]; busy: boolean; onStartChat: (text?: string) => void; onNewWorkspace: () => void }) {
  const greeting = useMemo(() => pickGreeting("welcome"), []);
  const [draft, setDraft] = useState("");
  const submit = () => {
    const text = draft.trim();
    if (text) onStartChat(text);
    else onStartChat();
    setDraft("");
  };
  return <div className="flex flex-1 flex-col items-center justify-center px-4 text-center animate-page-enter">
    <h1 className="mb-8 max-w-xl font-display text-[1.65rem] font-medium tracking-[-0.02em] text-white sm:mb-10 sm:text-[2.1rem]">{greeting.headline}</h1>
    <ComposerPill
      layout="hero"
      value={draft}
      onChange={setDraft}
      onSubmit={submit}
      onKeyDown={e => { if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) { e.preventDefault(); submit(); } }}
      placeholder="Ask Bridge…"
      disabled={busy}
      onPlusClick={onNewWorkspace}
      trailing={<WelcomeModelBadge adapters={adapters} />}
    />
    <p className="mt-5 max-w-md text-[13px] leading-relaxed text-neutral-500">{greeting.hint}</p>
  </div>;
}
function CommandPalette({ workspaces, onChoose }: { workspaces: Workspace[]; onChoose: (id:string)=>void }) { return <><InputGroup className="border-b border-border rounded-none border-x-0 border-t-0 shadow-none"><InputGroupInput autoFocus placeholder="Search workspaces and actions…" /><InputGroupAddon><Search size={17} aria-hidden="true" /></InputGroupAddon></InputGroup><div className="p-[9px]"><label className="block p-[5px_9px_7px] text-muted-foreground/65 text-[10px] font-semibold tracking-[0.09em]">WORKSPACES</label>{workspaces.map(w => <Button type="button" key={w.id} variant="ghost" className="w-full h-[44px] rounded-md justify-start px-2.5" onClick={() => onChoose(w.id)}><StatusDot status={w.status}/><span className="flex flex-col gap-[3px] flex-1 text-left"><b className="text-[12.5px] font-medium">{w.title}</b><small className="text-[10.5px] text-muted-foreground">{w.city} · {w.branch}</small></span><Kbd className="font-mono text-muted-foreground/65 border border-border rounded px-1 py-[1px] text-[10px]">↵</Kbd></Button>)}</div><div className="h-[32px] border-t border-border flex items-center gap-[14px] px-[13px] text-muted-foreground/65 text-[10.5px]"><span>↑↓ navigate</span><span>esc close</span></div></>; }
