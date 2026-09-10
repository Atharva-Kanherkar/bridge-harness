import { useCallback, useEffect, useMemo, useRef, useState, type DragEvent, type KeyboardEvent, type ReactNode } from "react";
import { ArrowUpRight, GripVertical, LayoutGrid, Maximize2, Minimize2, Square } from "lucide-react";
import { bridgeApi } from "../api";
import { cn } from "@/lib/utils";
import type { AgentEvent, Session, SessionForestSnapshot, Workspace, WorkerRuntimeRecord } from "../types";
import type { ApprovalDecision, InteractionResolutionResult, QuestionAction } from "../protocol/generated/protocol";
import { formatElapsed, harnessLabel } from "../utils";
import { leafIds, resizeNode, type PaneNode, type SplitDirection } from "../terminal/layout";
import { AgentConversation } from "./AgentConversation";
import { ComposerPill } from "./ComposerPill";
import { isRunning, workerStatus, type WorkerTone } from "./workerStatus";
import { dropEdge, moveLeaf, readLayout, reconcileLeaves, writeLayout, type DropEdge } from "./missionControl/layout";

export type MissionControlProps = {
  sessions: Session[];
  workspaces: Workspace[];
  events: AgentEvent[];
  activeSessionId?: string;
  onFocusSession: (sessionId: string) => void;
  onStopWorker?: (childSessionId: string) => Promise<void>;
};

const TILE_DRAG = "application/x-bridge-mission-tile";
const ACTIVE_STATUSES = new Set<Session["status"]>(["working", "waiting", "starting", "resuming", "checkpointing"]);
const FOREST_DEBOUNCE_MS = 300;

// chrome stays achromatic; only the dot and the small-caps label carry hue.
const TONE_INK: Record<WorkerTone, { text: string; dot: string }> = {
  working: { text: "text-success", dot: "bg-success" },
  waiting: { text: "text-warning", dot: "bg-warning" },
  attention: { text: "text-warning", dot: "bg-warning" },
  warm: { text: "text-info", dot: "bg-info" },
  done: { text: "text-muted-foreground", dot: "bg-muted-foreground/50" },
  failed: { text: "text-destructive", dot: "bg-destructive" },
  stalled: { text: "text-destructive", dot: "bg-destructive" },
  idle: { text: "text-muted-foreground", dot: "bg-muted-foreground/50" },
};

export function isActiveSession(session: Session, runtime?: WorkerRuntimeRecord): boolean {
  if (ACTIVE_STATUSES.has(session.status)) return true;
  return !!session.parentSessionId && !!runtime && isRunning(workerStatus(session, runtime).tone);
}

type TileActions = {
  sessions: Map<string, Session>;
  workspaces: Map<string, Workspace>;
  events: AgentEvent[];
  runtimes: Map<string, WorkerRuntimeRecord>;
  activeSessionId?: string;
  expandedLeafId: string | null;
  now: number;
  onFocusSession: (sessionId: string) => void;
  onStopWorker?: (childSessionId: string) => Promise<void>;
  onForest: (sessionId: string, forest: SessionForestSnapshot) => void;
  toggleExpanded: (id: string) => void;
  move: (id: string, target: string, direction: SplitDirection, before: boolean) => void;
  resize: (path: string, ratio: number) => void;
};

function IconButton({ title, onClick, children }: { title: string; onClick: () => void; children: ReactNode }) {
  return <button type="button" title={title} aria-label={title} onClick={onClick} className="rounded p-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring">{children}</button>;
}

// durable history per tile, reloaded shortly after that session's live events move.
function useSessionForest(sessionId: string, events: AgentEvent[], onForest: (sessionId: string, forest: SessionForestSnapshot) => void) {
  const [forest, setForest] = useState<SessionForestSnapshot | null>(null);
  const last = events.at(-1);
  const eventsKey = `${events.length}:${last?.id ?? 0}:${last?.sequence ?? 0}`;
  const first = useRef(true);
  useEffect(() => {
    let cancelled = false;
    const load = () => bridgeApi.sessionForest(sessionId).then(next => { if (!cancelled) { setForest(next); onForest(sessionId, next); } }).catch(() => undefined);
    if (first.current) { first.current = false; void load(); return () => { cancelled = true; }; }
    const timer = setTimeout(() => { void load(); }, FOREST_DEBOUNCE_MS);
    return () => { cancelled = true; clearTimeout(timer); };
  }, [sessionId, eventsKey, onForest]);
  return forest;
}

