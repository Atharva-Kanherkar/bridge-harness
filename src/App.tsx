import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Activity, Archive, Bot, Box, CircleDot, Clock3, Command, FileCode2, FolderGit2, GitBranch, GitPullRequest, Inbox, LayoutGrid, LoaderCircle, MessageSquareText, PanelLeft, Play, Plus, Search, Send, Settings2, Square, TerminalSquare, X } from "lucide-react";
import { bridgeApi } from "./api";
import type { BridgeState, Health, Session, SessionStatus, Workspace } from "./types";
import { AgentConversation } from "./components/AgentConversation";
import { TerminalPane } from "./components/TerminalPane";
import { formatElapsed } from "./utils";

const emptyState: BridgeState = { projects: [], workspaces: [], sessions: [], events: [], agentEvents: [] };
const statusCopy: Record<SessionStatus, string> = { idle: "IDLE", working: "WORKING", waiting: "NEEDS YOU", ready: "READY", stopped: "STOPPED", failed: "FAILED" };
const liveStatuses: SessionStatus[] = ["working", "waiting", "ready"];
const ORCHESTRATOR_MODEL = "gpt-5.6-luna";

function Meter({ label, value, detail }: { label: string; value: number; detail: string }) {
  return <div className="meter"><span>{label}</span><div className="meter-track"><i style={{ width: `${value}%` }} /></div><b>{value}%</b><small>{detail}</small></div>;
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
  const grouped = useMemo(() => state.projects.map(project => ({ project, workspaces: state.workspaces.filter(w => w.projectId === project.id) })), [state]);
  const orchestratorReady = !!health?.adapters.find(adapter => adapter.id === "codex" && adapter.available);

  useEffect(() => {
    if (!selectedId || !("__TAURI_INTERNALS__" in window) || busy || !orchestratorReady) return;
    if (autoStartRef.current === selectedId) return;
    autoStartRef.current = selectedId;
    setBusy(true);
    void bridgeApi.startSession(selectedId, "codex", ORCHESTRATOR_MODEL)
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
  }, [selectedId, state.sessions, health, busy, orchestratorReady]);

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
      const started = await bridgeApi.startSession(workspace.id, "codex", ORCHESTRATOR_MODEL);
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
        : await bridgeApi.startSession(selected.id, "codex", ORCHESTRATOR_MODEL);
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
  async function archiveSelected() {
    if (!selected || !window.confirm(`Archive ${selected.city}? The clean worktree will be removed; its branch is preserved.`)) return;
    setBusy(true); setError(undefined);
    try { const next = await bridgeApi.archiveWorkspace(selected.id); setState(next); setSelectedId(next.workspaces[0]?.id); }
    catch (e) { setError(errorMessage(e)); }
    finally { setBusy(false); }
  }

  return <div className="app-shell">
    <header className="top-rail" data-tauri-drag-region>
      <div className="traffic-space" data-tauri-drag-region /><div className="wordmark"><span className="mark">B</span><strong>BRIDGE</strong><em>LOCAL</em></div>
      <div className="rail-meters"><Meter label="CLAUDE" value={42} detail="EST · 2H 18M" /><Meter label="CODEX" value={24} detail="EST · 3H 41M" /></div>
      <button className="command-trigger" onClick={() => setModal("palette")}><Command size={13} /><span>Jump to anything</span><kbd>⌘ K</kbd></button>
      <div className={`daemon-pill ${health?.ok ? "online" : ""}`}><span />{health?.ok ? "DAEMON ONLINE" : "CONNECTING"}</div>
    </header>
    <aside className="sidebar">
      <div className="sidebar-heading"><button className="icon-button"><PanelLeft size={15}/></button><span>WORKSPACES</span><button className="icon-button" onClick={() => setModal("workspace")} title="New workspace"><Plus size={16}/></button></div>
      <div className="workspace-scroll">
        {grouped.map(({ project, workspaces }) => <section className="project-group" key={project.id}>
          <div className="project-title"><div className="repo-icon">{project.name.slice(0,1).toUpperCase()}</div><strong>{project.name}</strong></div>
          {workspaces.map((workspace, index) => <button key={workspace.id} className={`workspace-row ${workspace.id === selectedId ? "selected" : ""}`} onClick={() => { setSelectedId(workspace.id); setSelectedSessionId(undefined); autoStartRef.current = undefined; }}>
            <span className="workspace-index">{index + 1}</span><span className="workspace-main"><b>{workspace.title}</b><small>{workspace.city} · {workspace.branch}</small></span><span className="workspace-status"><StatusDot status={workspace.status}/><small>{statusCopy[workspace.status]}</small></span>
          </button>)}
        </section>)}
        {!state.projects.length && <div className="empty-sidebar"><FolderGit2 size={25}/><p>Add a Git repository to begin.</p></div>}
      </div>
      <button className="add-repository" onClick={() => setModal("project")}><Plus size={14}/> Add repository</button>
      <nav className="sidebar-nav"><button><Inbox size={15}/> Inbox <span>{state.events.filter(e => e.kind.includes("waiting")).length}</span></button><button><LayoutGrid size={15}/> All sessions</button><button><Clock3 size={15}/> Night queue</button><button><Settings2 size={15}/> Settings</button></nav>
    </aside>
    <main className="workspace-view">
      {selected ? <>
        <div className="workspace-header">
          <div><div className="eyebrow"><StatusDot status={selected.status}/>{statusCopy[selected.status]} · {selected.city.toUpperCase()}</div><h1>{selected.title}</h1><div className="branch-line"><GitBranch size={13}/>{selected.branch}<span>·</span><span className={selected.dirtyFiles ? "dirty" : ""}>{selected.dirtyFiles ? `${selected.dirtyFiles} files changed` : "clean"}</span></div></div>
          <div className="workspace-actions"><button className="secondary"><GitPullRequest size={14}/> Review changes</button><button className="secondary archive" onClick={() => void archiveSelected()} title="Archive clean workspace"><Archive size={14}/> Archive</button>{session?.activeTurnId && <button className="secondary" onClick={() => void bridgeApi.interruptTurn(session.id)}><Square size={12}/> Stop turn</button>}{sessionConnected ? <button className="run-button stop" disabled={busy} onClick={() => void toggleSession(session)}>{busy ? <LoaderCircle className="spin" size={14}/> : <Square size={13}/>} End agent</button> : <button className="run-button" disabled={busy || !orchestratorReady} onClick={() => void toggleSession(session)} title="Restart if auto-start failed">{busy ? <LoaderCircle className="spin" size={14}/> : <Play size={14}/>} Restart</button>}</div>
        </div>
        <div className="session-strip">
          {sessions.map(s => <button className={`session-chip ${s.id === session?.id ? "active" : ""}`} key={s.id} onClick={() => setSelectedSessionId(s.id)}><span className={`harness-icon ${s.harness}`}><Bot size={14}/></span><span><b>{s.label}</b><small><StatusDot status={s.status}/>{statusCopy[s.status]} · GPT Luna</small></span></button>)}
          <div className="session-metrics"><span>ELAPSED <b>{formatElapsed(session?.startedAt, clock)}</b></span><span>CONTEXT <b>{session?.contextPercent ?? "—"}{session?.contextPercent != null ? "%" : ""}</b></span><span>USAGE <b>{session?.usagePercent ?? "—"}{session?.usagePercent != null ? "%" : ""}</b></span><small>{session?.metricSource?.toUpperCase() ?? "UNAVAILABLE"}</small></div>
        </div>
        <div className="content-tabs"><button className={activeTab === "agent" ? "active" : ""} onClick={() => setActiveTab("agent")}><MessageSquareText size={14}/> Agent</button><button className={activeTab === "changes" ? "active" : ""} onClick={() => setActiveTab("changes")}><FileCode2 size={14}/> Changes <span>{selected.dirtyFiles}</span></button><button className={activeTab === "events" ? "active" : ""} onClick={() => setActiveTab("events")}><Activity size={14}/> Events</button><button className={activeTab === "terminal" ? "active" : ""} onClick={() => setActiveTab("terminal")}><TerminalSquare size={14}/> Terminal</button></div>
        <section className="content-body">
          {activeTab === "agent" && <AgentConversation session={session} events={sessionEvents} onResolve={(eventId, decision) => void resolveApproval(eventId, decision)}/>}
          {activeTab === "changes" && <ChangesPanel workspace={selected}/>}
          {activeTab === "events" && <EventPanel state={state} workspace={selected}/>}
          {activeTab === "terminal" && <div className="terminal-layer"><TerminalPane workspaceId={selected.id}/></div>}
        </section>
        {activeTab === "agent" && <div className="composer"><div className="composer-inner"><textarea value={composer} onChange={e => setComposer(e.target.value)} onKeyDown={e => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); void sendPrompt(); } }} placeholder={sessionConnected ? "Tell Bridge what you want to build…" : busy ? "Starting orchestrator…" : "Orchestrator starting…"} disabled={!sessionConnected}/><div className="composer-tools"><span className="orchestrator-pill">Orchestrator · GPT Luna</span><span>↵ send · ⇧↵ newline</span><button className="send" onClick={() => void sendPrompt()} disabled={!composer.trim() || !sessionConnected}><Send size={14}/></button></div></div></div>}
      </> : <Welcome onAdd={() => setModal("project")}/>}
    </main>
    {error && <div className="error-toast" role="alert"><span>{error}</span><button onClick={() => setError(undefined)}><X size={14}/></button></div>}
    {modal && <Modal kind={modal} onClose={() => setModal(null)}>
      {modal === "project" && <><ModalTitle icon={<FolderGit2/>} title="Add a repository" copy="Bridge works locally and never uploads your code."/><label className="field-label">REPOSITORY PATH</label><div className="path-input"><input autoFocus value={path} onChange={e => setPath(e.target.value)} placeholder="/Users/you/Developer/project"/><button onClick={() => void chooseFolder()}>Choose…</button></div><div className="modal-actions"><button onClick={() => setModal(null)}>Cancel</button><button className="primary" disabled={!path || busy} onClick={() => void addProject()}>{busy ? "Adding…" : "Add repository"}</button></div></>}
      {modal === "workspace" && <><ModalTitle icon={<Box/>} title="New workspace" copy="Name the lane. Bridge starts the orchestrator automatically—no harness or model to pick."/><label className="field-label">WORKSPACE NAME</label><textarea className="task-input" autoFocus value={title} onChange={e => setTitle(e.target.value)} placeholder="e.g. Keyboard navigation"/><p className="modal-hint">Starts Orchestrator on GPT Luna. You describe the work in chat.</p><div className="modal-actions"><button onClick={() => setModal(null)}>Cancel</button><button className="primary" disabled={!title.trim() || !state.projects.length || busy || !orchestratorReady} onClick={() => void createWorkspace()}>{busy ? "Starting…" : "Create workspace"}</button></div></>}
      {modal === "palette" && <CommandPalette workspaces={state.workspaces} onChoose={id => { setSelectedId(id); setModal(null); autoStartRef.current = undefined; }}/>}
    </Modal>}
  </div>;
}

