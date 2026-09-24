import { useCallback, useEffect, useMemo, useRef, useState, type DragEvent, type KeyboardEvent, type ReactNode } from "react";
import { ArrowUpRight, Hand, LayoutGrid, Maximize2, Minimize2, Pin, PinOff, Square, X } from "lucide-react";
import { bridgeApi } from "../api";
import { useShowWorkerChatsInMissionControl } from "../missionControlSettings";
import { cn } from "@/lib/utils";
import type { AgentEvent, Project, Session, SessionForestSnapshot, Workspace, WorkerRuntimeRecord } from "../types";
import type { ApprovalDecision, InteractionResolutionResult, QuestionAction } from "../protocol/generated/protocol";
import { formatElapsed, harnessLabel } from "../utils";
import { leafIds, resizeNode, type PaneNode } from "../terminal/layout";
import { AgentConversation } from "./AgentConversation";
import { ComposerPill } from "./ComposerPill";
import { HarnessMark } from "./harnessMarks";
import { workerStatus, type WorkerStatus, type WorkerTone } from "./workerStatus";
import { arrangeLeaves, dropEdge, groupedOrder, insertLeaf, minimumSize, moveLeaf, readLayout, reconcileLeaves, writeLayout, type DropEdge } from "./missionControl/layout";
import { isChatDrag, readChatDrag, SIDEBAR_CHAT_DRAG, TILE_DRAG } from "./missionControl/drag";
import { displayTitle, latestAsk, projectLabel } from "./missionControl/identity";

export type MissionControlProps = {
  sessions: Session[];
  workspaces: Workspace[];
  projects?: Project[];
  events: AgentEvent[];
  activeSessionId?: string;
  onFocusSession: (sessionId: string) => void;
  onStopWorker?: (childSessionId: string) => Promise<void>;
};

const ACTIVE_STATUSES = new Set<Session["status"]>(["working", "waiting", "starting", "resuming", "checkpointing"]);
const FOREST_DEBOUNCE_MS = 300;
const PLACEHOLDER_LABELS = new Set(["orchestrator", "bridge orchestrator", "new chat"]);

// one signal pops: a tile that needs you is the only inverted ink on the board.
// warning is a warm grey by design, so the salience is luminance plus an icon and
// words, never a tint. running work stays quiet; failure keeps its red.
const TONE_INK: Record<WorkerTone, { text: string; dot: string }> = {
  working: { text: "text-foreground", dot: "bg-foreground" },
  waiting: { text: "text-foreground", dot: "bg-foreground" },
  attention: { text: "text-foreground", dot: "bg-foreground" },
  warm: { text: "text-muted-foreground", dot: "border border-muted-foreground" },
  done: { text: "text-muted-foreground", dot: "border border-muted-foreground" },
  failed: { text: "text-destructive", dot: "bg-destructive" },
  stalled: { text: "text-destructive", dot: "bg-destructive" },
  idle: { text: "text-muted-foreground", dot: "border border-muted-foreground" },
};

export function isActiveSession(session: Session): boolean {
  // Session updates are live; forest runtime snapshots can outlive a worker's turn.
  return ACTIVE_STATUSES.has(session.status) || session.activeTurnId != null;
}

// a worker tile can render before any forest has carried its runtime; the live
// session status is a better answer than "status unavailable" meanwhile.
function tileStatus(session: Session, runtime?: WorkerRuntimeRecord): WorkerStatus {
  return workerStatus(runtime || !session.parentSessionId ? session : { ...session, parentSessionId: null }, runtime);
}

const needsYou = (status: WorkerStatus) => status.tone === "waiting" || status.tone === "attention";
const sentenceCase = (label: string) => label.charAt(0) + label.slice(1).toLowerCase();

