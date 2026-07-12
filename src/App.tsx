import { useCallback, useEffect, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Activity, Archive, Bot, Box, ChevronDown, CircleDot, Clock3, Code2, Command, FileCode2, FolderGit2, GitBranch, GitPullRequest, Inbox, LayoutGrid, LoaderCircle, MessageSquareText, PanelLeft, Play, Plus, Search, Send, Settings2, Square, TerminalSquare, X } from "lucide-react";
import { bridgeApi } from "./api";
import type { BridgeState, Harness, Health, Session, SessionStatus, Workspace } from "./types";
import { TerminalPane } from "./components/TerminalPane";

const emptyState: BridgeState = { projects: [], workspaces: [], sessions: [], events: [] };
const statusCopy: Record<SessionStatus, string> = { idle: "IDLE", working: "WORKING", waiting: "NEEDS YOU", ready: "READY", stopped: "STOPPED", failed: "FAILED" };

function Meter({ label, value, detail }: { label: string; value: number; detail: string }) {
  return <div className="meter"><span>{label}</span><div className="meter-track"><i style={{ width: `${value}%` }} /></div><b>{value}%</b><small>{detail}</small></div>;
}

function StatusDot({ status }: { status: SessionStatus }) { return <span className={`status-dot ${status}`} />; }

