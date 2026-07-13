import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Activity, Archive, ArrowUp, Bot, Check, ChevronDown, ChevronRight, CircleDot, Clock3, FileCode2, FileDiff, FileText, FolderGit2, Gauge, GitBranch, GitCommitHorizontal, GitPullRequest, Inbox, LayoutGrid, LoaderCircle, MessageSquarePlus, MessageSquareText, Monitor, PanelLeft, Play, Plus, Search, Settings2, Square, TerminalSquare, X } from "lucide-react";
import { bridgeApi } from "./api";
import type { AgentEvent, BridgeState, Harness, Health, Project, Session, SessionForestSnapshot, SessionStatus, Workspace } from "./types";
import { AgentConversation } from "./components/AgentConversation";
import { TerminalPane } from "./components/TerminalPane";
import { formatElapsed, tierRuntimeLabel } from "./utils";
import { projectSessionConversation, reduceConversation } from "./conversation";
import { pickGreeting } from "./greetings";
import { extractUsageSnapshot, formatReset, type UsageProvider, type UsageSnapshot } from "./usage";
import { queueExplanation, restorationPresentation, turnBudget } from "./observability";
import { Alert, AlertAction, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogClose, DialogDescription, DialogFooter, DialogHeader, DialogPanel, DialogPopup, DialogTitle } from "@/components/ui/dialog";
import { InputGroup, InputGroupAddon, InputGroupInput } from "@/components/ui/input-group";
import { Kbd } from "@/components/ui/kbd";
import { Label } from "@/components/ui/label";
import { Tabs, TabsList, TabsTab } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";

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