type TileActions = {
  sessions: Map<string, Session>;
  workspaces: Map<string, Workspace>;
  events: AgentEvent[];
  statusOf: (id: string) => WorkerStatus | undefined;
  projectOf: (id: string) => string | null;
  highlightedProject: string | null;
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
  return <button type="button" title={title} aria-label={title} onClick={onClick} className="grid h-6 w-6 place-items-center rounded text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring">{children}</button>;
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
  const ask = useMemo(() => latestAsk(forest?.entries), [forest?.entries]);
  if (!session) return null;
  const status = actions.statusOf(id) ?? tileStatus(session);
  const ink = TONE_INK[status.tone];
  const attention = needsYou(status);
  const working = session.status === "working" || session.activeTurnId != null;
  const isWorker = !!session.parentSessionId;
  const expanded = actions.expandedLeafId === id;
  const focused = actions.activeSessionId === id;
  const pinned = actions.pinnedSessionIds.includes(id);
  const project = actions.projectOf(id);
  const dimmed = !!actions.highlightedProject && actions.highlightedProject !== project;
  const workspace = session.workspaceId ? actions.workspaces.get(session.workspaceId) : undefined;
  const stored = session.title?.trim() || session.label;
  // a chat still wearing its birth label is better named by what was asked.
  const unnamed = !session.title?.trim() && PLACEHOLDER_LABELS.has(session.label.trim().toLowerCase());
  const title = unnamed && ask ? ask : displayTitle(stored, project);
  const subtitle = !unnamed && ask ? ask : [harnessLabel(session.harness), session.model].filter(Boolean).join(" · ");

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

  return <section aria-label={`Chat ${title}`} data-session-id={id} data-attention={attention || undefined} data-project-highlight={actions.highlightedProject && !dimmed ? "true" : undefined}
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
    className={cn("relative flex h-full min-h-0 min-w-0 flex-col overflow-hidden rounded-lg border bg-background transition-opacity duration-200",
      attention ? "border-foreground/45" : "border-border",
      focused && "ring-1 ring-ring/60",
      dimmed && "opacity-40")}>
    <header draggable title="Drag to move this tile. Drop it on another tile's edge to split." onDragStart={event => { event.dataTransfer.setData(TILE_DRAG, id); event.dataTransfer.effectAllowed = "move"; }}
      className="group/header flex shrink-0 cursor-grab select-none flex-col gap-0.5 border-b border-border bg-card py-1.5 pl-3 pr-1.5 active:cursor-grabbing">
      <div className="relative flex h-6 min-w-0 items-center gap-2">
        {project && <span title={workspace?.branch ? `${project} · ${workspace.branch}` : project} className="max-w-[40%] shrink-0 truncate rounded bg-accent px-1.5 py-px text-[11px] font-medium leading-4 text-muted-foreground">{project}</span>}
        <h2 className="min-w-0 flex-1 truncate text-[13px] font-medium text-foreground" title={stored}>{title}</h2>
        <span className={cn("flex shrink-0 items-center gap-1.5 text-[11px]", ink.text)} title={status.detail ?? undefined}>
          {pinned && <Pin size={11} aria-label="Pinned" className="text-muted-foreground" />}
          {attention
            ? <span className="inline-flex items-center gap-1 rounded-full bg-primary px-2 py-0.5 font-medium text-primary-foreground"><Hand size={11} aria-hidden="true" />{sentenceCase(status.label)}</span>
            : <><span aria-hidden="true" className={cn("h-1.5 w-1.5 rounded-full", ink.dot)} /><span className="font-medium">{sentenceCase(status.label)}</span></>}
          <span className="font-mono text-muted-foreground" title="Elapsed">{formatElapsed(session.startedAt, actions.now)}</span>
        </span>
        {/* revealed over the status on header hover or keyboard focus: the
            actions stay one tab stop away without crowding the title at rest. */}
        <div className="pointer-events-none absolute right-6 top-0 flex h-6 items-center gap-0.5 bg-card pl-2 opacity-0 transition-opacity group-hover/header:pointer-events-auto group-hover/header:opacity-100 group-focus-within/header:pointer-events-auto group-focus-within/header:opacity-100">
          <IconButton title="Focus chat" onClick={() => actions.onFocusSession(id)}><ArrowUpRight size={13} /></IconButton>
          <IconButton title={expanded ? "Restore grid" : "Maximize tile"} onClick={() => actions.toggleExpanded(id)}>{expanded ? <Minimize2 size={13} /> : <Maximize2 size={13} />}</IconButton>
          {pinned
            ? <IconButton title="Unpin chat" onClick={() => actions.unpin(id)}><PinOff size={13} /></IconButton>
            : <IconButton title="Pin chat in Mission Control" onClick={() => actions.pin(id)}><Pin size={13} /></IconButton>}
          {isWorker && actions.onStopWorker && <IconButton title="Stop worker" onClick={() => { void actions.onStopWorker?.(id); }}><Square size={12} /></IconButton>}
        </div>
        <IconButton title="Close chat" onClick={() => actions.dismiss(id)}><X size={13} /></IconButton>
      </div>
      <p className="flex h-4 min-w-0 items-center gap-1.5 pr-1.5 text-[11px] text-muted-foreground">
        <span title={[harnessLabel(session.harness), session.model].filter(Boolean).join(" · ")} className="shrink-0"><HarnessMark harness={session.harness} size={11} /></span>
        <span data-tile-subtitle className="min-w-0 truncate" title={subtitle}>{subtitle}</span>
      </p>
    </header>
    <div className="relative min-h-0 flex-1 overflow-hidden">
      <AgentConversation
        session={session}
        projectName={project ?? undefined}
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
        density="compact"
      />
    </div>
    <div className="shrink-0 border-t border-border">
      {error && <p role="alert" className="px-3 pt-2 text-[11px] text-destructive">{error}</p>}
      <ComposerPill layout="inline" value={draft} onChange={setDraft} onSubmit={() => { void submit(); }} onKeyDown={onKeyDown}
        placeholder={working ? "Steer this chat…" : "Reply…"} working={working} activeAction="steer" disabled={sending} onStop={() => { void interrupt(); }} stopping={stopping} />
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

export function MissionControl({ sessions, workspaces, projects = [], events, activeSessionId, onFocusSession, onStopWorker }: MissionControlProps) {
  const [forests, setForests] = useState<Record<string, SessionForestSnapshot>>({});
  const [stored, setStored] = useState(() => readLayout());
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [dropOnCanvas, setDropOnCanvas] = useState(false);
  const [hoveredProject, setHoveredProject] = useState<string | null>(null);
  const [heldProject, setHeldProject] = useState<string | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const board = useRef<HTMLElement>(null);
  const attentionCursor = useRef(0);
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
  const projectMap = useMemo(() => new Map(sessions.map(session => [session.id, projectLabel(session, session.workspaceId ? workspaceMap.get(session.workspaceId) : undefined, projects)])), [sessions, workspaceMap, projects]);
  const projectOf = useCallback((id: string) => projectMap.get(id) ?? null, [projectMap]);
  const [showWorkerChats] = useShowWorkerChatsInMissionControl();
  const dismissedSessionIds = useMemo(() => stored.dismissedSessionIds.filter(id => sessionMap.has(id)), [stored.dismissedSessionIds, sessionMap]);
  const live = useMemo(
    () => sessions.filter(session => isActiveSession(session) && (showWorkerChats || !session.parentSessionId) && !dismissedSessionIds.includes(session.id)),
    [sessions, showWorkerChats, dismissedSessionIds],
  );
  const pinnedSessionIds = useMemo(() => stored.pinnedSessionIds.filter(id => sessionMap.has(id) && !dismissedSessionIds.includes(id)), [stored.pinnedSessionIds, sessionMap, dismissedSessionIds]);
  const ids = useMemo(() => [...new Set([...live.map(session => session.id), ...pinnedSessionIds])], [live, pinnedSessionIds]);
  const idsKey = ids.join(" ");
  // new tiles open beside their own project's tiles; nothing already on the board moves.
  const root = useMemo(() => reconcileLeaves(stored.root, ids, projectOf), [stored.root, idsKey]); // eslint-disable-line react-hooks/exhaustive-deps
  const expandedLeafId = stored.expandedLeafId && root && leafIds(root).includes(stored.expandedLeafId) ? stored.expandedLeafId : null;
  const size = root ? minimumSize(expandedLeafId ? { type: "leaf", leafId: expandedLeafId } : root) : undefined;
  useEffect(() => { setStored(prev => prev.root === root && prev.expandedLeafId === expandedLeafId && prev.pinnedSessionIds.length === pinnedSessionIds.length && prev.dismissedSessionIds.length === dismissedSessionIds.length ? prev : { version: 1, root, expandedLeafId, pinnedSessionIds, dismissedSessionIds }); }, [root, expandedLeafId, pinnedSessionIds, dismissedSessionIds]);
  useEffect(() => { writeLayout({ version: 1, root, expandedLeafId, pinnedSessionIds, dismissedSessionIds }); }, [root, expandedLeafId, pinnedSessionIds, dismissedSessionIds]);

  const order = useMemo(() => root ? leafIds(root) : [], [root]);
  const statuses = useMemo(() => new Map(order.flatMap(id => { const session = sessionMap.get(id); return session ? [[id, tileStatus(session, runtimes.get(id))] as const] : []; })), [order, sessionMap, runtimes]);
  const statusOf = useCallback((id: string) => statuses.get(id), [statuses]);
  const waitingIds = order.filter(id => { const status = statuses.get(id); return !!status && needsYou(status); });
  // Includes active pinned workers the visibility filter kept out of `live` —
  // otherwise a pinned worker can be visible and working while this reads 0.
  const workingCount = order.filter(id => { const session = sessionMap.get(id); return !!session && isActiveSession(session) && !waitingIds.includes(id); }).length;
  const legend = useMemo(() => {
    const counts = new Map<string, number>();
    for (const id of order) { const project = projectOf(id); if (project) counts.set(project, (counts.get(project) ?? 0) + 1); }
    return [...counts];
  }, [order, projectOf]);
  const highlightedProject = hoveredProject ?? (heldProject && legend.some(([name]) => name === heldProject) ? heldProject : null);

  function dropChat(id: string, fromSidebar: boolean, target?: string, edge?: DropEdge) {
    setDropOnCanvas(false);
    if (!sessionMap.has(id) || (!fromSidebar && !ids.includes(id))) return;
    const next = root && target && edge
      ? moveLeaf(root, id, target, edge === "left" || edge === "right" ? "horizontal" : "vertical", edge === "left" || edge === "top")
      : insertLeaf(root, id, root ? leafIds(root).filter(other => projectOf(other) && projectOf(other) === projectOf(id)) : []);
    setStored({
      version: 1, root: next, expandedLeafId: null,
      pinnedSessionIds: fromSidebar ? [...new Set([...pinnedSessionIds, id])] : pinnedSessionIds,
      // Bringing a chat back in by hand is what un-closes it.
      dismissedSessionIds: dismissedSessionIds.filter(dismissed => dismissed !== id),
    });
  }

  // each project's tiles together in an even grid; nothing else rearranges the board.
  function arrange() {
    setStored(prev => ({ ...prev, root: arrangeLeaves(groupedOrder(order, projectOf)), expandedLeafId: null }));
  }

  // cycles through tiles that need you, so repeated presses visit each once.
  function jumpToAttention() {
    if (!waitingIds.length) return;
    const target = waitingIds[attentionCursor.current % waitingIds.length];
    attentionCursor.current += 1;
    if (expandedLeafId && expandedLeafId !== target) setStored(prev => ({ ...prev, root, expandedLeafId: null }));
    requestAnimationFrame(() => {
      const tile = [...board.current?.querySelectorAll<HTMLElement>("[data-session-id]") ?? []].find(element => element.dataset.sessionId === target);
      tile?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
      tile?.querySelector<HTMLTextAreaElement>("textarea")?.focus();
    });
  }

  const actions: TileActions = {
    sessions: sessionMap, workspaces: workspaceMap, events, statusOf, projectOf, highlightedProject, activeSessionId, expandedLeafId, now, onFocusSession, onStopWorker, onForest,
    toggleExpanded: id => setStored(prev => ({ ...prev, root, expandedLeafId: prev.expandedLeafId === id ? null : id })),
    dropChat, pinnedSessionIds, drafts,
    setDraft: (id, draft) => setDrafts(prev => ({ ...prev, [id]: draft })),
    pin: id => setStored(prev => ({ ...prev, pinnedSessionIds: [...new Set([...prev.pinnedSessionIds, id])], dismissedSessionIds: prev.dismissedSessionIds.filter(dismissed => dismissed !== id) })),
    unpin: id => setStored(prev => ({ ...prev, pinnedSessionIds: prev.pinnedSessionIds.filter(pinned => pinned !== id) })),
    dismiss: id => setStored(prev => ({ ...prev, pinnedSessionIds: prev.pinnedSessionIds.filter(pinned => pinned !== id), dismissedSessionIds: [...new Set([...prev.dismissedSessionIds, id])] })),
    resize: (path, ratio) => { if (root) setStored(prev => ({ ...prev, root: resizeNode(root, path, ratio) })); },
  };

  return <main ref={board} aria-label="Mission Control"
    onDragOver={event => { if (isChatDrag(event.dataTransfer)) { event.preventDefault(); event.dataTransfer.dropEffect = event.dataTransfer.types.includes(SIDEBAR_CHAT_DRAG) ? "copy" : "move"; setDropOnCanvas(true); } }}
    onDragLeave={event => { if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setDropOnCanvas(false); }}
    onDrop={event => {
      setDropOnCanvas(false);
      if (!isChatDrag(event.dataTransfer)) return;
      event.preventDefault();
      const source = readChatDrag(event.dataTransfer);
      dropChat(source.id, source.fromSidebar);
    }}
    className={cn("flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-sidebar", dropOnCanvas && !root && "ring-2 ring-inset ring-ring")}>
    {/* the window title already reads "Mission Control"; this bar is the board's state. */}
    <h1 className="sr-only">Mission Control</h1>
    {root && <div role="toolbar" aria-label="Board" className="flex h-10 shrink-0 items-center gap-3 border-b border-border bg-background px-3 text-xs">
      <div className="flex shrink-0 items-center gap-3" aria-live="polite">
        {waitingIds.length > 0 && <button type="button" onClick={jumpToAttention} title="Go to the next chat waiting for you"
          className="inline-flex items-center gap-1.5 rounded-full bg-primary px-2.5 py-1 font-medium text-primary-foreground transition-opacity hover:opacity-90 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring">
          <Hand size={12} aria-hidden="true" />{waitingIds.length} needs you
        </button>}
        <span className="inline-flex items-center gap-1.5 text-muted-foreground"><span aria-hidden="true" className="h-1.5 w-1.5 rounded-full bg-muted-foreground" />{workingCount} working</span>
        {pinnedSessionIds.length > 0 && <span className="inline-flex items-center gap-1 text-muted-foreground"><Pin size={11} aria-hidden="true" />{pinnedSessionIds.length} pinned</span>}
      </div>
      {legend.length > 1 && <div role="group" aria-label="Projects on the board" className="flex min-w-0 items-center gap-1 overflow-hidden border-l border-border pl-3" onMouseLeave={() => setHoveredProject(null)}>
        {legend.map(([name, count]) => <button key={name} type="button" aria-pressed={heldProject === name}
          onMouseEnter={() => setHoveredProject(name)} onFocus={() => setHoveredProject(name)} onBlur={() => setHoveredProject(null)}
          onClick={() => setHeldProject(prev => prev === name ? null : name)}
          title={heldProject === name ? `Stop highlighting ${name}` : `Highlight ${name}'s chats`}
          className={cn("inline-flex max-w-40 shrink-0 items-center gap-1.5 rounded-md px-2 py-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring", heldProject === name && "bg-accent text-foreground")}>
          <span className="truncate font-medium">{name}</span><span className="font-mono text-[10px] text-muted-foreground">{count}</span>
        </button>)}
      </div>}
      <button type="button" onClick={arrange} title="Group tiles by project into an even grid. Drag a header to move one tile; drag a chat in from the sidebar to add it."
        className="ml-auto inline-flex shrink-0 items-center gap-1.5 rounded-md px-2 py-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring">
        <LayoutGrid size={13} aria-hidden="true" />Arrange
      </button>
    </div>}
    {root ? <div className="min-h-0 min-w-0 flex-1 overflow-auto p-2"><div className="h-full" style={{ minWidth: size?.width, minHeight: size?.height }}>{expandedLeafId ? <Tile key={expandedLeafId} id={expandedLeafId} actions={actions} /> : <SplitTree node={root} actions={actions} />}</div></div>
      : <div role="status" className="flex flex-1 flex-col items-center justify-center gap-3 px-8 text-center">
        <LayoutGrid size={28} strokeWidth={1.2} className="text-muted-foreground" aria-hidden="true" />
        <h2 className="text-lg font-medium">Nothing running right now</h2>
        <p className="max-w-sm text-sm leading-relaxed text-muted-foreground">Chats and agents appear here automatically while they work or wait for you, grouped by project. Drag any chat from the sidebar to keep it here, even when idle.</p>
      </div>}
  </main>;
}