function ChangesPanel({ workspace }: { workspace: Workspace }) { return <div className="panel-view"><div className="panel-kicker">CHANGE STORY</div><h2>{workspace.dirtyFiles ? `${workspace.dirtyFiles} files changed` : "Workspace is clean"}</h2><p>Behavior-grouped review will live here. High-risk authentication, migrations, test weakening, and evaluation thresholds are always expanded.</p><div className="diff-stat"><b className="add">+{workspace.additions}</b><b className="del">−{workspace.deletions}</b><span/><small>{workspace.branch}</small></div><div className="placeholder-lines">{[78,92,64,85,51,70].map((n,i)=><i key={i} style={{width:`${n}%`}}/>)}</div></div>; }
function EventPanel({ state, workspace }: { state: BridgeState; workspace: Workspace }) { const events = state.events.filter(e => e.entityId === workspace.id || state.sessions.some(s => s.workspaceId === workspace.id && s.id === e.entityId)); return <div className="event-list">{events.length ? events.map(e => <article key={e.id}><CircleDot size={14}/><div><b>{e.kind.replaceAll(".", " ")}</b><p>{e.body}</p><small>{new Date(e.createdAt).toLocaleTimeString([], {hour:"2-digit",minute:"2-digit"})}</small></div></article>) : <div className="empty-panel">No events for this workspace yet.</div>}</div>; }
function Welcome({ onAdd }: { onAdd: () => void }) { return <div className="welcome"><div className="welcome-mark">B</div><div className="eyebrow">LOCAL AGENT CONTROL ROOM</div><h1>One orchestrator.<br/>Many workers later.</h1><p>Open a workspace and Bridge starts a cheap Codex orchestrator on GPT Luna.<br/>Describe what to build—no harness picker.</p><button className="primary large" onClick={onAdd}><FolderGit2 size={16}/> Add your first repository</button><small>Your code stays on this Mac.</small></div>; }
function Modal({ children, onClose, kind }: { children: React.ReactNode; onClose: () => void; kind: string }) { return <div className="modal-backdrop" onMouseDown={e => { if (e.target === e.currentTarget) onClose(); }}><div className={`modal-card ${kind === "palette" ? "palette" : ""}`}><button className="modal-close" onClick={onClose}><X size={16}/></button>{children}</div></div>; }
function ModalTitle({ icon, title, copy }: { icon: React.ReactNode; title: string; copy: string }) { return <div className="modal-title"><span>{icon}</span><div><h2>{title}</h2><p>{copy}</p></div></div>; }
function CommandPalette({ workspaces, onChoose }: { workspaces: Workspace[]; onChoose: (id:string)=>void }) { return <><div className="palette-input"><Search size={17}/><input autoFocus placeholder="Search workspaces and actions…"/></div><div className="palette-section"><label>WORKSPACES</label>{workspaces.map(w => <button key={w.id} onClick={() => onChoose(w.id)}><StatusDot status={w.status}/><span><b>{w.title}</b><small>{w.city} · {w.branch}</small></span><kbd>↵</kbd></button>)}</div><div className="palette-footer"><span>↑↓ navigate</span><span>esc close</span></div></>; }