export function App() {
  const [state, setState] = useState<BridgeState>(emptyState);
  const [agentEvents, setAgentEvents] = useState<AgentEvent[]>([]);
  const [health, setHealth] = useState<Health>();
  const [selectedSessionId, setSelectedSessionId] = useState<string>();
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [activeTab, setActiveTab] = useState<"agent" | "changes" | "events" | "terminal">("agent");
  const [modal, setModal] = useState<"chat" | "workspace" | null>(null);
  const [title, setTitle] = useState("");
  const [composer, setComposer] = useState("");
  const [slashCommands, setSlashCommands] = useState<import("./types").SlashCommand[]>([]);
  const [slashIndex, setSlashIndex] = useState(0);
  const [slashDismissed, setSlashDismissed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [clock, setClock] = useState(Date.now());
  const [forest, setForest] = useState<SessionForestSnapshot>();
  const [pending, setPending] = useState<{ key: string; sessionId: string; text: string }[]>([]);
  const [usageByProvider, setUsageByProvider] = useState<Partial<Record<UsageProvider, UsageSnapshot>>>({});
  const startedRef = useRef<Set<string>>(new Set());

  const reload = useCallback(async () => { setState(await bridgeApi.state()); }, []);
  const errorMessage = (value: unknown) => value instanceof Error ? value.message : String(value);

  useEffect(() => {
    void Promise.all([reload(), bridgeApi.health().then(setHealth)]);
    let offState: (() => void) | undefined;
    let offAgent: (() => void) | undefined;
    let offUsage: (() => void) | undefined;
    void bridgeApi.onStateChanged(reload).then(fn => offState = fn);
    void bridgeApi.onAgentEvent(event => setAgentEvents(current => current.some(item => item.id === event.id) ? current : [...current, event])).then(fn => offAgent = fn);
    void bridgeApi.onAccountUsage(payload => {
      const snapshot = extractUsageSnapshot({ rateLimits: payload.rateLimits });
      if (snapshot && snapshot.windows.length) setUsageByProvider(current => ({ ...current, [payload.provider]: snapshot }));
    }).then(fn => offUsage = fn);
    return () => { offState?.(); offAgent?.(); offUsage?.(); };
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
  const sessionEvents = agentEvents.filter(event => event.sessionId === session?.id);
  const pendingForSession = useMemo(() => pending.filter(p => p.sessionId === session?.id).map(p => p.text), [pending, session?.id]);
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
    if (!session?.id) { setForest(undefined); return; }
    let active = true;
    const refresh = () => void bridgeApi.sessionForest(session.id).then(value => { if (active) setForest(value); }).catch(() => undefined);
    refresh();
    const timer = window.setInterval(refresh, 3000);
    return () => { active = false; window.clearInterval(timer); };
  }, [session?.id]);

  // Keep git stats fresh for the selected chat's connected workspace.
  useEffect(() => {
    const workspaceId = workspace?.id;
    if (!workspaceId || !hasRepo || !("__TAURI_INTERNALS__" in window)) return;
    const refresh = () => void bridgeApi.refreshWorkspace(workspaceId).then(setState).catch(() => undefined);
    refresh(); const timer = window.setInterval(refresh, 5000); return () => window.clearInterval(timer);
  }, [workspace?.id, hasRepo]);

  // Poll real subscription usage for every provider, independent of the chat on screen.
  useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;
    const poll = () => void bridgeApi.refreshAccountUsage().catch(() => undefined);
    poll();
    const timer = window.setInterval(poll, 30_000);
    return () => window.clearInterval(timer);
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

  function openSession(id: string) { setSelectedSessionId(id); }
  // New chat opens instantly (no picker up front): create a direct chat with the
  // default model and select it. The model can be changed inside the chat.
  async function openNewChat() {
    const preferred = adapters.find(adapter => adapter.available) ?? adapters[0];
    const harness = (preferred?.id as Harness) ?? "codex";
    const model = preferred?.defaultModel ?? preferred?.models[0]?.id ?? null;
    setBusy(true); setError(undefined);
    try {
      const next = await bridgeApi.createChat(harness, model, null);
      const created = [...next.sessions].reverse().find(s => !s.parentSessionId && !s.workspaceId);
      setState(next);
      if (created) setSelectedSessionId(created.id);
    } catch (e) { setError(errorMessage(e)); }
    finally { setBusy(false); }
  }
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
  async function sendPrompt() {
    if (!session || !composer.trim()) return;
    const text = composer.trim();
    const key = crypto.randomUUID();
    let target = session;
    setComposer("");
    setSlashIndex(0);
    setPending(current => [...current, { key, sessionId: target.id, text }]);
    try {
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
    catch (e) { setComposer(text); setPending(current => current.filter(item => item.key !== key)); setError(errorMessage(e)); }
  }
  async function resolveApproval(eventId: number, decision: string) { if (!session) return; try { await bridgeApi.resolveApproval(session.id, eventId, decision); await reload(); } catch (e) { setError(errorMessage(e)); } }
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

  return <div className="h-screen grid grid-rows-[44px_1fr] grid-cols-[1fr] sm:grid-cols-[248px_1fr] bg-[radial-gradient(circle_at_18%_-8%,color-mix(in_srgb,var(--color-foreground)_6%,transparent),transparent_42%),radial-gradient(circle_at_100%_0%,color-mix(in_srgb,var(--color-foreground)_4%,transparent),transparent_38%),color-mix(in_srgb,var(--color-background)_46%,transparent)]">
    <header className="col-span-full flex items-center gap-2 border-b border-foreground/8 bg-background/35 backdrop-blur-[34px] backdrop-saturate-[1.7] select-none relative z-20 u-hairline-t" data-tauri-drag-region>
      <div className="w-[72px] h-full shrink-0" data-tauri-drag-region />
      <div className="w-auto flex items-center shrink-0"><strong className="text-[13px] font-semibold text-foreground tracking-[-0.02em]">Bridge</strong></div>
      <div className="ml-auto flex items-center h-full pr-3"><UsageWidget usage={usageByProvider}/></div>
    </header>
    <aside className="hidden sm:flex row-start-2 bg-sidebar/45 backdrop-blur-[36px] backdrop-saturate-[1.8] border-r border-foreground/8 flex-col min-h-0 shadow-[inset_-1px_0_0_color-mix(in_srgb,var(--color-foreground)_5%,transparent),inset_1px_0_0_color-mix(in_srgb,var(--color-foreground)_6%,transparent)] transition-colors duration-300">
      <div className="h-[42px] px-2.5 flex items-center gap-2 shrink-0">
        <Button type="button" size="sm" className="flex-1 justify-start h-[30px] gap-2 rounded-lg" onClick={() => openNewChat()}><MessageSquarePlus size={15} aria-hidden="true" /> New chat</Button>
      </div>
      <div className="flex-1 overflow-auto px-[7px] pb-2 scrollbar-thin scrollbar-thumb-foreground/10">
        <div className="h-[26px] flex items-center gap-2 px-[9px] text-muted-foreground/60 text-[9px] font-semibold tracking-[0.14em] uppercase">Chats</div>
        {standaloneChats.map(chat => <ChatRow key={chat.id} chat={chat} active={chat.id === session?.id} onClick={() => openSession(chat.id)} />)}
        {!standaloneChats.length && <div className="px-[10px] py-1.5 text-muted-foreground/50 text-[11px]">No chats yet.</div>}

        <div className="h-[26px] mt-3 flex items-center gap-2 px-[9px] text-muted-foreground/60 text-[9px] font-semibold tracking-[0.14em] uppercase">
          <span className="flex-1">Workspaces</span>
          <button type="button" className="text-muted-foreground/60 hover:text-foreground transition-colors" onClick={() => { setTitle(""); setModal("workspace"); }} title="New workspace" aria-label="New workspace"><Plus size={13} aria-hidden="true" /></button>
        </div>
        {state.workspaces.map(ws => {
          const chats = topSessions.filter(s => s.workspaceId === ws.id);
          const open = expanded.has(ws.id);
          return <section key={ws.id} className="mb-0.5">
            <div className="w-full h-[34px] rounded-lg flex items-center gap-1.5 px-[7px] transition-colors hover:bg-foreground/[0.05] group/ws">
              <button type="button" className="flex-1 min-w-0 flex items-center gap-1.5 text-left" onClick={() => toggleExpanded(ws.id)}>
                <ChevronRight size={13} className={`text-muted-foreground/60 transition-transform ${open ? "rotate-90" : ""}`} aria-hidden="true" />
                <FolderGit2 size={13} className="text-muted-foreground/70" aria-hidden="true" />
                <b className="flex-1 min-w-0 text-[12px] text-foreground font-medium whitespace-nowrap overflow-hidden text-ellipsis">{ws.title}</b>
              </button>
              <span className="text-muted-foreground/50 text-[10px] font-mono group-hover/ws:hidden">{chats.length || ""}</span>
              <button type="button" className="hidden group-hover/ws:flex text-muted-foreground/60 hover:text-foreground transition-colors" title="New agent in this workspace" aria-label="New agent" disabled={busy} onClick={() => void newWorkspaceSession(ws.id)}><Plus size={14} aria-hidden="true" /></button>
            </div>
            {open && <div className="ml-[13px] pl-2 border-l border-foreground/8">
              {chats.map(chat => <ChatRow key={chat.id} chat={chat} active={chat.id === session?.id} onClick={() => openSession(chat.id)} />)}
              <div className="flex items-center gap-1 py-0.5">
                <button type="button" className="flex items-center gap-1.5 px-[9px] h-[26px] rounded-md text-[11px] text-muted-foreground/70 hover:text-foreground hover:bg-foreground/[0.05] transition-colors" disabled={busy} onClick={() => void newWorkspaceSession(ws.id)}><Plus size={12} aria-hidden="true" /> New agent</button>
                {!ws.path && <button type="button" className="flex items-center gap-1.5 px-[9px] h-[26px] rounded-md text-[11px] text-muted-foreground/70 hover:text-foreground hover:bg-foreground/[0.05] transition-colors" onClick={() => void connectFolder(ws.id)}><FolderGit2 size={12} aria-hidden="true" /> Connect folder</button>}
              </div>
              {ws.path && <div className="px-[9px] py-1 text-[9.5px] text-muted-foreground/50 font-mono flex items-center gap-1 whitespace-nowrap overflow-hidden text-ellipsis"><GitBranch size={10} aria-hidden="true" />{ws.branch ?? "folder"} · {ws.dirtyFiles ? `${ws.dirtyFiles} changed` : "clean"}</div>}
            </div>}
          </section>;
        })}
        {!state.workspaces.length && <div className="px-[10px] py-1.5 text-muted-foreground/50 text-[11px]">Group chats and connect a repo with a workspace.</div>}
      </div>
    </aside>
    <main className="row-start-2 min-w-0 min-h-0 flex flex-col bg-transparent">
      {session ? <>
        <div className={`shrink-0 px-[18px] flex items-center border-b border-border/70 ${isDirectChat ? "h-[48px]" : "min-h-[58px] py-[9px]"}`}>
          <div className="min-w-0 flex-1">
            <h1 className="m-0 font-heading text-[14px] leading-tight text-foreground font-semibold tracking-[-0.015em] whitespace-nowrap overflow-hidden text-ellipsis">{session.title || session.label}</h1>
            {!isDirectChat && <div className="mt-[3px] flex items-center gap-1.5 text-muted-foreground font-mono text-[9.5px]">
              <Bot size={12} aria-hidden="true" />{session.kind === "orchestrator" ? "Orchestrator" : harnessLabel(session.harness)}
              {hasRepo && workspace && <><span>·</span><GitBranch size={12} aria-hidden="true" />{workspace.branch ?? "folder"}<span>·</span>{workspace.dirtyFiles ? <span className="text-warning">{workspace.dirtyFiles} changed</span> : <span>clean</span>}</>}
            </div>}
          </div>
          <div className="ml-auto flex items-center gap-[7px]">
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
        <section className="flex-1 min-h-0 overflow-hidden flex border-t border-border/50">
          <div className="flex-1 min-w-0 flex flex-col relative">
            {(activeTab === "agent" || !hasRepo) && <>
              <div className="flex-1 min-h-0 relative">
                <AgentConversation
                  session={session}
                  events={sessionEvents}
                  forestEntries={forest?.entries}
                  activeLeafId={forest?.head?.activeEntryId}
                  repositoryDivergence={forest?.repositoryDivergence.status}
                  continuationFidelity={session?.continuationFidelity}
                  preview={false}
                  working={!!session?.activeTurnId || pendingForSession.length > 0}
                  pendingMessages={pendingForSession}
                  onResolve={(eventId, decision) => void resolveApproval(eventId, decision)}
                />
              </div>
              <div className={`flex-none ${isDirectChat ? "pb-5 px-5" : "pb-4 px-7"}`}>
                {hasRepo && workspace && workspace.dirtyFiles > 0 && <div className="max-w-[720px] mx-auto flex justify-center -mb-2 relative z-10">
                  <div className="inline-flex items-center gap-2 h-[30px] px-[13px] border border-foreground/10 rounded-full bg-card/60 backdrop-blur-xl shadow-[0_10px_28px_-10px_rgba(0,0,0,0.55)] text-muted-foreground text-xs">
                    <FileDiff size={12} className="text-muted-foreground/65" aria-hidden="true" />
                    <span>{`${workspace.dirtyFiles} file${workspace.dirtyFiles === 1 ? "" : "s"}`}</span>
                    <em className="not-italic font-mono text-[11.5px]"><b className="text-success font-medium">+{workspace.additions}</b> <b className="text-destructive font-medium">−{workspace.deletions}</b></em>
                  </div>
                </div>}
                <div className="max-w-[720px] mx-auto relative">
                  {slashOpen && <div className="absolute left-0 right-0 bottom-full mb-2 z-20 rounded-xl border border-foreground/10 bg-popover/95 backdrop-blur-2xl backdrop-saturate-150 shadow-[0_20px_55px_-18px_rgba(0,0,0,0.65)] overflow-hidden flex flex-col max-h-[min(420px,55vh)]">
                    <div className="shrink-0 px-3 py-1.5 text-[9px] uppercase tracking-[0.12em] text-muted-foreground/50 border-b border-border/60 flex items-center gap-2">
                      <span>Commands & skills</span>
                      <span className="normal-case tracking-normal text-muted-foreground/35">{slashMatches.length}</span>
                    </div>
                    <div ref={slashListRef} className="min-h-0 flex-1 overflow-y-auto overscroll-contain scrollbar-thin scrollbar-thumb-foreground/15" onWheel={e => e.stopPropagation()}>
                      {slashMatches.map((command, index) => <button key={`${command.harness}:${command.kind}:${command.name}`} type="button" data-slash-index={index} onMouseEnter={() => setSlashIndex(index)} onMouseDown={e => { e.preventDefault(); void applySlash(command); }} className={`w-full flex items-center gap-2 px-3 py-2 text-left transition-colors ${index === slashIndex ? "bg-foreground/[0.08]" : "hover:bg-foreground/[0.05]"}`}>
                        <span className="font-mono text-[12px] text-foreground whitespace-nowrap">/{command.name}</span>
                        <span className="flex-1 min-w-0 text-[11px] text-muted-foreground/70 whitespace-nowrap overflow-hidden text-ellipsis">{command.description}</span>
                        <span className="shrink-0 text-[8.5px] uppercase tracking-[0.06em] text-muted-foreground/70 border border-border rounded px-1 py-[1px]">{harnessLabel(command.harness)}</span>
                        <span className="shrink-0 text-[8.5px] uppercase tracking-[0.06em] text-muted-foreground/45">{command.kind}</span>
                      </button>)}
                    </div>
                  </div>}
                  <div className="border border-foreground/10 bg-card/60 backdrop-blur-2xl rounded-2xl shadow-[0_18px_50px_-22px_rgba(0,0,0,0.55)] transition-colors focus-within:border-foreground/18 focus-within:bg-card/70">
                    <Textarea className="w-full [&>textarea]:min-h-[48px] [&>textarea]:max-h-[200px] [&>textarea]:resize-none [&>textarea]:bg-transparent [&>textarea]:text-foreground [&>textarea]:px-[18px] [&>textarea]:pt-3.5 [&>textarea]:pb-1 [&>textarea]:text-[14px] [&>textarea]:leading-relaxed [&>textarea]:placeholder:text-muted-foreground/45" unstyled value={composer} onChange={e => { setComposer(e.target.value); setSlashDismissed(false); setSlashIndex(0); }} onKeyDown={onComposerKeyDown} placeholder={isDirectChat ? "Message Bridge…" : sessionConnected ? "Message…" : "Message…  (starts the agent)"} disabled={!session} />
                    <div className="h-11 flex items-center gap-2 px-2.5">
                      {isDirectChat
                        ? <ChatModelControl adapters={adapters} harness={session.harness} model={session.model ?? null} disabled={busy} onChange={(harness, model) => void changeChatModel(harness, model)} />
                        : <span className="inline-flex items-center gap-1.5 h-[28px] px-2.5 text-muted-foreground text-[11.5px] rounded-full border border-foreground/8">{session.kind === "orchestrator" ? "Orchestrator" : harnessLabel(session.harness)}</span>}
                      <span className="ml-auto text-muted-foreground/50 text-[10.5px] hidden sm:inline">{slashOpen ? "↑↓ · ↵" : "/ commands"}</span>
                      {session?.activeTurnId
                        ? <Button type="button" size="icon-sm" className="rounded-full" onClick={() => void bridgeApi.interruptTurn(session.id)} title="Stop turn" aria-label="Stop turn"><Square size={11} fill="currentColor" aria-hidden="true" /></Button>
                        : <Button type="button" size="icon-sm" className="rounded-full" onClick={() => void sendPrompt()} disabled={!composer.trim() || !session} aria-label="Send message"><ArrowUp size={15} aria-hidden="true" /></Button>}
                    </div>
                  </div>
                </div>
              </div>
            </>}
            {hasRepo && workspace && activeTab === "changes" && <ChangesPanel workspace={workspace}/>}
            {hasRepo && workspace && activeTab === "terminal" && <div className="absolute inset-0"><TerminalPane workspaceId={workspace.id}/></div>}
          </div>
        </section>
      </> : <Welcome onStartChat={() => openNewChat()} onNewWorkspace={() => { setTitle(""); setModal("workspace"); }}/>}
    </main>
    {error && (
      <Alert variant="error" className="fixed right-[18px] bottom-[18px] z-40 max-w-[520px] bg-card/70 backdrop-blur-2xl backdrop-saturate-150 border-foreground/10 shadow-[0_24px_70px_-20px_rgba(0,0,0,0.65)]">
        <AlertDescription>{error}</AlertDescription>
        <AlertAction>
          <Button type="button" size="icon-sm" variant="ghost" aria-label="Dismiss error" onClick={() => setError(undefined)}><X size={14} aria-hidden="true" /></Button>
        </AlertAction>
      </Alert>
    )}

    <Dialog open={modal === "workspace"} onOpenChange={open => !open && setModal(null)}>
      <DialogPopup className="bg-popover/75 backdrop-blur-2xl backdrop-saturate-150 border-foreground/10 shadow-[0_32px_90px_-24px_rgba(0,0,0,0.7)]">
        <DialogHeader>
          <DialogTitle>New workspace</DialogTitle>
          <DialogDescription>Group related chats. Connect a folder or git repo later—optional.</DialogDescription>
        </DialogHeader>
        <DialogPanel>
          <Label className="block text-muted-foreground/65 text-[10.5px] font-semibold tracking-[0.09em] m-[0_0_7px]">WORKSPACE NAME</Label>
          <InputGroup>
            <InputGroupInput autoFocus value={title} onChange={e => setTitle(e.target.value)} placeholder="e.g. Payments service" onKeyDown={e => { if (e.key === "Enter") { e.preventDefault(); if (!busy && title.trim()) void submitNewWorkspace(); } }} />
          </InputGroup>
        </DialogPanel>
        <DialogFooter>
          <DialogClose render={<Button type="button" variant="ghost" />}>Cancel</DialogClose>
          <Button type="button" loading={busy} disabled={!title.trim()} onClick={() => void submitNewWorkspace()}>{busy ? "Creating…" : "Create workspace"}</Button>
        </DialogFooter>
      </DialogPopup>
    </Dialog>
  </div>;
}

function ChatRow({ chat, active, onClick }: { chat: Session; active: boolean; onClick: () => void }) {
  return <button type="button" onClick={onClick} className={`w-full min-h-[34px] rounded-lg flex items-center gap-2 px-[9px] py-1 text-left my-[1px] transition-colors hover:bg-foreground/[0.055] ${active ? "bg-foreground/[0.09] shadow-[0_1px_2px_rgba(0,0,0,0.18)]" : ""}`}>
    <StatusDot status={chat.status}/>
    <span className="min-w-0 flex-1 flex flex-col gap-[1px]">
      <b className="text-[12px] text-foreground font-medium whitespace-nowrap overflow-hidden text-ellipsis">{chat.title || chat.label}</b>
      <small className="text-[9px] text-muted-foreground/70 whitespace-nowrap overflow-hidden text-ellipsis">{harnessLabel(chat.harness)}{chat.model ? ` · ${chat.model}` : ""}</small>
    </span>
  </button>;
}

function ChatModelControl({ adapters, harness, model, disabled, onChange }: { adapters: import("./types").AdapterDescriptor[]; harness: Harness; model: string | null; disabled?: boolean; onChange: (harness: Harness, model: string | null) => void }) {
  const [open, setOpen] = useState(false);
  const chatAdapters = adapters.filter(adapter => adapter.id === "codex" || adapter.id === "claude");
  const current = chatAdapters.find(adapter => adapter.id === harness);
  const modelLabel = current?.models.find(option => option.id === model)?.label ?? model ?? "Default";
  return <div className="relative">
    <button type="button" disabled={disabled} onClick={() => setOpen(value => !value)} className="flex items-center gap-1.5 h-[28px] max-w-[220px] px-2 rounded-lg text-[11.5px] text-foreground/90 hover:bg-foreground/[0.06] transition-colors disabled:opacity-45 disabled:hover:bg-transparent" title={disabled ? "End the chat to switch models" : "Choose model"}>
      <span className="whitespace-nowrap overflow-hidden text-ellipsis">{harnessLabel(harness)} · {modelLabel}</span>
      <ChevronDown size={12} className={`shrink-0 text-muted-foreground/55 transition-transform ${open ? "rotate-180" : ""}`} aria-hidden="true" />
    </button>
    {open && <>
      <div className="fixed inset-0 z-30" onClick={() => setOpen(false)} />
      <div className="absolute left-0 bottom-full mb-2 z-40 w-[280px] py-1.5 rounded-xl border border-foreground/10 bg-popover/92 backdrop-blur-2xl backdrop-saturate-150 shadow-[0_24px_70px_-20px_rgba(0,0,0,0.65)] max-h-[340px] overflow-y-auto scrollbar-thin scrollbar-thumb-foreground/10">
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

const PROVIDER_LABEL: Record<UsageProvider, string> = { claude: "Claude", codex: "Codex" };

// Monochrome-friendly gradient that brightens as a limit fills up.
function barTone(used: number): string {
  if (used >= 90) return "from-foreground/75 to-foreground";
  if (used >= 70) return "from-foreground/45 to-foreground/85";
  return "from-foreground/25 to-foreground/55";
}

function UsageRing({ used, size = 18 }: { used: number; size?: number }) {
  const clamped = Math.min(100, Math.max(0, used));
  const radius = 7;
  const circumference = 2 * Math.PI * radius;
  const offset = circumference * (1 - clamped / 100);
  const tone = clamped >= 90 ? "text-foreground" : clamped >= 70 ? "text-foreground/85" : "text-foreground/60";
  return <svg width={size} height={size} viewBox="0 0 18 18" className="shrink-0 -rotate-90" aria-hidden="true">
    <circle cx={9} cy={9} r={radius} fill="none" strokeWidth={2} stroke="currentColor" className="text-foreground/12" />
    <circle cx={9} cy={9} r={radius} fill="none" strokeWidth={2} strokeLinecap="round" stroke="currentColor" strokeDasharray={circumference} strokeDashoffset={offset} className={`${tone} transition-[stroke-dashoffset] duration-700 ease-out`} />
  </svg>;
}

function UsageBar({ used }: { used: number }) {
  return <span className="block w-full h-[5px] rounded-full bg-foreground/[0.08] overflow-hidden">
    <span className={`block h-full rounded-full bg-gradient-to-r ${barTone(used)} transition-[width] duration-700 ease-out`} style={{ width: `${Math.min(100, Math.max(0, used))}%` }} />
  </span>;
}

function UsageWidget({ usage }: { usage: Partial<Record<UsageProvider, UsageSnapshot>> }) {
  const entries = (Object.keys(PROVIDER_LABEL) as UsageProvider[])
    .map(id => ({ id, snapshot: usage[id] }))
    .filter((entry): entry is { id: UsageProvider; snapshot: UsageSnapshot } => !!entry.snapshot && entry.snapshot.windows.length > 0);
  if (!entries.length) {
    return <div className="flex items-center gap-1.5 h-[28px] px-3 rounded-full border border-border/60 bg-foreground/[0.02] text-muted-foreground/50 text-[10px] font-mono select-none" title="Subscription usage appears here once the agent reports it.">
      <Gauge size={11} aria-hidden="true" /><span className="tracking-[0.03em]">Usage</span>
    </div>;
  }
  return <div className="group relative flex items-center h-full">
    <div className="flex items-center gap-2.5 h-[28px] px-3 rounded-full border border-foreground/[0.09] bg-foreground/[0.04] backdrop-blur-md cursor-default select-none transition-colors group-hover:border-foreground/15 group-hover:bg-foreground/[0.07]">
      {entries.map((entry, index) => <ProviderChip key={entry.id} label={PROVIDER_LABEL[entry.id]} snapshot={entry.snapshot} divided={index > 0} />)}
    </div>
    <div className="absolute right-0 top-full pt-2 z-50 invisible opacity-0 translate-y-1 scale-[0.98] transition-all duration-150 ease-out group-hover:visible group-hover:opacity-100 group-hover:translate-y-0 group-hover:scale-100">
      <div className="w-[318px] p-4 rounded-2xl border border-foreground/10 bg-popover/70 backdrop-blur-2xl backdrop-saturate-150 shadow-[0_28px_80px_-20px_rgba(0,0,0,0.65)] u-hairline-t">
        <div className="flex items-center gap-1.5 text-muted-foreground/55 text-[9px] font-semibold tracking-[0.14em] uppercase mb-3">
          <Gauge size={11} aria-hidden="true" /> Subscription usage
        </div>
        {entries.map((entry, index) => <ProviderDetail key={entry.id} label={PROVIDER_LABEL[entry.id]} snapshot={entry.snapshot} divided={index > 0} />)}
      </div>
    </div>
  </div>;
}

function ProviderChip({ label, snapshot, divided }: { label: string; snapshot: UsageSnapshot; divided: boolean }) {
  const used = Math.round(Math.max(...snapshot.windows.map(window => window.usedPercent)));
  const left = Math.max(0, 100 - used);
  return <>
    {divided && <span className="w-px h-4 bg-border/70" aria-hidden="true" />}
    <div className="flex items-center gap-[7px]">
      <UsageRing used={used} />
      <div className="flex flex-col leading-none gap-[3px]">
        <span className="text-[10px] text-foreground font-medium tracking-[0.01em]">{label}</span>
        <span className="text-[9px] font-mono text-muted-foreground/80 tabular-nums">{left}% left</span>
      </div>
    </div>
  </>;
}

function ProviderDetail({ label, snapshot, divided }: { label: string; snapshot: UsageSnapshot; divided: boolean }) {
  const used = Math.round(Math.max(...snapshot.windows.map(window => window.usedPercent)));
  return <div className={divided ? "mt-3.5 pt-3.5 border-t border-border/70" : ""}>
    <div className="flex items-center gap-2 mb-2.5">
      <UsageRing used={used} size={16} />
      <b className="text-[11.5px] text-foreground font-semibold tracking-[-0.01em]">{label}</b>
      <span className="ml-auto text-[9px] font-mono text-muted-foreground/70 tabular-nums">{Math.max(0, 100 - used)}% left</span>
      {snapshot.planType && <span className="px-1.5 py-[1px] rounded-full border border-border/80 text-[8.5px] uppercase tracking-[0.06em] text-muted-foreground/70">{snapshot.planType}</span>}
    </div>
    <div className="grid gap-2.5">
      {snapshot.windows.map(window => {
        const windowUsed = Math.round(window.usedPercent);
        const reset = window.resetsLabel ?? formatReset(window.resetsInSeconds);
        return <div key={window.id}>
          <div className="flex items-baseline justify-between text-[10.5px] mb-1.5"><span className="text-muted-foreground">{window.label}</span><span className="font-mono text-foreground/90 tabular-nums">{windowUsed}%</span></div>
          <UsageBar used={windowUsed} />
          {reset && <div className="text-[9.5px] text-muted-foreground/55 mt-1">{reset}</div>}
        </div>;
      })}
    </div>
  </div>;
}

function ChangesPanel({ workspace }: { workspace: Workspace }) { return <div className="p-[38px_44px] max-w-[780px]"><div className="text-muted-foreground/65 text-[10.5px] font-semibold tracking-[0.1em]">CHANGE STORY</div><h2 className="font-heading text-foreground text-[20px] my-2.5 tracking-[-0.015em]">{workspace.dirtyFiles ? `${workspace.dirtyFiles} files changed` : "Workspace is clean"}</h2><p className="text-muted-foreground text-[13px] leading-relaxed max-w-[560px]">Behavior-grouped review will live here. High-risk authentication, migrations, test weakening, and evaluation thresholds are always expanded.</p><div className="mt-6 h-[44px] border border-border flex items-center gap-3 px-3.5 rounded-lg font-mono text-[11.5px]"><b className="text-success font-medium">+{workspace.additions}</b><b className="text-destructive font-medium">−{workspace.deletions}</b><span className="h-[3px] flex-1 rounded-[2px] bg-[linear-gradient(90deg,color-mix(in_srgb,var(--color-success)_55%,transparent)_0_72%,color-mix(in_srgb,var(--color-destructive)_55%,transparent)_72%)]"/><small className="text-muted-foreground">{workspace.branch}</small></div><div className="mt-5 flex flex-col gap-2.5">{[78,92,64,85,51,70].map((n,i)=><i key={i} className="block h-[7px] bg-muted rounded-[3px]" style={{width:`${n}%`}}/>)}</div></div>; }
function EventPanel({ state, workspace }: { state: BridgeState; workspace: Workspace }) { const events = state.events.filter(e => e.entityId === workspace.id || state.sessions.some(s => s.workspaceId === workspace.id && s.id === e.entityId)); return <div className="max-w-[720px] px-8 py-[22px]">{events.length ? events.map(e => <article key={e.id} className="flex gap-3 py-[13px] border-b border-border text-muted-foreground"><CircleDot size={14} aria-hidden="true" /><div><b className="text-foreground text-[11px] font-medium tracking-[0.02em] capitalize">{e.kind.replaceAll(".", " ")}</b><p className="text-[12.5px] my-1 text-foreground">{e.body}</p><small className="font-mono text-[10.5px] text-muted-foreground/65">{new Date(e.createdAt).toLocaleTimeString([], {hour:"2-digit",minute:"2-digit"})}</small></div></article>) : <div className="text-muted-foreground text-[12.5px] p-7">No events for this workspace yet.</div>}</div>; }
function Welcome({ onStartChat, onNewWorkspace }: { onStartChat: () => void; onNewWorkspace: () => void }) {
  const greeting = useMemo(() => pickGreeting("welcome"), []);
  return <div className="flex-1 flex flex-col items-center justify-center text-center px-6">
    <div className="animate-home-rise flex flex-col items-center">
      <div className="w-11 h-11 grid place-items-center mb-5 rounded-2xl border border-foreground/10 bg-foreground/[0.04] backdrop-blur-md text-muted-foreground/80 shadow-[inset_0_1px_0_0_color-mix(in_srgb,var(--color-foreground)_10%,transparent)]"><Bot size={20} aria-hidden="true" /></div>
      <h1 className="font-heading text-[26px] leading-tight tracking-[-0.02em] text-foreground font-semibold">{greeting.headline}</h1>
      <p className="mt-2.5 max-w-[440px] text-[13px] leading-relaxed text-muted-foreground">Start a chat with any model. Group chats in a workspace and connect a folder or git repo whenever you like—never required.</p>
      <div className="mt-[18px] flex items-center gap-2.5">
        <Button type="button" size="lg" onClick={onStartChat}><MessageSquarePlus size={16} aria-hidden="true" /> Start a chat</Button>
        <Button type="button" size="lg" variant="outline" onClick={onNewWorkspace}><Plus size={16} aria-hidden="true" /> New workspace</Button>
      </div>
      <small className="mt-[14px] text-muted-foreground/65 text-[11px]">Your code stays on this Mac.</small>
    </div>
  </div>;
}
function CommandPalette({ workspaces, onChoose }: { workspaces: Workspace[]; onChoose: (id:string)=>void }) { return <><InputGroup className="border-b border-border rounded-none border-x-0 border-t-0 shadow-none"><InputGroupInput autoFocus placeholder="Search workspaces and actions…" /><InputGroupAddon><Search size={17} aria-hidden="true" /></InputGroupAddon></InputGroup><div className="p-[9px]"><label className="block p-[5px_9px_7px] text-muted-foreground/65 text-[10px] font-semibold tracking-[0.09em]">WORKSPACES</label>{workspaces.map(w => <Button type="button" key={w.id} variant="ghost" className="w-full h-[44px] rounded-md justify-start px-2.5" onClick={() => onChoose(w.id)}><StatusDot status={w.status}/><span className="flex flex-col gap-[3px] flex-1 text-left"><b className="text-[12.5px] font-medium">{w.title}</b><small className="text-[10.5px] text-muted-foreground">{w.city} · {w.branch}</small></span><Kbd className="font-mono text-muted-foreground/65 border border-border rounded px-1 py-[1px] text-[10px]">↵</Kbd></Button>)}</div><div className="h-[32px] border-t border-border flex items-center gap-[14px] px-[13px] text-muted-foreground/65 text-[10.5px]"><span>↑↓ navigate</span><span>esc close</span></div></>; }