export function App() {
  const [state, setState] = useState<BridgeState>(emptyState);
  const [health, setHealth] = useState<Health>();
  const [selectedId, setSelectedId] = useState<string>();
  const [selectedSessionId, setSelectedSessionId] = useState<string>();
  const [activeTab, setActiveTab] = useState<"agent" | "changes" | "events">("agent");
  const [modal, setModal] = useState<"workspace" | "project" | "palette" | null>(null);
  const [title, setTitle] = useState("");
  const [path, setPath] = useState("");
  const [harness, setHarness] = useState<Harness>("codex");
  const [composer, setComposer] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();

  const reload = useCallback(async () => {
    const next = await bridgeApi.state(); setState(next);
    setSelectedId(current => current && next.workspaces.some(w => w.id === current) ? current : next.workspaces[0]?.id);
  }, []);

  useEffect(() => { void Promise.all([reload(), bridgeApi.health().then(setHealth)]); let off: (() => void) | undefined; void bridgeApi.onStateChanged(reload).then(fn => off = fn); return () => off?.(); }, [reload]);
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
  const sessions = state.sessions.filter(s => s.workspaceId === selectedId);
  const session = sessions.find(s => s.id === selectedSessionId) ?? sessions.find(s => s.status === "working" || s.status === "waiting") ?? sessions[0];
  const grouped = useMemo(() => state.projects.map(project => ({ project, workspaces: state.workspaces.filter(w => w.projectId === project.id) })), [state]);

  async function chooseFolder() {
    if (!("__TAURI_INTERNALS__" in window)) return setPath("/Users/you/Developer/new-project");
    const value = await open({ directory: true, multiple: false, title: "Add a Git repository" }); if (value) setPath(value);
  }
  const errorMessage = (value: unknown) => value instanceof Error ? value.message : String(value);
  async function addProject() { setBusy(true); setError(undefined); try { setState(await bridgeApi.addProject(path)); setModal(null); setPath(""); } catch (e) { setError(errorMessage(e)); } finally { setBusy(false); } }
  async function createWorkspace() {
    const projectId = selectedProject?.id ?? state.projects[0]?.id; if (!projectId || !title.trim()) return;
    setBusy(true); setError(undefined); try { const next = await bridgeApi.createWorkspace(projectId, title.trim(), harness); setState(next); setSelectedId(next.workspaces.at(-1)?.id); setModal(null); setTitle(""); } catch (e) { setError(errorMessage(e)); } finally { setBusy(false); }
  }
  async function toggleSession(target?: Session, requestedHarness: Harness = harness) {
    if (!selected) return; setBusy(true);
    try {
      setError(undefined);
      const next = target && (target.status === "working" || target.status === "waiting") ? await bridgeApi.stopSession(target.id) : await bridgeApi.startSession(selected.id, target?.harness ?? requestedHarness);
      setState(next);
      if (!target || (target.status !== "working" && target.status !== "waiting")) {
        const started = [...next.sessions].reverse().find(item => item.workspaceId === selected.id && item.status === "working");
        setSelectedSessionId(started?.id);
      }
    } catch (e) { setError(errorMessage(e)); }
    finally { setBusy(false); }
  }
  async function sendPrompt() { if (!session || !composer.trim()) return; await bridgeApi.writeSession(session.id, `${composer}\r`); setComposer(""); }
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
          <div className="project-title"><div className="repo-icon">{project.name.slice(0,1).toUpperCase()}</div><strong>{project.name}</strong><ChevronDown size={13}/></div>
          {workspaces.map((workspace, index) => <button key={workspace.id} className={`workspace-row ${workspace.id === selectedId ? "selected" : ""}`} onClick={() => { setSelectedId(workspace.id); setSelectedSessionId(undefined); }}>
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
          <div className="workspace-actions"><button className="secondary"><GitPullRequest size={14}/> Review changes</button><button className="secondary archive" onClick={() => void archiveSelected()} title="Archive clean workspace"><Archive size={14}/> Archive</button><button className={`run-button ${session?.status === "working" ? "stop" : ""}`} disabled={busy} onClick={() => void toggleSession(session)}>{busy ? <LoaderCircle className="spin" size={14}/> : session?.status === "working" || session?.status === "waiting" ? <Square size={13}/> : <Play size={14}/>} {session?.status === "working" || session?.status === "waiting" ? "Stop" : "Start agent"}</button></div>
        </div>
        <div className="session-strip">
          {sessions.map(s => <button className={`session-chip ${s.id === session?.id ? "active" : ""}`} key={s.id} onClick={() => setSelectedSessionId(s.id)}><span className={`harness-icon ${s.harness}`}>{s.harness === "shell" ? <TerminalSquare size={14}/> : <Bot size={14}/>}</span><span><b>{s.label}</b><small><StatusDot status={s.status}/>{statusCopy[s.status]}</small></span></button>)}
          <button className="new-session" onClick={() => void toggleSession(undefined, session?.harness === "codex" ? "claude" : "codex")}><Plus size={14}/> Agent</button>
          <div className="session-metrics"><span>CONTEXT <b>{session?.contextPercent ?? "—"}{session?.contextPercent != null ? "%" : ""}</b></span><span>USAGE <b>{session?.usagePercent ?? "—"}{session?.usagePercent != null ? "%" : ""}</b></span><small>{session?.metricSource?.toUpperCase() ?? "UNAVAILABLE"}</small></div>
        </div>
        <div className="content-tabs"><button className={activeTab === "agent" ? "active" : ""} onClick={() => setActiveTab("agent")}><MessageSquareText size={14}/> Agent</button><button className={activeTab === "changes" ? "active" : ""} onClick={() => setActiveTab("changes")}><FileCode2 size={14}/> Changes <span>{selected.dirtyFiles}</span></button><button className={activeTab === "events" ? "active" : ""} onClick={() => setActiveTab("events")}><Activity size={14}/> Events</button></div>
        <section className="content-body">
          <div className={`terminal-layer ${activeTab === "agent" ? "" : "hidden"}`}><TerminalPane session={session}/></div>
          {activeTab === "changes" && <ChangesPanel workspace={selected}/>} 
          {activeTab === "events" && <EventPanel state={state} workspace={selected}/>} 
        </section>
        {activeTab === "agent" && <div className="composer"><div className="composer-inner"><textarea value={composer} onChange={e => setComposer(e.target.value)} onKeyDown={e => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); void sendPrompt(); } }} placeholder={session?.status === "working" || session?.status === "waiting" ? `Steer ${session.label}…` : "Start an agent to send a task…"} disabled={!session || session.status === "stopped"}/><div className="composer-tools"><button className="tool-select"><Code2 size={13}/>{session?.harness ?? harness}<ChevronDown size={12}/></button><span>↵ send · ⇧↵ newline</span><button className="send" onClick={() => void sendPrompt()} disabled={!composer.trim()}><Send size={14}/></button></div></div></div>}
      </> : <Welcome onAdd={() => setModal("project")}/>} 
    </main>
    {error && <div className="error-toast" role="alert"><span>{error}</span><button onClick={() => setError(undefined)}><X size={14}/></button></div>}
    {modal && <Modal kind={modal} onClose={() => setModal(null)}>
      {modal === "project" && <><ModalTitle icon={<FolderGit2/>} title="Add a repository" copy="Bridge works locally and never uploads your code."/><label className="field-label">REPOSITORY PATH</label><div className="path-input"><input autoFocus value={path} onChange={e => setPath(e.target.value)} placeholder="/Users/you/Developer/project"/><button onClick={() => void chooseFolder()}>Choose…</button></div><div className="modal-actions"><button onClick={() => setModal(null)}>Cancel</button><button className="primary" disabled={!path || busy} onClick={() => void addProject()}>{busy ? "Adding…" : "Add repository"}</button></div></>}
      {modal === "workspace" && <><ModalTitle icon={<Box/>} title="New workspace" copy="A fresh branch and isolated Git worktree for this task."/><label className="field-label">WHAT SHOULD THE AGENT DO?</label><textarea className="task-input" autoFocus value={title} onChange={e => setTitle(e.target.value)} placeholder="e.g. Add keyboard navigation to the command palette"/><label className="field-label">START WITH</label><div className="harness-picker">{(["codex","claude","shell"] as Harness[]).map(h => <button key={h} className={harness === h ? "selected" : ""} onClick={() => setHarness(h)}><span className={`harness-icon ${h}`}>{h === "shell" ? <TerminalSquare/> : <Bot/>}</span><b>{h[0].toUpperCase()+h.slice(1)}</b><small>{health?.harnesses[h] ? "Available" : h === "shell" ? "Available" : "Not detected"}</small></button>)}</div><div className="modal-actions"><button onClick={() => setModal(null)}>Cancel</button><button className="primary" disabled={!title.trim() || !state.projects.length || busy} onClick={() => void createWorkspace()}>{busy ? "Creating worktree…" : "Create workspace"}</button></div></>}
      {modal === "palette" && <CommandPalette workspaces={state.workspaces} onChoose={id => { setSelectedId(id); setModal(null); }}/>} 
    </Modal>}
  </div>;
}

