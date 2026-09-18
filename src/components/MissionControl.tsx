import { useCallback, useEffect, useMemo, useRef, useState, type DragEvent, type KeyboardEvent, type ReactNode } from "react";
import { ArrowUpRight, GripVertical, LayoutGrid, Maximize2, Minimize2, Pin, PinOff, Square, X } from "lucide-react";
import { bridgeApi } from "../api";
import { useShowWorkerChatsInMissionControl } from "../missionControlSettings";
import { cn } from "@/lib/utils";
import type { AgentEvent, Session, SessionForestSnapshot, Workspace, WorkerRuntimeRecord } from "../types";
import type { ApprovalDecision, InteractionResolutionResult, QuestionAction } from "../protocol/generated/protocol";
import { formatElapsed, harnessLabel } from "../utils";
import { leafIds, resizeNode, type PaneNode } from "../terminal/layout";
import { AgentConversation } from "./AgentConversation";
import { ComposerPill } from "./ComposerPill";
import { workerStatus, type WorkerTone } from "./workerStatus";
import { dropEdge, insertLeaf, minimumSize, moveLeaf, readLayout, reconcileLeaves, writeLayout, type DropEdge } from "./missionControl/layout";
import { isChatDrag, readChatDrag, SIDEBAR_CHAT_DRAG, TILE_DRAG } from "./missionControl/drag";

export type MissionControlProps = {
  sessions: Session[];
  workspaces: Workspace[];
  events: AgentEvent[];
  activeSessionId?: string;
  onFocusSession: (sessionId: string) => void;
  onStopWorker?: (childSessionId: string) => Promise<void>;
};

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

