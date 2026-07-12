import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Activity, Archive, ArrowUp, Bot, CircleDot, Clock3, Command, FileCode2, FileDiff, FileText, FolderGit2, GitBranch, GitCommitHorizontal, GitPullRequest, Inbox, LayoutGrid, LoaderCircle, MessageSquareText, Monitor, PanelLeft, Play, Plus, Search, Settings2, Square, TerminalSquare, X } from "lucide-react";
import { bridgeApi } from "./api";
import type { BridgeState, Health, Project, Session, SessionForestSnapshot, SessionStatus, Workspace } from "./types";
import { MOCK_CONVERSATION } from "./mockConversation";
import { AgentConversation } from "./components/AgentConversation";
import { TerminalPane } from "./components/TerminalPane";
import { WelcomeScreen } from "./components/WelcomeScreen";
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

const emptyState: BridgeState = { projects: [], workspaces: [], sessions: [], events: [], agentEvents: [] };
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

function StatusDot({ status }: { status: SessionStatus }) { return <span className={`status-dot ${status}`} />; }

export function App() {
  const [state, setState] = useState<BridgeState>(emptyState);
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
  const [home, setHome] = useState(true);
  const [forest, setForest] = useState<SessionForestSnapshot>();
  const autoStartRef = useRef<string>();

  const reload = useCallback(async () => {
    const next = await bridgeApi.state(); setState(next);
    setSelectedId(current => current && next.workspaces.some(w => w.id === current) ? current : next.workspaces[0]?.id);
  }, []);

  useEffect(() => {
    void Promise.all([reload(), bridgeApi.health().then(setHealth)]);
    let off: (() => void) | undefined;
    void bridgeApi.onStateChanged(reload).then(fn => off = fn);
    return () => off?.();
  }, [reload]);
  useEffect(() => { let off: (() => void) | undefined; void bridgeApi.onAgentEvent(event => setState(current => current.agentEvents.some(item => item.id === event.id) ? current : { ...current, agentEvents: [...current.agentEvents, event] })).then(fn => off = fn); return () => off?.(); }, []);
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

  const selected = state.workspaces.find(w => w.id === selectedId);
  const selectedProject = state.projects.find(p => p.id === selected?.projectId);
  const sessions = state.sessions.filter(s => s.workspaceId === selectedId && s.harness !== "shell");
  const session = sessions.find(s => s.id === selectedSessionId) ?? sessions.find(s => liveStatuses.includes(s.status)) ?? sessions[0];
  const sessionConnected = !!session && !session.endedAt && liveStatuses.includes(session.status);
  const sessionEvents = state.agentEvents.filter(event => event.sessionId === session?.id);
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
  async function resolveApproval(eventId: number, decision: string) { try { await bridgeApi.resolveApproval(eventId, decision); await reload(); } catch (e) { setError(errorMessage(e)); } }
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

  return <div className="app-shell">
    <header className="top-rail" data-tauri-drag-region>
      <div className="traffic-space" data-tauri-drag-region />
      <div className="wordmark"><span className="mark">B</span><strong>Bridge</strong><em>LOCAL</em></div>
      <Button type="button" variant="outline" size="sm" className="command-trigger" onClick={() => setModal("palette")}>
        <Command size={13} aria-hidden="true" /><span>Jump to anything</span><Kbd>⌘ K</Kbd>
      </Button>
      <Badge variant={health?.ok ? "success" : "outline"} className={`daemon-pill ${health?.ok ? "online" : ""}`}>
        <span />{health?.ok ? "DAEMON ONLINE" : "CONNECTING"}
      </Badge>
    </header>
    <aside className="sidebar">
      <div className="sidebar-heading">
        <Button type="button" size="icon-sm" variant="ghost" className="icon-button" aria-label="Toggle sidebar"><PanelLeft size={15} aria-hidden="true" /></Button>
        <span>WORKSPACES</span>
        <Button type="button" size="icon-sm" variant="ghost" className="icon-button" onClick={() => setModal("workspace")} title="New workspace" aria-label="New workspace"><Plus size={16} aria-hidden="true" /></Button>
      </div>
      <div className="workspace-scroll">
        {grouped.map(({ project, workspaces }) => <section className="project-group" key={project.id}>
          <div className="project-title"><div className="repo-icon">{project.name.slice(0,1).toUpperCase()}</div><strong>{project.name}</strong></div>
          {workspaces.map((workspace, index) => <button key={workspace.id} type="button" className={`workspace-row ${workspace.id === selectedId ? "selected" : ""}`} onClick={() => { setSelectedId(workspace.id); setSelectedSessionId(undefined); autoStartRef.current = undefined; }}>
            <span className="workspace-index">{index + 1}</span><span className="workspace-main"><b>{workspace.title}</b><small>{workspace.city} · {workspace.branch}</small></span><span className="workspace-status"><StatusDot status={workspace.status}/><Badge variant="outline" size="sm">{statusCopy[workspace.status]}</Badge></span>
          </button>)}
        </section>)}
        {!state.projects.length && <div className="empty-sidebar"><FolderGit2 size={25} aria-hidden="true" /><p>Add a Git repository to begin.</p></div>}
      </div>
      <Button type="button" variant="outline" className="add-repository" onClick={() => setModal("project")}><Plus size={14} aria-hidden="true" /> Add repository</Button>
      <nav className="sidebar-nav">
        <button type="button"><Inbox size={15} aria-hidden="true" /> Inbox <Badge variant="secondary" size="sm">{state.events.filter(e => e.kind.includes("waiting")).length}</Badge></button>
        <button type="button"><LayoutGrid size={15} aria-hidden="true" /> All sessions</button>
        <button type="button"><Clock3 size={15} aria-hidden="true" /> Night queue</button>
        <button type="button"><Settings2 size={15} aria-hidden="true" /> Settings</button>
      </nav>
    </aside>
    <main className="workspace-view">
      {selected ? <>
        <div className="workspace-header">
          <div>
            <div className="eyebrow"><StatusDot status={selected.status}/><Badge variant="outline" size="sm">{statusCopy[selected.status]}</Badge> · {selected.city.toUpperCase()}</div>
            <h1>{selected.title}</h1>
            <div className="branch-line"><GitBranch size={13} aria-hidden="true" />{selected.branch}<span>·</span>{selected.dirtyFiles ? <Badge variant="warning" size="sm">{selected.dirtyFiles} files changed</Badge> : <span>clean</span>}</div>
          </div>
          <div className="workspace-actions">
            <Button type="button" variant="outline" size="sm" className="secondary"><GitPullRequest size={14} aria-hidden="true" /> Review changes</Button>
            <Button type="button" variant="outline" size="sm" className="secondary archive" onClick={() => void archiveSelected()} title="Archive clean workspace"><Archive size={14} aria-hidden="true" /> Archive</Button>
            {session?.activeTurnId && <Button type="button" variant="outline" size="sm" className="secondary" onClick={() => void bridgeApi.interruptTurn(session.id)}><Square size={12} aria-hidden="true" /> Stop turn</Button>}
            {sessionConnected
              ? <Button type="button" variant="destructive-outline" size="sm" className="run-button stop" disabled={busy} onClick={() => void toggleSession(session)}>{busy ? <LoaderCircle className="spin" size={14} aria-hidden="true" /> : <Square size={13} aria-hidden="true" />} End agent</Button>
              : <Button type="button" size="sm" className="run-button" disabled={busy || !orchestratorReady} loading={busy} onClick={() => void toggleSession(session)} title="Restart if auto-start failed"><Play size={14} aria-hidden="true" /> Restart</Button>}
          </div>
        </div>
        <div className="session-strip">
          {orderedSessions.map(s => <button type="button" className={`session-chip ${s.id === session?.id ? "active" : ""} ${(s.depth ?? 0) > 0 ? "worker" : ""}`} key={s.id} style={(s.depth ?? 0) > 0 ? { marginLeft: (s.depth ?? 0) * 14 } : undefined} onClick={() => setSelectedSessionId(s.id)}><span className={`harness-icon ${s.harness}`}><Bot size={14} aria-hidden="true" /></span><span><b>{s.label}</b><small><StatusDot status={s.status}/>{statusCopy[s.status]} · {s.restorationMode.replaceAll("_", " ").toUpperCase()} · {tierRuntimeLabel(s.requestedTier, s.model, s.effort)}</small></span></button>)}
          <div className="session-metrics"><span>ELAPSED <b>{formatElapsed(session?.startedAt, clock)}</b></span><span>CONTEXT <b>{session?.contextPercent ?? "—"}{session?.contextPercent != null ? "%" : ""}</b></span><span>USAGE <b>{session?.usagePercent ?? "—"}{session?.usagePercent != null ? "%" : ""}</b></span><Badge variant="outline" size="sm">{session?.metricSource?.toUpperCase() ?? "UNAVAILABLE"}</Badge></div>
        </div>
        <Tabs value={activeTab} onValueChange={v => setActiveTab(v as typeof activeTab)}>
          <div className="content-tabs-wrap">
            <TabsList variant="underline">
              <TabsTab value="agent"><MessageSquareText size={14} aria-hidden="true" /> Agent</TabsTab>
              <TabsTab value="changes"><FileCode2 size={14} aria-hidden="true" /> Changes {selected.dirtyFiles > 0 && <Badge variant="secondary" size="sm">{selected.dirtyFiles}</Badge>}</TabsTab>
              <TabsTab value="events"><Activity size={14} aria-hidden="true" /> Events</TabsTab>
              <TabsTab value="terminal"><TerminalSquare size={14} aria-hidden="true" /> Terminal</TabsTab>
            </TabsList>
          </div>
        </Tabs>
        <section className="content-body">
          <div className="content-main">
            {activeTab === "agent" && <>
              <div className="convo-host">
                <AgentConversation
                  session={session}
                  events={sessionEvents.length ? sessionEvents : MOCK_CONVERSATION}
                  forestEntries={forest?.entries}
                  activeLeafId={forest?.head?.activeEntryId}
                  preview={!sessionEvents.length && !forest?.entries.length}
                  onResolve={(eventId, decision) => void resolveApproval(eventId, decision)}
                />
              </div>
              <div className="composer">
                {(selected.dirtyFiles > 0 || session?.activeTurnId) && <div className="turn-chip-row"><div className="turn-chip">{session?.activeTurnId ? <LoaderCircle size={12} className="spin" aria-hidden="true" /> : <FileDiff size={12} aria-hidden="true" />}<span>{selected.dirtyFiles ? `${selected.dirtyFiles} file${selected.dirtyFiles === 1 ? "" : "s"}` : "working"}</span>{selected.dirtyFiles > 0 && <em><b className="add">+{selected.additions}</b> <b className="del">−{selected.deletions}</b></em>}</div></div>}
                <div className="composer-inner">
                  <Textarea className="composer-textarea" unstyled value={composer} onChange={e => setComposer(e.target.value)} onKeyDown={e => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); void sendPrompt(); } }} placeholder={sessionConnected ? "Ask for follow-up changes…" : busy ? "Starting orchestrator…" : "Orchestrator starting…"} disabled={!sessionConnected} />
                  <div className="composer-tools">
                    <Button type="button" size="icon-sm" variant="ghost" className="composer-plus" title="Attach context" aria-label="Attach context"><Plus size={15} aria-hidden="true" /></Button>
                    <span className="orchestrator-pill">{`${session?.label ?? "Orchestrator"} · ${tierRuntimeLabel(session?.requestedTier, session?.model)}`}</span>
                    <span className="hint">↵ send · ⇧↵ newline</span>
                    {session?.activeTurnId
                      ? <Button type="button" size="icon-sm" className="send-button" onClick={() => void bridgeApi.interruptTurn(session.id)} title="Stop turn" aria-label="Stop turn"><Square size={11} fill="currentColor" aria-hidden="true" /></Button>
                      : <Button type="button" size="icon-sm" className="send-button" onClick={() => void sendPrompt()} disabled={!composer.trim() || !sessionConnected} aria-label="Send message"><ArrowUp size={15} aria-hidden="true" /></Button>}
                  </div>
                </div>
              </div>
            </>}
            {activeTab === "changes" && <ChangesPanel workspace={selected}/>}
            {activeTab === "events" && <EventPanel state={state} workspace={selected}/>}
            {activeTab === "terminal" && <div className="terminal-layer"><TerminalPane workspaceId={selected.id}/></div>}
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
    {home && <WelcomeScreen onDismiss={() => setHome(false)}/>}
    {error && (
      <Alert variant="error" className="error-toast">
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
          <Label className="field-label">REPOSITORY PATH</Label>
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
          <Label className="field-label">WORKSPACE NAME</Label>
          <Textarea autoFocus value={title} onChange={e => setTitle(e.target.value)} placeholder="e.g. Keyboard navigation" />
          <p className="modal-hint">Starts a fast-tier Orchestrator. The adapter chooses the runtime model.</p>
        </DialogPanel>
        <DialogFooter>
          <DialogClose render={<Button type="button" variant="ghost" />}>Cancel</DialogClose>
          <Button type="button" loading={busy} disabled={!title.trim() || !state.projects.length || !orchestratorReady} onClick={() => void createWorkspace()}>{busy ? "Starting…" : "Create workspace"}</Button>
        </DialogFooter>
      </DialogPopup>
    </Dialog>

    <Dialog open={modal === "palette"} onOpenChange={open => !open && setModal(null)}>
      <DialogPopup className="palette-dialog" showCloseButton={false} bottomStickOnMobile={false}>
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
  return <aside className="env-panel">
    <div className="env-label">Environment</div>
    <button type="button" className="env-row" onClick={onChanges}><FileDiff size={14} aria-hidden="true" /><span>Changes</span>{workspace.dirtyFiles ? <small className="dstat"><b className="add">+{workspace.additions}</b><b className="del">−{workspace.deletions}</b></small> : <small>clean</small>}</button>
    <button type="button" className="env-row"><Monitor size={14} aria-hidden="true" /><span>Local</span><small>{workspace.city}</small></button>
    <button type="button" className="env-row"><GitBranch size={14} aria-hidden="true" /><span>{workspace.branch}</span></button>
    <button type="button" className="env-row"><GitCommitHorizontal size={14} aria-hidden="true" /><span>Commit or push</span></button>
    <button type="button" className="env-row"><GitPullRequest size={14} aria-hidden="true" /><span>Create pull request</span></button>
    <div className="env-label">Session forest</div>
    <div className={`restoration-badge ${session?.restorationMode ?? "fresh"}`}><span>{restoration.label}</span><small>{restoration.detail}</small></div>
    <div className="budget-card"><span>Turn budget</span><b>{budget.units}/{forest?.policyLimits.maxCapabilityUnitsPerTurn ?? 24} units</b><small>{budget.workers}/{forest?.policyLimits.maxWorkersPerTurn ?? 3} workers · {budget.strongWorkers}/{forest?.policyLimits.maxStrongWorkersPerTurn ?? 1} strong</small></div>
    <button type="button" className="env-row" onClick={onCompact}><FileText size={14} aria-hidden="true" /><span>Compact context</span><small>{session?.contextPercent == null ? "manual" : `${session.contextPercent}%`}</small></button>
    <div className="rewind-warning">Conversation rewind never rewinds files or Git state.</div>
    {(forest?.leaves ?? []).map(leaf => <button type="button" className={`env-row forest-leaf ${leaf.id === forest?.head?.activeEntryId ? "active" : ""}`} key={leaf.id} onClick={() => onSelectLeaf(leaf.id)}><GitBranch size={14} aria-hidden="true" /><span>{leaf.payload.summary ? String(leaf.payload.summary) : `${leaf.kind} · ${leaf.sequence}`}</span><small>{leaf.id === forest?.head?.activeEntryId ? "active" : "switch"}</small></button>)}
    <div className="env-label">Subagents</div>
    {workers.length
      ? workers.map(worker => {
          const lease = forest?.workerLeases.find(item => item.sessionId === worker.id);
          const runtime = forest?.workerRuntimes.find(item => item.sessionId === worker.id);
          return <details className="worker-drilldown" key={worker.id}><summary><StatusDot status={worker.status}/><span>{worker.label}</span><small>{runtime?.lifecycleState ?? worker.status}</small></summary><div><p><b>{lease?.role ?? "worker"}</b> · {lease?.writeMode ?? "—"} · {lease?.leaseStatus ?? "—"}</p><p>{(lease?.ownedPaths ?? []).join(", ") || "No owned paths"}</p><p>{tierRuntimeLabel(worker.requestedTier, worker.model, worker.effort)}</p>{runtime?.lastResult && <pre>{JSON.stringify(runtime.lastResult, null, 2)}</pre>}</div></details>;
        })
      : <div className="env-empty">{doneWorkers ? `${doneWorkers} done` : "None spawned yet"}</div>}
    {(forest?.workerQueue ?? []).map(item => <details className="queue-explanation" key={item.id}><summary><Clock3 size={13} aria-hidden="true" /><span>{item.queueStatus}: {String(item.request.objective ?? "worker request")}</span></summary><p>{queueExplanation(item, forest?.workerLeases ?? [])}</p><code>{Array.isArray(item.request.ownedPaths) ? item.request.ownedPaths.join(", ") : ""}</code><small>runtime {item.actualModel}</small></details>)}
    {!!forest?.reasons.length && <details className="reason-log"><summary><CircleDot size={13} aria-hidden="true" /> Inspect lifecycle reasons</summary>{forest.reasons.map(reason => <div key={reason.id}><b>{reason.kind}</b><p>{reason.body}</p></div>)}</details>}
    <div className="env-label">Sources</div>
    <button type="button" className="env-row"><FolderGit2 size={14} aria-hidden="true" /><span>{project?.path ?? workspace.path}</span></button>
  </aside>;
}

function ChangesPanel({ workspace }: { workspace: Workspace }) { return <div className="panel-view"><div className="panel-kicker">CHANGE STORY</div><h2>{workspace.dirtyFiles ? `${workspace.dirtyFiles} files changed` : "Workspace is clean"}</h2><p>Behavior-grouped review will live here. High-risk authentication, migrations, test weakening, and evaluation thresholds are always expanded.</p><div className="diff-stat"><b className="add">+{workspace.additions}</b><b className="del">−{workspace.deletions}</b><span/><small>{workspace.branch}</small></div><div className="placeholder-lines">{[78,92,64,85,51,70].map((n,i)=><i key={i} style={{width:`${n}%`}}/>)}</div></div>; }
function EventPanel({ state, workspace }: { state: BridgeState; workspace: Workspace }) { const events = state.events.filter(e => e.entityId === workspace.id || state.sessions.some(s => s.workspaceId === workspace.id && s.id === e.entityId)); return <div className="event-list">{events.length ? events.map(e => <article key={e.id}><CircleDot size={14} aria-hidden="true" /><div><b>{e.kind.replaceAll(".", " ")}</b><p>{e.body}</p><small>{new Date(e.createdAt).toLocaleTimeString([], {hour:"2-digit",minute:"2-digit"})}</small></div></article>) : <div className="empty-panel">No events for this workspace yet.</div>}</div>; }
function Welcome({ onAdd }: { onAdd: () => void }) { return <div className="welcome"><div className="welcome-mark">B</div><div className="eyebrow">LOCAL AGENT CONTROL ROOM</div><h1>One orchestrator.<br/>Focused workers.</h1><p>Open a workspace and Bridge starts a fast-tier orchestrator.<br/>Describe what to build—Bridge resolves the runtime.</p><Button type="button" size="lg" className="welcome-cta" onClick={onAdd}><FolderGit2 size={16} aria-hidden="true" /> Add your first repository</Button><small>Your code stays on this Mac.</small></div>; }
function CommandPalette({ workspaces, onChoose }: { workspaces: Workspace[]; onChoose: (id:string)=>void }) { return <><InputGroup className="palette-input"><InputGroupInput autoFocus placeholder="Search workspaces and actions…" /><InputGroupAddon><Search size={17} aria-hidden="true" /></InputGroupAddon></InputGroup><div className="palette-section"><label>WORKSPACES</label>{workspaces.map(w => <Button type="button" key={w.id} variant="ghost" className="palette-row" onClick={() => onChoose(w.id)}><StatusDot status={w.status}/><span><b>{w.title}</b><small>{w.city} · {w.branch}</small></span><Kbd>↵</Kbd></Button>)}</div><div className="palette-footer"><span>↑↓ navigate</span><span>esc close</span></div></>; }