function Tile({ id, actions }: { id: string; actions: TileActions }) {
  const session = actions.sessions.get(id);
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [drop, setDrop] = useState<DropEdge>();
  const events = useMemo(() => actions.events.filter(event => event.sessionId === id), [actions.events, id]);
  const forest = useSessionForest(id, events, actions.onForest);
  if (!session) return null;
  const runtime = actions.runtimes.get(id);
  const status = workerStatus(session, runtime);
  const ink = TONE_INK[status.tone];
  const working = session.status === "working" || session.activeTurnId != null;
  const isWorker = !!session.parentSessionId;
  const expanded = actions.expandedLeafId === id;
  const focused = actions.activeSessionId === id;
  const title = session.title || session.label;
  const workspace = session.workspaceId ? actions.workspaces.get(session.workspaceId) : undefined;

  const resolve = (eventId: number, decision: ApprovalDecision, optionId?: string): Promise<InteractionResolutionResult | void> => bridgeApi.resolveApproval(session.id, eventId, decision, optionId);
  const answer = (eventId: number, action: QuestionAction, answers: Record<string, string[]>): Promise<InteractionResolutionResult | void> => bridgeApi.resolveQuestion(session.id, eventId, action, answers);
  async function submit() {
    const text = draft.trim();
    if (!text || sending) return;
    setSending(true); setError(null);
    try { await bridgeApi.submitInput(session!.id, text); setDraft(""); } catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); } finally { setSending(false); }
  }
  async function interrupt() {
    setStopping(true);
    try { await bridgeApi.interruptTurn(session!.id); } catch { /* the transcript reports adapter failures */ } finally { setStopping(false); }
  }
  function onKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (event.nativeEvent.isComposing) return;
    if (event.key === "Enter" && !event.shiftKey) { event.preventDefault(); void submit(); }
  }

  return <section aria-label={`Chat ${title}`} data-session-id={id}
    onDragOver={event => { if (event.dataTransfer.types.includes(TILE_DRAG)) { event.preventDefault(); event.dataTransfer.dropEffect = "move"; setDrop(dropEdge(event.currentTarget.getBoundingClientRect(), event.clientX, event.clientY)); } }}
    onDragLeave={event => { if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setDrop(undefined); }}
    onDrop={(event: DragEvent) => {
      setDrop(undefined);
      const source = event.dataTransfer.getData(TILE_DRAG);
      if (!source || !actions.sessions.has(source)) return;
      event.preventDefault();
      const edge = dropEdge(event.currentTarget.getBoundingClientRect(), event.clientX, event.clientY);
      actions.move(source, id, edge === "left" || edge === "right" ? "horizontal" : "vertical", edge === "left" || edge === "top");
    }}
    className={cn("relative flex h-full min-h-0 min-w-0 flex-col overflow-hidden rounded-md border border-border bg-background", focused && "ring-1 ring-foreground/30")}>
    <header draggable onDragStart={event => { event.dataTransfer.setData(TILE_DRAG, id); event.dataTransfer.effectAllowed = "move"; }} className="flex h-9 shrink-0 cursor-grab items-center gap-1.5 border-b border-border bg-card px-2 active:cursor-grabbing">
      <GripVertical size={12} className="shrink-0 text-muted-foreground/60" aria-hidden="true" />
      <span title={status.detail ?? status.label} className={cn("h-1.5 w-1.5 shrink-0 rounded-full", ink.dot)} />
      <span className="min-w-0 flex-1 truncate text-xs font-medium" title={title}>{title}</span>
      <span className={cn("shrink-0 font-mono text-[10px] uppercase tracking-wide", ink.text)}>{status.label}</span>
      <span className="hidden shrink-0 text-[10px] text-muted-foreground sm:inline">{harnessLabel(session.harness)}{workspace ? ` · ${workspace.title}` : ""}</span>
      <span className="shrink-0 font-mono text-[10px] text-muted-foreground" title="Elapsed">{formatElapsed(session.startedAt, actions.now)}</span>
      <IconButton title="Focus chat" onClick={() => actions.onFocusSession(id)}><ArrowUpRight size={13} /></IconButton>
      <IconButton title={expanded ? "Restore grid" : "Maximize tile"} onClick={() => actions.toggleExpanded(id)}>{expanded ? <Minimize2 size={13} /> : <Maximize2 size={13} />}</IconButton>
      {isWorker && actions.onStopWorker && <IconButton title="Stop worker" onClick={() => { void actions.onStopWorker?.(id); }}><Square size={12} /></IconButton>}
    </header>
    <div className="relative min-h-0 flex-1 overflow-y-auto">
      <AgentConversation
        session={session}
        events={events}
        forestEntries={forest?.entries}
        entryWindow={forest?.entryWindow}
        activeLeafId={forest?.head?.activeEntryId}
        repositoryDivergence={forest?.repositoryDivergence.status}
        completion={forest?.completion}
        continuationFidelity={session.continuationFidelity}
        working={working}
        onResolve={resolve}
        onAnswerQuestion={answer}
        onOpenSession={actions.onFocusSession}
        onStopWorker={actions.onStopWorker}
        onInterrupt={interrupt}
        stopping={stopping}
        preview={false}
      />
    </div>
    <div className="shrink-0 border-t border-border">
      {error && <p role="alert" className="px-3 pt-2 text-[11px] text-destructive">{error}</p>}
      <ComposerPill layout="dock" className="max-w-none px-2 pb-2 pt-2 sm:px-2 sm:pb-2" value={draft} onChange={setDraft} onSubmit={() => { void submit(); }} onKeyDown={onKeyDown}
        placeholder={working ? `Steer ${title}` : `Message ${title}`} working={working} activeAction="steer" disabled={sending} onStop={() => { void interrupt(); }} stopping={stopping} />
    </div>
    {drop && <div aria-hidden="true" className={cn("pointer-events-none absolute z-10 rounded border-2 border-ring bg-selection/40", drop === "left" && "inset-y-0 left-0 w-1/2", drop === "right" && "inset-y-0 right-0 w-1/2", drop === "top" && "inset-x-0 top-0 h-1/2", drop === "bottom" && "inset-x-0 bottom-0 h-1/2")} />}
  </section>;
}