export function isActiveSession(session: Session): boolean {
  // Session updates are live; forest runtime snapshots can outlive a worker's turn.
  return ACTIVE_STATUSES.has(session.status) || session.activeTurnId != null;
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
  dropChat: (id: string, fromSidebar: boolean, target?: string, edge?: DropEdge) => void;
  pinnedSessionIds: string[];
  pin: (id: string) => void;
  unpin: (id: string) => void;
  dismiss: (id: string) => void;
  drafts: Record<string, string>;
  setDraft: (id: string, draft: string) => void;
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
  const draft = actions.drafts[id] ?? "";
  const setDraft = (value: string) => actions.setDraft(id, value);
  const [sending, setSending] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [drop, setDrop] = useState<DropEdge>();
  useEffect(() => {
    const clear = () => setDrop(undefined);
    window.addEventListener("dragend", clear);
    window.addEventListener("drop", clear);
    return () => { window.removeEventListener("dragend", clear); window.removeEventListener("drop", clear); };
  }, []);
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
    onDragOver={event => { if (isChatDrag(event.dataTransfer)) { event.preventDefault(); event.stopPropagation(); event.dataTransfer.dropEffect = event.dataTransfer.types.includes(SIDEBAR_CHAT_DRAG) ? "copy" : "move"; setDrop(dropEdge(event.currentTarget.getBoundingClientRect(), event.clientX, event.clientY)); } }}
    onDragLeave={event => { if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setDrop(undefined); }}
    onDrop={(event: DragEvent) => {
      setDrop(undefined);
      const source = readChatDrag(event.dataTransfer);
      if (!source.id || !actions.sessions.has(source.id)) return;
      event.preventDefault();
      event.stopPropagation();
      const edge = dropEdge(event.currentTarget.getBoundingClientRect(), event.clientX, event.clientY);
      actions.dropChat(source.id, source.fromSidebar, id, edge);
    }}
    className={cn("relative flex h-full min-h-0 min-w-0 flex-col overflow-hidden rounded-md border border-border bg-background", focused && "ring-1 ring-foreground/30")}>
    <header draggable title="Drag to rearrange. Drop on an edge to split." onDragStart={event => { event.dataTransfer.setData(TILE_DRAG, id); event.dataTransfer.effectAllowed = "move"; }} className="flex h-9 shrink-0 cursor-grab select-none items-center gap-1.5 border-b border-border bg-card px-2 active:cursor-grabbing">
      <GripVertical size={12} className="shrink-0 text-muted-foreground/60" aria-hidden="true" />
      <span title={status.detail ?? status.label} className={cn("h-1.5 w-1.5 shrink-0 rounded-full", ink.dot)} />
      <span className="min-w-0 flex-1 truncate text-xs font-medium" title={title}>{title}</span>
      <span className={cn("shrink-0 font-mono text-[10px] uppercase tracking-wide", ink.text)}>{status.label}</span>
      <span className="hidden max-w-28 truncate text-[10px] text-muted-foreground sm:inline" title={`${harnessLabel(session.harness)}${workspace ? ` · ${workspace.title}` : ""}`}>{harnessLabel(session.harness)}{workspace ? ` · ${workspace.title}` : ""}</span>
      <span className="shrink-0 font-mono text-[10px] text-muted-foreground" title="Elapsed">{formatElapsed(session.startedAt, actions.now)}</span>
      <IconButton title="Focus chat" onClick={() => actions.onFocusSession(id)}><ArrowUpRight size={13} /></IconButton>
      <IconButton title={expanded ? "Restore grid" : "Maximize tile"} onClick={() => actions.toggleExpanded(id)}>{expanded ? <Minimize2 size={13} /> : <Maximize2 size={13} />}</IconButton>
      {isWorker && actions.onStopWorker && <IconButton title="Stop worker" onClick={() => { void actions.onStopWorker?.(id); }}><Square size={12} /></IconButton>}
      {actions.pinnedSessionIds.includes(id)
        ? <IconButton title="Unpin chat" onClick={() => actions.unpin(id)}><PinOff size={13} /></IconButton>
        : <IconButton title="Pin chat in Mission Control" onClick={() => actions.pin(id)}><Pin size={13} /></IconButton>}
      <IconButton title="Close chat" onClick={() => actions.dismiss(id)}><X size={13} /></IconButton>
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
    {drop && <div aria-hidden="true" data-drop-edge={drop} className={cn("pointer-events-none absolute z-10 flex items-center justify-center rounded border-2 border-ring bg-selection/80 text-xs font-medium text-selection-foreground", drop === "left" && "inset-y-0 left-0 w-1/2", drop === "right" && "inset-y-0 right-0 w-1/2", drop === "top" && "inset-x-0 top-0 h-1/2", drop === "bottom" && "inset-x-0 bottom-0 h-1/2")}>Drop to split {drop}</div>}
  </section>;
}