function ChangesPanel({ workspace }: { workspace: Workspace }) { return <div className="panel-view"><div className="panel-kicker">CHANGE STORY</div><h2>{workspace.dirtyFiles ? `${workspace.dirtyFiles} files changed` : "Workspace is clean"}</h2><p>Behavior-grouped review will live here. High-risk authentication, migrations, test weakening, and evaluation thresholds are always expanded.</p><div className="diff-stat"><b className="add">+{workspace.additions}</b><b className="del">−{workspace.deletions}</b><span/><small>{workspace.branch}</small></div><div className="placeholder-lines">{[78,92,64,85,51,70].map((n,i)=><i key={i} style={{width:`${n}%`}}/>)}</div></div>; }
function EventPanel({ state, workspace }: { state: BridgeState; workspace: Workspace }) { const events = state.events.filter(e => e.entityId === workspace.id || state.sessions.some(s => s.workspaceId === workspace.id && s.id === e.entityId)); return <div className="event-list">{events.length ? events.map(e => <article key={e.id}><CircleDot size={14}/><div><b>{e.kind.replaceAll(".", " ")}</b><p>{e.body}</p><small>{new Date(e.createdAt).toLocaleTimeString([], {hour:"2-digit",minute:"2-digit"})}</small></div></article>) : <div className="empty-panel">No events for this workspace yet.</div>}</div>; }
function Welcome({ onAdd }: { onAdd: () => void }) { return <div className="welcome"><div className="welcome-mark">B</div><div className="eyebrow">LOCAL AGENT CONTROL ROOM</div><h1>Put every coding agent<br/>in its own lane.</h1><p>Run Claude, Codex, and shells in isolated worktrees.<br/>See what needs you without watching terminals.</p><button className="primary large" onClick={onAdd}><FolderGit2 size={16}/> Add your first repository</button><small>Your code stays on this Mac.</small></div>; }
function Modal({ children, onClose, kind }: { children: React.ReactNode; onClose: () => void; kind: string }) { return <div className="modal-backdrop" onMouseDown={e => { if (e.target === e.currentTarget) onClose(); }}><div className={`modal-card ${kind === "palette" ? "palette" : ""}`}><button className="modal-close" onClick={onClose}><X size={16}/></button>{children}</div></div>; }
function ModalTitle({ icon, title, copy }: { icon: React.ReactNode; title: string; copy: string }) { return <div className="modal-title"><span>{icon}</span><div><h2>{title}</h2><p>{copy}</p></div></div>; }
function CommandPalette({ workspaces, onChoose }: { workspaces: Workspace[]; onChoose: (id:string)=>void }) { return <><div className="palette-input"><Search size={17}/><input autoFocus placeholder="Search workspaces and actions…"/></div><div className="palette-section"><label>WORKSPACES</label>{workspaces.map(w => <button key={w.id} onClick={() => onChoose(w.id)}><StatusDot status={w.status}/><span><b>{w.title}</b><small>{w.city} · {w.branch}</small></span><kbd>↵</kbd></button>)}</div><div className="palette-footer"><span>↑↓ navigate</span><span>esc close</span></div></>; }
