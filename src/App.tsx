import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Activity, Archive, ArrowUp, Bot, CircleDot, Clock3, Command, FileCode2, FileDiff, FileText, FolderGit2, GitBranch, GitCommitHorizontal, GitPullRequest, Inbox, LayoutGrid, LoaderCircle, MessageSquareText, Monitor, PanelLeft, Play, Plus, Search, Settings2, Square, TerminalSquare, X } from "lucide-react";
import { bridgeApi } from "./api";
import type { AgentEvent, BridgeState, Health, Project, Session, SessionForestSnapshot, SessionStatus, Workspace } from "./types";
import { AgentConversation } from "./components/AgentConversation";
import { TerminalPane } from "./components/TerminalPane";
import { formatElapsed, tierRuntimeLabel } from "./utils";
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

export function App() {
  const [state, setState] = useState<BridgeState>(emptyState);
  const [agentEvents, setAgentEvents] = useState<AgentEvent[]>([]);
  const [health, setHealth] = useState<Health>();
  const [selectedId, setSelectedId] = useState<string>();
  const [selectedSessionId, setSelectedSessionId] = useState<string>();
  const [activeTab, setActiveTab] = useState<"agent" | "changes" | "events" | "terminal">("agent");
  const [modal, setModal] = useState<"workspace" | "project" | "palette" | null>(null);
  const [title, setTitle] = useState("");
  const [path, setPath] = useState("");
  const [composer, setComposer] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [clock, setClock] = useState(Date.now());
  const [home] = useState(false);
  const [forest, setForest] = useState<SessionForestSnapshot>();
  const autoStartRef = useRef<string>();

  const reload = useCallback(async () => {
    const next = await bridgeApi.state(); setState(next);
    setSelectedId(current => current && next.workspaces.some(w => w.id === current) ? current : next.workspaces[0]?.id);
  }, []);

  useEffect(() => {
    void Promise.all([reload(), bridgeApi.health().then(setHealth)]);
    let offState: (() => void) | undefined;
    let offAgent: (() => void) | undefined;
    void bridgeApi.onStateChanged(reload).then(fn => offState = fn);
    void bridgeApi.onAgentEvent(event => setAgentEvents(current => current.some(item => item.id === event.id) ? current : [...current, event])).then(fn => offAgent = fn);
    return () => { offState?.(); offAgent?.(); };
  }, [reload]);
  useEffect(() => { const timer = window.setInterval(() => setClock(Date.now()), 30_000); return () => window.clearInterval(timer); }, []);
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.metaKey && e.key.toLowerCase() === "k") { e.preventDefault(); setModal("palette"); }
      if (e.metaKey && e.key.toLowerCase() === "n") { e.preventDefault(); setModal("workspace"); }
      if (e.metaKey && /^[1-9]$/.test(e.key)) { const ws = state.workspaces[Number(e.key) - 1]; if (ws) setSelectedId(ws.id); }
      if (e.key === "Escape") setModal(null);
    }; window.addEventListener("keydown", key); return () => window.removeEventListener("keydown", key);
  }, [state.workspaces]);
  useEffect(() => {
    if (!selectedId || !("__TAURI_INTERNALS__" in window)) return;
    const refresh = () => { void bridgeApi.refreshWorkspace(selectedId).then(setState).catch(() => undefined); };
    refresh(); const timer = window.setInterval(refresh, 5000); return () => window.clearInterval(timer);
  }, [selectedId]);

  useEffect(() => {
    document.documentElement.classList.add("dark");
  }, []);
  
  const selected = state.workspaces.find(w => w.id === selectedId);
  const selectedProject = state.projects.find(p => p.id === selected?.projectId);
  const sessions = state.sessions.filter(s => s.workspaceId === selectedId && s.harness !== "shell");
  const session = sessions.find(s => s.id === selectedSessionId) ?? sessions.find(s => liveStatuses.includes(s.status)) ?? sessions[0];
  const sessionConnected = !!session && !session.endedAt && liveStatuses.includes(session.status);
  const sessionEvents = agentEvents.filter(event => event.sessionId === session?.id);
  const orderedSessions = useMemo(() => orderSessionTree(sessions), [sessions]);
  const grouped = useMemo(() => state.projects.map(project => ({ project, workspaces: state.workspaces.filter(w => w.projectId === project.id) })), [state]);
  const orchestratorReady = !!health?.adapters.find(adapter => adapter.id === "codex" && adapter.available);

  useEffect(() => {
    if (!session?.id) { setForest(undefined); return; }
    let active = true;
    const refresh = () => void bridgeApi.sessionForest(session.id).then(value => { if (active) setForest(value); }).catch(() => undefined);
    refresh();
    const timer = window.setInterval(refresh, 3000);
    return () => { active = false; window.clearInterval(timer); };
  }, [session?.id]);

  useEffect(() => {
    if (home || !selectedId || !("__TAURI_INTERNALS__" in window) || busy || !orchestratorReady) return;
    if (autoStartRef.current === selectedId) return;
    autoStartRef.current = selectedId;
    setBusy(true);
    void bridgeApi.startSession(selectedId, "codex", null)
      .then(next => {
        setState(next);
        const started = [...next.sessions].reverse().find(item => item.workspaceId === selectedId && liveStatuses.includes(item.status));
        if (started) setSelectedSessionId(started.id);
      })
      .catch(e => {
        autoStartRef.current = undefined;
        setError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => setBusy(false));
  }, [home, selectedId, state.sessions, health, busy, orchestratorReady]);

  async function chooseFolder() {
    if (!("__TAURI_INTERNALS__" in window)) return setPath("/Users/you/Developer/new-project");
    const value = await open({ directory: true, multiple: false, title: "Add a Git repository" }); if (value) setPath(value);
  }
  const errorMessage = (value: unknown) => value instanceof Error ? value.message : String(value);
  async function addProject() { setBusy(true); setError(undefined); try { setState(await bridgeApi.addProject(path)); setModal(null); setPath(""); } catch (e) { setError(errorMessage(e)); } finally { setBusy(false); } }
  async function createWorkspace() {
    const projectId = selectedProject?.id ?? state.projects[0]?.id; if (!projectId || !title.trim()) return;
    const name = title.trim();
    setBusy(true); setError(undefined);
    try {
      const next = await bridgeApi.createWorkspace(projectId, name, "codex");
      const workspace = next.workspaces.at(-1);
      if (!workspace) { setState(next); setModal(null); setTitle(""); return; }
      autoStartRef.current = workspace.id;
      setSelectedId(workspace.id);
      setModal(null);
      setTitle("");
      const started = await bridgeApi.startSession(workspace.id, "codex", null);
      setState(started);
      const active = [...started.sessions].reverse().find(item => item.workspaceId === workspace.id && liveStatuses.includes(item.status));
      if (active) setSelectedSessionId(active.id);
    } catch (e) { setError(errorMessage(e)); autoStartRef.current = undefined; }
    finally { setBusy(false); }
  }
  async function toggleSession(target?: Session) {
    if (!selected) return; setBusy(true);
    try {
      setError(undefined);
      const connected = !!target && !target.endedAt && liveStatuses.includes(target.status);
      const next = connected
        ? await bridgeApi.stopSession(target.id)
        : await bridgeApi.startSession(selected.id, "codex", null);
      setState(next);
      autoStartRef.current = selected.id;
      if (!connected) {
        const started = [...next.sessions].reverse().find(item => item.workspaceId === selected.id && liveStatuses.includes(item.status));
        setSelectedSessionId(started?.id);
      }
    } catch (e) { setError(errorMessage(e)); }
    finally { setBusy(false); }
  }
  async function sendPrompt() { if (!session || !composer.trim()) return; const text = composer.trim(); setComposer(""); try { await bridgeApi.sendTurn(session.id, text); } catch (e) { setComposer(text); setError(errorMessage(e)); } }
  async function resolveApproval(eventId: number, decision: string) { if (!session) return; try { await bridgeApi.resolveApproval(session.id, eventId, decision); await reload(); } catch (e) { setError(errorMessage(e)); } }
  async function selectConversationLeaf(entryId: string) {
    if (!session || !window.confirm("Switch conversation history? This changes the active conversation branch only. Files and Git state will not be rewound.")) return;
    try { setForest(await bridgeApi.activateSessionEntry(session.id, entryId)); }
    catch (e) { setError(errorMessage(e)); }
  }
  async function compactConversation() {
    if (!session) return;
    try { await bridgeApi.compactSession(session.id); window.setTimeout(() => void bridgeApi.sessionForest(session.id).then(setForest), 250); }
    catch (e) { setError(errorMessage(e)); }
  }
  async function archiveSelected() {
    if (!selected || !window.confirm(`Archive ${selected.city}? The clean worktree will be removed; its branch is preserved.`)) return;
    setBusy(true); setError(undefined);
    try { const next = await bridgeApi.archiveWorkspace(selected.id); setState(next); setSelectedId(next.workspaces[0]?.id); }
    catch (e) { setError(errorMessage(e)); }
    finally { setBusy(false); }
  }

  return <div className="h-screen grid grid-rows-[44px_1fr] grid-cols-[1fr] sm:grid-cols-[238px_1fr] bg-[radial-gradient(circle_at_18%_0%,color-mix(in_srgb,var(--color-foreground)_3%,transparent),transparent_34%),var(--color-background)]">
    <header className="col-span-full flex items-center gap-2 border-b border-border bg-background/82 backdrop-blur-[22px] backdrop-saturate-125 select-none relative z-20" data-tauri-drag-region>
      <div className="w-[72px] h-full shrink-0" data-tauri-drag-region />
      <div className="w-auto flex items-center shrink-0"><span className="hidden">B</span><strong className="text-[13px] font-semibold text-foreground tracking-[-0.02em]">Bridge</strong></div>
      <Button type="button" variant="outline" size="sm" className="ml-auto w-[190px] h-[30px] justify-start text-muted-foreground border-transparent bg-transparent hover:bg-accent" onClick={() => setModal("palette")}>
        <Command size={13} aria-hidden="true" /><span>Jump to anything</span><Kbd className="font-mono text-muted-foreground/65 border border-border rounded px-1 py-[1px] text-[10px]">⌘ K</Kbd>
      </Button>
    </header>
    <aside className="hidden sm:flex row-start-2 bg-sidebar/72 backdrop-blur-[28px] backdrop-saturate-135 border-r border-border flex-col min-h-0">
      <div className="h-[38px] px-[9px] flex items-center gap-[9px] text-muted-foreground/65 text-[10.5px] font-semibold tracking-[0.1em]">
        <Button type="button" size="icon-sm" variant="ghost" className="text-muted-foreground" aria-label="Toggle sidebar"><PanelLeft size={15} aria-hidden="true" /></Button>
        <span className="flex-1 text-[9px] tracking-[0.12em]">WORKSPACES</span>
        <Button type="button" size="icon-sm" variant="ghost" className="text-muted-foreground" onClick={() => setModal("workspace")} title="New workspace" aria-label="New workspace"><Plus size={16} aria-hidden="true" /></Button>
      </div>
      <div className="flex-1 overflow-auto px-[7px] pb-2 scrollbar-thin scrollbar-thumb-foreground/10">
        {grouped.map(({ project, workspaces }) => <section className="mb-3" key={project.id}>
          <div className="h-[28px] flex items-center gap-2 px-[7px] text-muted-foreground text-xs"><div className="hidden">{project.name.slice(0,1).toUpperCase()}</div><strong className="flex-1 font-medium whitespace-nowrap overflow-hidden text-ellipsis">{project.name}</strong></div>
          {workspaces.map((workspace, index) => <button key={workspace.id} type="button" className={`w-full min-h-[42px] rounded-lg grid grid-cols-[1fr_auto] items-center gap-[7px] px-[9px] py-1.5 text-left my-[1px] transition-colors hover:bg-accent ${workspace.id === selectedId ? "bg-foreground/7" : ""}`} onClick={() => { setSelectedId(workspace.id); setSelectedSessionId(undefined); autoStartRef.current = undefined; }}>
            <span className="hidden">{index + 1}</span><span className="min-w-0 flex flex-col gap-[2px]"><b className="text-[12px] text-foreground font-medium whitespace-nowrap overflow-hidden text-ellipsis">{workspace.title}</b><small className="text-[9.5px] text-muted-foreground whitespace-nowrap overflow-hidden text-ellipsis">{workspace.city} · {workspace.branch}</small></span><span className="self-start mt-[3px] flex items-center gap-[5px]"><StatusDot status={workspace.status}/><Badge variant="outline" size="sm" className="hidden">{statusCopy[workspace.status]}</Badge></span>
          </button>)}
        </section>)}
        {!state.projects.length && <div className="py-9 px-5 text-center text-muted-foreground text-xs grid gap-2.5 justify-items-center"><FolderGit2 size={25} aria-hidden="true" /><p>Add a Git repository to begin.</p></div>}
      </div>
      <Button type="button" variant="outline" className="mx-2.5 mb-2.5 w-[calc(100%-20px)] border-transparent justify-start h-[34px] text-muted-foreground hover:bg-accent" onClick={() => setModal("project")}><Plus size={14} aria-hidden="true" /> Add repository</Button>
    </aside>
    <main className="row-start-2 min-w-0 min-h-0 grid grid-rows-[auto_auto_auto_1fr] bg-transparent">
      {selected ? <>
        <div className="min-h-[58px] px-[18px] py-[9px] flex items-center border-b border-border">
          <div>
            <div className="hidden"><StatusDot status={selected.status}/><Badge variant="outline" size="sm">{statusCopy[selected.status]}</Badge> · {selected.city.toUpperCase()}</div>
            <h1 className="m-0 mb-[3px] font-heading text-[14px] leading-tight text-foreground font-semibold tracking-[-0.015em]">{selected.title}</h1>
            <div className="flex items-center gap-1.5 text-muted-foreground font-mono text-[9.5px]"><GitBranch size={13} aria-hidden="true" />{selected.branch}<span>·</span>{selected.dirtyFiles ? <span className="text-warning">{selected.dirtyFiles} files changed</span> : <span>clean</span>}</div>
          </div>
          <div className="ml-auto flex gap-[7px]">
            <Button type="button" variant="outline" size="sm" className="hidden"><GitPullRequest size={14} aria-hidden="true" /> Review changes</Button>
            <Button type="button" variant="outline" size="sm" className="hidden" onClick={() => void archiveSelected()} title="Archive clean workspace"><Archive size={14} aria-hidden="true" /> Archive</Button>
            {session?.activeTurnId && <Button type="button" variant="outline" size="sm" className="hidden" onClick={() => void bridgeApi.interruptTurn(session.id)}><Square size={12} aria-hidden="true" /> Stop turn</Button>}
            {sessionConnected
              ? <Button type="button" variant="destructive-outline" size="sm" className="border-destructive/30" disabled={busy} onClick={() => void toggleSession(session)}>{busy ? <LoaderCircle className="animate-spin" size={14} aria-hidden="true" /> : <Square size={13} aria-hidden="true" />} End agent</Button>
              : <Button type="button" size="sm" disabled={busy || !orchestratorReady} loading={busy} onClick={() => void toggleSession(session)} title="Restart if auto-start failed"><Play size={14} aria-hidden="true" /> Restart</Button>}
          </div>
        </div>
        <div className="min-h-[40px] border-b-0 flex items-center px-[14px] py-[5px] gap-1.5 flex-wrap">
          {orderedSessions.map(s => <button type="button" className={`h-[28px] border border-transparent rounded-md flex items-center gap-2 px-[7px] text-left transition-colors ${s.id === session?.id ? "bg-transparent border-transparent" : "hidden"} ${(s.depth ?? 0) > 0 ? "opacity-90" : ""}`} key={s.id} style={(s.depth ?? 0) > 0 ? { marginLeft: (s.depth ?? 0) * 14 } : undefined} onClick={() => setSelectedSessionId(s.id)}><span className="hidden"><Bot size={14} aria-hidden="true" /></span><span className="flex flex-col gap-[3px]"><b className="text-[10.5px] text-muted-foreground font-medium">{s.label}</b><small className="hidden"><StatusDot status={s.status}/>{statusCopy[s.status]} · {s.restorationMode.replaceAll("_", " ").toUpperCase()} · {tierRuntimeLabel(s.requestedTier, s.model, s.effort)}</small></span></button>)}
          <div className="ml-auto flex items-center gap-[12px] font-mono text-[9px] text-muted-foreground">
            <span className="flex flex-row gap-1">ELAPSED <b className="text-muted-foreground font-medium">{formatElapsed(session?.startedAt, clock)}</b></span>
            <span className="flex flex-row gap-1">CONTEXT <b className="text-muted-foreground font-medium">{session?.contextPercent ?? "—"}{session?.contextPercent != null ? "%" : ""}</b></span>
            <span className="flex flex-row gap-1">USAGE <b className="text-muted-foreground font-medium">{session?.usagePercent ?? "—"}{session?.usagePercent != null ? "%" : ""}</b></span>
            <Badge variant="outline" size="sm" className="hidden">{session?.metricSource?.toUpperCase() ?? "UNAVAILABLE"}</Badge>
          </div>
        </div>
        <Tabs value={activeTab} onValueChange={v => setActiveTab(v as typeof activeTab)}>
          <div className="border-b-0 px-[14px]">
            <TabsList variant="underline" className="w-full justify-start gap-[2px] bg-transparent p-0">
              <TabsTab value="agent" className="h-[28px] px-2 text-[10.5px] text-muted-foreground rounded-none"><MessageSquareText size={14} aria-hidden="true" /> Agent</TabsTab>
              <TabsTab value="changes" className="h-[28px] px-2 text-[10.5px] text-muted-foreground rounded-none"><FileCode2 size={14} aria-hidden="true" /> Changes {selected.dirtyFiles > 0 && <Badge variant="secondary" size="sm">{selected.dirtyFiles}</Badge>}</TabsTab>
              <TabsTab value="events" className="h-[28px] px-2 text-[10.5px] text-muted-foreground rounded-none"><Activity size={14} aria-hidden="true" /> Events</TabsTab>
              <TabsTab value="terminal" className="h-[28px] px-2 text-[10.5px] text-muted-foreground rounded-none"><TerminalSquare size={14} aria-hidden="true" /> Terminal</TabsTab>
            </TabsList>
          </div>
        </Tabs>
        <section className="min-h-0 overflow-hidden flex border-t border-border">
          <div className="flex-1 min-w-0 flex flex-col relative">
            {activeTab === "agent" && <>
              <div className="flex-1 min-h-0 relative">
                <AgentConversation
                  session={session}
                  events={sessionEvents}
                  forestEntries={forest?.entries}
                  activeLeafId={forest?.head?.activeEntryId}
                  preview={!forest?.entries?.length && !sessionEvents.length}
                  onResolve={(eventId, decision) => void resolveApproval(eventId, decision)}
                />
              </div>
              <div className="flex-none pb-4 px-7">
                {(selected.dirtyFiles > 0 || session?.activeTurnId) && <div className="max-w-[760px] mx-auto flex justify-center -mb-2 relative z-10">
                  <div className="inline-flex items-center gap-2 h-[30px] px-[13px] border border-border rounded-full bg-card shadow-[0_6px_18px_rgba(0,0,0,0.16)] text-muted-foreground text-xs">
                    {session?.activeTurnId ? <LoaderCircle size={12} className="animate-spin text-muted-foreground/65" aria-hidden="true" /> : <FileDiff size={12} className="text-muted-foreground/65" aria-hidden="true" />}
                    <span>{selected.dirtyFiles ? `${selected.dirtyFiles} file${selected.dirtyFiles === 1 ? "" : "s"}` : "working"}</span>
                    {selected.dirtyFiles > 0 && <em className="not-italic font-mono text-[11.5px]"><b className="text-success font-medium">+{selected.additions}</b> <b className="text-destructive font-medium">−{selected.deletions}</b></em>}
                  </div>
                </div>}
                <div className="max-w-[760px] mx-auto border border-border bg-card rounded-[13px] shadow-[0_14px_38px_rgba(0,0,0,0.16)] transition-colors focus-within:border-foreground/16">
                  <Textarea className="w-full [&>textarea]:min-h-[44px] [&>textarea]:max-h-[180px] [&>textarea]:resize-none [&>textarea]:bg-transparent [&>textarea]:text-foreground [&>textarea]:px-4 [&>textarea]:pt-[13px] [&>textarea]:pb-1 [&>textarea]:text-[13.5px] [&>textarea]:leading-relaxed" unstyled value={composer} onChange={e => setComposer(e.target.value)} onKeyDown={e => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); void sendPrompt(); } }} placeholder={sessionConnected ? "Ask for follow-up changes…" : busy ? "Starting orchestrator…" : "Orchestrator starting…"} disabled={!sessionConnected} />
                  <div className="h-10 flex items-center gap-2 px-2.5">
                    <Button type="button" size="icon-sm" variant="ghost" className="text-muted-foreground" title="Attach context" aria-label="Attach context"><Plus size={15} aria-hidden="true" /></Button>
                    <span className="inline-flex items-center gap-1.5 h-[26px] px-1 text-muted-foreground text-[11.5px] rounded-full">{`${session?.label ?? "Orchestrator"} · ${tierRuntimeLabel(session?.requestedTier, session?.model)}`}</span>
                    <span className="ml-auto text-muted-foreground/65 text-[10.5px]">↵ send · ⇧↵ newline</span>
                    {session?.activeTurnId
                      ? <Button type="button" size="icon-sm" className="rounded-full" onClick={() => void bridgeApi.interruptTurn(session.id)} title="Stop turn" aria-label="Stop turn"><Square size={11} fill="currentColor" aria-hidden="true" /></Button>
                      : <Button type="button" size="icon-sm" className="rounded-full" onClick={() => void sendPrompt()} disabled={!composer.trim() || !sessionConnected} aria-label="Send message"><ArrowUp size={15} aria-hidden="true" /></Button>}
                  </div>
                </div>
              </div>
            </>}
            {activeTab === "changes" && <ChangesPanel workspace={selected}/>}
            {activeTab === "events" && <EventPanel state={state} workspace={selected}/>}
            {activeTab === "terminal" && <div className="absolute inset-0"><TerminalPane workspaceId={selected.id}/></div>}
          </div>
          {activeTab === "agent" && <EnvPanel
            workspace={selected}
            project={selectedProject}
            session={session}
            sessions={state.sessions}
            forest={forest}
            onChanges={() => setActiveTab("changes")}
            onSelectLeaf={entryId => void selectConversationLeaf(entryId)}
            onCompact={() => void compactConversation()}
          />}
        </section>
      </> : <Welcome onAdd={() => setModal("project")}/>}
    </main>
    {error && (
      <Alert variant="error" className="fixed right-[18px] bottom-[18px] z-40 max-w-[520px] shadow-[0_16px_50px_color-mix(in_srgb,var(--color-background)_50%,transparent)]">
        <AlertDescription>{error}</AlertDescription>
        <AlertAction>
          <Button type="button" size="icon-sm" variant="ghost" aria-label="Dismiss error" onClick={() => setError(undefined)}><X size={14} aria-hidden="true" /></Button>
        </AlertAction>
      </Alert>
    )}

    <Dialog open={modal === "project"} onOpenChange={open => !open && setModal(null)}>
      <DialogPopup>
        <DialogHeader>
          <DialogTitle>Add a repository</DialogTitle>
          <DialogDescription>Bridge works locally and never uploads your code.</DialogDescription>
        </DialogHeader>
        <DialogPanel>
          <Label className="block text-muted-foreground/65 text-[10.5px] font-semibold tracking-[0.09em] m-[0_0_7px]">REPOSITORY PATH</Label>
          <InputGroup>
            <InputGroupInput autoFocus value={path} onChange={e => setPath(e.target.value)} placeholder="/Users/you/Developer/project" className="font-mono text-xs" />
            <InputGroupAddon align="inline-end">
              <Button type="button" variant="outline" size="sm" onClick={() => void chooseFolder()}>Choose…</Button>
            </InputGroupAddon>
          </InputGroup>
        </DialogPanel>
        <DialogFooter>
          <DialogClose render={<Button type="button" variant="ghost" />}>Cancel</DialogClose>
          <Button type="button" loading={busy} disabled={!path} onClick={() => void addProject()}>{busy ? "Adding…" : "Add repository"}</Button>
        </DialogFooter>
      </DialogPopup>
    </Dialog>

    <Dialog open={modal === "workspace"} onOpenChange={open => !open && setModal(null)}>
      <DialogPopup>
        <DialogHeader>
          <DialogTitle>New workspace</DialogTitle>
          <DialogDescription>Name the lane. Bridge starts the orchestrator automatically—no harness or model to pick.</DialogDescription>
        </DialogHeader>
        <DialogPanel>
          <Label className="block text-muted-foreground/65 text-[10.5px] font-semibold tracking-[0.09em] m-[0_0_7px]">WORKSPACE NAME</Label>
          <Textarea autoFocus value={title} onChange={e => setTitle(e.target.value)} placeholder="e.g. Keyboard navigation" />
          <p className="mt-[13px] text-muted-foreground text-xs leading-relaxed">Starts a fast-tier Orchestrator. The adapter chooses the runtime model.</p>
        </DialogPanel>
        <DialogFooter>
          <DialogClose render={<Button type="button" variant="ghost" />}>Cancel</DialogClose>
          <Button type="button" loading={busy} disabled={!title.trim() || !state.projects.length || !orchestratorReady} onClick={() => void createWorkspace()}>{busy ? "Starting…" : "Create workspace"}</Button>
        </DialogFooter>
      </DialogPopup>
    </Dialog>

    <Dialog open={modal === "palette"} onOpenChange={open => !open && setModal(null)}>
      <DialogPopup className="max-w-[560px] p-0 overflow-hidden" showCloseButton={false} bottomStickOnMobile={false}>
        <CommandPalette workspaces={state.workspaces} onChoose={id => { setSelectedId(id); setModal(null); autoStartRef.current = undefined; }} />
      </DialogPopup>
    </Dialog>
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
function Welcome({ onAdd }: { onAdd: () => void }) { return <div className="row-[1/-1] flex flex-col items-center justify-center text-center text-muted-foreground"><div className="w-10 h-10 grid place-items-center mb-5 text-foreground bg-accent font-mono font-semibold text-[18px] rounded-[11px]">B</div><div className="flex items-center gap-1.5 mb-[5px] text-muted-foreground text-[10px] tracking-[0.08em] justify-center">LOCAL AGENT CONTROL ROOM</div><h1 className="font-heading text-[30px] leading-tight tracking-[-0.03em] text-foreground my-1.5 font-semibold">One orchestrator.<br/>Focused workers.</h1><p className="text-[13px] leading-relaxed text-muted-foreground">Open a workspace and Bridge starts a fast-tier orchestrator.<br/>Describe what to build—Bridge resolves the runtime.</p><Button type="button" size="lg" className="mt-[18px]" onClick={onAdd}><FolderGit2 size={16} aria-hidden="true" /> Add your first repository</Button><small className="mt-[14px] text-muted-foreground/65 text-[11px]">Your code stays on this Mac.</small></div>; }
function CommandPalette({ workspaces, onChoose }: { workspaces: Workspace[]; onChoose: (id:string)=>void }) { return <><InputGroup className="border-b border-border rounded-none border-x-0 border-t-0 shadow-none"><InputGroupInput autoFocus placeholder="Search workspaces and actions…" /><InputGroupAddon><Search size={17} aria-hidden="true" /></InputGroupAddon></InputGroup><div className="p-[9px]"><label className="block p-[5px_9px_7px] text-muted-foreground/65 text-[10px] font-semibold tracking-[0.09em]">WORKSPACES</label>{workspaces.map(w => <Button type="button" key={w.id} variant="ghost" className="w-full h-[44px] rounded-md justify-start px-2.5" onClick={() => onChoose(w.id)}><StatusDot status={w.status}/><span className="flex flex-col gap-[3px] flex-1 text-left"><b className="text-[12.5px] font-medium">{w.title}</b><small className="text-[10.5px] text-muted-foreground">{w.city} · {w.branch}</small></span><Kbd className="font-mono text-muted-foreground/65 border border-border rounded px-1 py-[1px] text-[10px]">↵</Kbd></Button>)}</div><div className="h-[32px] border-t border-border flex items-center gap-[14px] px-[13px] text-muted-foreground/65 text-[10.5px]"><span>↑↓ navigate</span><span>esc close</span></div></>; }