function SplitTree({ node, path = "", actions }: { node: PaneNode; path?: string; actions: TileActions }) {
  const container = useRef<HTMLDivElement>(null);
  if (node.type === "leaf") return <Tile id={node.leafId} actions={actions} />;
  const horizontal = node.direction === "horizontal";
  const position = (x: number, y: number) => {
    const rect = container.current?.getBoundingClientRect();
    if (rect) actions.resize(path, horizontal ? (x - rect.left) / rect.width : (y - rect.top) / rect.height);
  };
  // the ratio is runtime state, so the track template is the one inline value.
  return <div ref={container} className="grid h-full min-h-0 min-w-0" style={horizontal ? { gridTemplateColumns: `minmax(0, ${node.ratio}fr) 6px minmax(0, ${1 - node.ratio}fr)` } : { gridTemplateRows: `minmax(0, ${node.ratio}fr) 6px minmax(0, ${1 - node.ratio}fr)` }}>
    <SplitTree node={node.first} path={`${path}0`} actions={actions} />
    <div role="separator" tabIndex={0} aria-label="Resize chat split" aria-orientation={horizontal ? "vertical" : "horizontal"} aria-valuemin={10} aria-valuemax={90} aria-valuenow={Math.round(node.ratio * 100)}
      onPointerDown={event => { event.preventDefault(); event.currentTarget.setPointerCapture(event.pointerId); }}
      onPointerMove={event => { if (event.currentTarget.hasPointerCapture(event.pointerId)) position(event.clientX, event.clientY); }}
      onPointerUp={event => { position(event.clientX, event.clientY); event.currentTarget.releasePointerCapture(event.pointerId); }}
      onKeyDown={event => {
        if (!["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home"].includes(event.key)) return;
        event.preventDefault(); actions.resize(path, event.key === "Home" ? 0.5 : node.ratio + (["ArrowLeft", "ArrowUp"].includes(event.key) ? -0.05 : 0.05));
      }} className={cn("rounded transition-colors hover:bg-ring/30 focus-visible:bg-ring/40 focus-visible:outline-none", horizontal ? "cursor-col-resize" : "cursor-row-resize")} />
    <SplitTree node={node.second} path={`${path}1`} actions={actions} />
  </div>;
}