function SplitTree({ node, path = "", actions }: { node: PaneNode; path?: string; actions: TileActions }) {
  const container = useRef<HTMLDivElement>(null);
  if (node.type === "leaf") return <Tile key={node.leafId} id={node.leafId} actions={actions} />;
  const horizontal = node.direction === "horizontal";
  const firstSize = minimumSize(node.first);
  const secondSize = minimumSize(node.second);
  const position = (x: number, y: number) => {
    const rect = container.current?.getBoundingClientRect();
    if (rect) actions.resize(path, horizontal ? (x - rect.left) / rect.width : (y - rect.top) / rect.height);
  };
  // the ratio is runtime state, so the track template is the one inline value.
  return <div ref={container} className="grid h-full min-h-0 min-w-0" style={horizontal ? { gridTemplateColumns: `minmax(${firstSize.width}px, ${node.ratio}fr) 6px minmax(${secondSize.width}px, ${1 - node.ratio}fr)` } : { gridTemplateRows: `minmax(${firstSize.height}px, ${node.ratio}fr) 6px minmax(${secondSize.height}px, ${1 - node.ratio}fr)` }}>
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
  const [forests, setForests] = useState<Record<string, SessionForestSnapshot>>({});
  const [stored, setStored] = useState(() => readLayout());
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [dropOnCanvas, setDropOnCanvas] = useState(false);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => { const timer = setInterval(() => setNow(Date.now()), 30_000); return () => clearInterval(timer); }, []);
  useEffect(() => {
    const clear = () => setDropOnCanvas(false);
    window.addEventListener("dragend", clear);
    return () => window.removeEventListener("dragend", clear);
  }, []);

  const onForest = useCallback((sessionId: string, forest: SessionForestSnapshot) => setForests(prev => ({ ...prev, [sessionId]: forest })), []);
  const runtimes = useMemo(() => {
    const map = new Map<string, WorkerRuntimeRecord>();
    for (const forest of Object.values(forests)) for (const runtime of forest.workerRuntimes) map.set(runtime.sessionId, runtime);
    return map;
  }, [forests]);
  const sessionMap = useMemo(() => new Map(sessions.map(session => [session.id, session])), [sessions]);
  const workspaceMap = useMemo(() => new Map(workspaces.map(workspace => [workspace.id, workspace])), [workspaces]);
  const [showWorkerChats] = useShowWorkerChatsInMissionControl();
  const dismissedSessionIds = useMemo(() => stored.dismissedSessionIds.filter(id => sessionMap.has(id)), [stored.dismissedSessionIds, sessionMap]);
  const live = useMemo(
    () => sessions.filter(session => isActiveSession(session) && (showWorkerChats || !session.parentSessionId) && !dismissedSessionIds.includes(session.id)),
    [sessions, showWorkerChats, dismissedSessionIds],
  );
  const pinnedSessionIds = useMemo(() => stored.pinnedSessionIds.filter(id => sessionMap.has(id) && !dismissedSessionIds.includes(id)), [stored.pinnedSessionIds, sessionMap, dismissedSessionIds]);
  const ids = useMemo(() => [...new Set([...live.map(session => session.id), ...pinnedSessionIds])], [live, pinnedSessionIds]);
  // Includes active pinned workers the visibility filter kept out of `live` —
  // otherwise a pinned worker can be visible and working while this reads 0.
  const activeCount = useMemo(() => ids.filter(id => { const session = sessionMap.get(id); return session && isActiveSession(session); }).length, [ids, sessionMap]);
  const idsKey = ids.join(" ");
  const root = useMemo(() => reconcileLeaves(stored.root, ids), [stored.root, idsKey]); // eslint-disable-line react-hooks/exhaustive-deps
  const expandedLeafId = stored.expandedLeafId && root && leafIds(root).includes(stored.expandedLeafId) ? stored.expandedLeafId : null;
  const size = root ? minimumSize(expandedLeafId ? { type: "leaf", leafId: expandedLeafId } : root) : undefined;
  useEffect(() => { setStored(prev => prev.root === root && prev.expandedLeafId === expandedLeafId && prev.pinnedSessionIds.length === pinnedSessionIds.length && prev.dismissedSessionIds.length === dismissedSessionIds.length ? prev : { version: 1, root, expandedLeafId, pinnedSessionIds, dismissedSessionIds }); }, [root, expandedLeafId, pinnedSessionIds, dismissedSessionIds]);
  useEffect(() => { writeLayout({ version: 1, root, expandedLeafId, pinnedSessionIds, dismissedSessionIds }); }, [root, expandedLeafId, pinnedSessionIds, dismissedSessionIds]);

  function dropChat(id: string, fromSidebar: boolean, target?: string, edge?: DropEdge) {
    setDropOnCanvas(false);
    if (!sessionMap.has(id) || (!fromSidebar && !ids.includes(id))) return;
    const next = root && target && edge
      ? moveLeaf(root, id, target, edge === "left" || edge === "right" ? "horizontal" : "vertical", edge === "left" || edge === "top")
      : insertLeaf(root, id);
    setStored({
      version: 1, root: next, expandedLeafId: null,
      pinnedSessionIds: fromSidebar ? [...new Set([...pinnedSessionIds, id])] : pinnedSessionIds,
      // Bringing a chat back in by hand is what un-closes it.
      dismissedSessionIds: dismissedSessionIds.filter(dismissed => dismissed !== id),
    });
  }

  const actions: TileActions = {
    sessions: sessionMap, workspaces: workspaceMap, events, runtimes, activeSessionId, expandedLeafId, now, onFocusSession, onStopWorker, onForest,
    toggleExpanded: id => setStored(prev => ({ ...prev, root, expandedLeafId: prev.expandedLeafId === id ? null : id })),
    dropChat, pinnedSessionIds, drafts,
    setDraft: (id, draft) => setDrafts(prev => ({ ...prev, [id]: draft })),
    pin: id => setStored(prev => ({ ...prev, pinnedSessionIds: [...new Set([...prev.pinnedSessionIds, id])], dismissedSessionIds: prev.dismissedSessionIds.filter(dismissed => dismissed !== id) })),
    unpin: id => setStored(prev => ({ ...prev, pinnedSessionIds: prev.pinnedSessionIds.filter(pinned => pinned !== id) })),
    dismiss: id => setStored(prev => ({ ...prev, pinnedSessionIds: prev.pinnedSessionIds.filter(pinned => pinned !== id), dismissedSessionIds: [...new Set([...prev.dismissedSessionIds, id])] })),
    resize: (path, ratio) => { if (root) setStored(prev => ({ ...prev, root: resizeNode(root, path, ratio) })); },
  };

  return <main aria-label="Mission Control"
    onDragOver={event => { if (isChatDrag(event.dataTransfer)) { event.preventDefault(); event.dataTransfer.dropEffect = event.dataTransfer.types.includes(SIDEBAR_CHAT_DRAG) ? "copy" : "move"; setDropOnCanvas(true); } }}
    onDragLeave={event => { if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setDropOnCanvas(false); }}
    onDrop={event => {
      setDropOnCanvas(false);
      if (!isChatDrag(event.dataTransfer)) return;
      event.preventDefault();
      const source = readChatDrag(event.dataTransfer);
      dropChat(source.id, source.fromSidebar);
    }}
    className={cn("flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-background", dropOnCanvas && !root && "ring-2 ring-inset ring-ring")}>
    <header className="flex h-11 shrink-0 items-center gap-3 border-b border-border px-4">
      <LayoutGrid size={14} className="text-muted-foreground" aria-hidden="true" />
      <h1 className="font-display text-sm font-medium">Mission Control</h1>
      <span aria-label={`${activeCount} live`} className="rounded-full border border-border px-2 py-0.5 font-mono text-[10px] text-muted-foreground">{activeCount} live</span>
      {pinnedSessionIds.length > 0 && <span className="text-xs text-muted-foreground">{pinnedSessionIds.length} pinned</span>}
      <span className="ml-auto hidden truncate text-xs text-muted-foreground lg:block">Drag chats here · Drag headers to rearrange</span>
    </header>
    {root ? <div className="min-h-0 min-w-0 flex-1 overflow-auto p-2"><div className="h-full" style={{ minWidth: size?.width, minHeight: size?.height }}>{expandedLeafId ? <Tile key={expandedLeafId} id={expandedLeafId} actions={actions} /> : <SplitTree node={root} actions={actions} />}</div></div>
      : <div role="status" className="flex flex-1 flex-col items-center justify-center gap-3 px-8 text-center">
        <LayoutGrid size={30} strokeWidth={1.2} className="text-muted-foreground" aria-hidden="true" />
        <h2 className="font-display text-xl">No active chats</h2>
        <p className="max-w-sm text-sm leading-relaxed text-muted-foreground">Chats and agents appear here automatically while working or waiting for your input. Drag any chat from the sidebar to keep it here, even when idle.</p>
      </div>}
  </main>;
}