export function MissionControl({ sessions, workspaces, events, activeSessionId, onFocusSession, onStopWorker }: MissionControlProps) {
  const [showAll, setShowAll] = useState(false);
  const [forests, setForests] = useState<Record<string, SessionForestSnapshot>>({});
  const [stored, setStored] = useState(() => readLayout());
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => { const timer = setInterval(() => setNow(Date.now()), 30_000); return () => clearInterval(timer); }, []);

  const onForest = useCallback((sessionId: string, forest: SessionForestSnapshot) => setForests(prev => ({ ...prev, [sessionId]: forest })), []);
  const runtimes = useMemo(() => {
    const map = new Map<string, WorkerRuntimeRecord>();
    for (const forest of Object.values(forests)) for (const runtime of forest.workerRuntimes) map.set(runtime.sessionId, runtime);
    return map;
  }, [forests]);
  const sessionMap = useMemo(() => new Map(sessions.map(session => [session.id, session])), [sessions]);
  const workspaceMap = useMemo(() => new Map(workspaces.map(workspace => [workspace.id, workspace])), [workspaces]);
  const live = useMemo(() => sessions.filter(session => isActiveSession(session, runtimes.get(session.id))), [sessions, runtimes]);
  const visible = showAll ? sessions : live;
  const ids = useMemo(() => visible.map(session => session.id), [visible]);
  const idsKey = ids.join(" ");
  const root = useMemo(() => reconcileLeaves(stored.root, ids), [stored.root, idsKey]); // eslint-disable-line react-hooks/exhaustive-deps
  const expandedLeafId = stored.expandedLeafId && root && leafIds(root).includes(stored.expandedLeafId) ? stored.expandedLeafId : null;
  useEffect(() => { writeLayout({ version: 1, root, expandedLeafId }); }, [root, expandedLeafId]);

  const actions: TileActions = {
    sessions: sessionMap, workspaces: workspaceMap, events, runtimes, activeSessionId, expandedLeafId, now, onFocusSession, onStopWorker, onForest,
    toggleExpanded: id => setStored(prev => ({ ...prev, root, expandedLeafId: prev.expandedLeafId === id ? null : id })),
    move: (id, target, direction, before) => { if (root) setStored(prev => ({ ...prev, root: moveLeaf(root, id, target, direction, before), expandedLeafId: null })); },
    resize: (path, ratio) => { if (root) setStored(prev => ({ ...prev, root: resizeNode(root, path, ratio) })); },
  };

  return <main aria-label="Mission Control" className="flex min-h-0 flex-1 flex-col bg-background">
    <header className="flex h-11 shrink-0 items-center gap-3 border-b border-border px-4">
      <LayoutGrid size={14} className="text-muted-foreground" aria-hidden="true" />
      <h1 className="font-display text-sm font-medium">Mission Control</h1>
      <span aria-label={`${live.length} live`} className="rounded-full border border-border px-2 py-0.5 font-mono text-[10px] text-muted-foreground">{live.length} live</span>
      <div className="flex-1" />
      <button type="button" aria-pressed={showAll} onClick={() => setShowAll(value => !value)} className={cn("rounded-md border px-2.5 py-1 text-xs transition-colors", showAll ? "border-foreground/30 bg-accent text-foreground" : "border-border text-muted-foreground hover:bg-accent hover:text-foreground")}>Show all</button>
    </header>
    {root ? <div className="min-h-0 flex-1 p-2">{expandedLeafId ? <Tile id={expandedLeafId} actions={actions} /> : <SplitTree node={root} actions={actions} />}</div>
      : <div role="status" className="flex flex-1 flex-col items-center justify-center gap-3 px-8 text-center">
        <LayoutGrid size={30} strokeWidth={1.2} className="text-muted-foreground" aria-hidden="true" />
        <h2 className="font-display text-xl">No active chats</h2>
        <p className="max-w-sm text-sm leading-relaxed text-muted-foreground">Active chats appear here automatically as live tiles you can type into. {sessions.length > 0 ? `${sessions.length} idle or finished ${sessions.length === 1 ? "chat is" : "chats are"} hidden.` : "Start a chat from the sidebar to see it here."}</p>
        {sessions.length > 0 && !showAll && <button type="button" onClick={() => setShowAll(true)} className="mt-1 rounded-md border border-border-card bg-card px-4 py-2 text-xs font-medium shadow-control hover:bg-accent">Show all chats</button>}
      </div>}
  </main>;
}
