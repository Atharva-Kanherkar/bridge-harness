import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type DragEvent, type KeyboardEvent, type ReactNode, type RefObject } from "react";
import { ArrowLeft, ArrowRight, ArrowUpRight, Hand, LayoutGrid, Maximize2, Minimize2, MessageSquarePlus, Pin, PinOff, Square, X } from "lucide-react";
import { bridgeApi } from "../api";
import type { WorkflowRun } from "../protocol/generated/protocol";
import { activeWorkflowRuns, WorkflowRunsStrip } from "./workflows/WorkflowInbox";
import { useShowWorkerChatsInMissionControl } from "../missionControlSettings";
import { cn } from "@/lib/utils";
import { MenuPanel, useMenuPanel } from "@/components/ui/menu-panel";
import type { AgentEvent, Project, Session, SessionForestSnapshot, Workspace, WorkerRuntimeRecord } from "../types";
import type { ApprovalDecision, InteractionResolutionResult, QuestionAction } from "../protocol/generated/protocol";
import { formatElapsed, harnessLabel } from "../utils";
import { AgentConversation } from "./AgentConversation";
import { ComposerPill } from "./ComposerPill";
import { HarnessMark } from "./harnessMarks";
import { workerStatus, type WorkerStatus, type WorkerTone } from "./workerStatus";
import { displayOrder, moveInOrder, moveToEnd, moveToFront, planBoard, readLayout, reconcileOrder, STAGE_LIMITS, writeLayout, type DropPlace, type MissionLayout, type StageLimit } from "./missionControl/layout";
import { isChatDrag, readChatDrag, SIDEBAR_CHAT_DRAG, TILE_DRAG } from "./missionControl/drag";
import { askText, displayTitle, latestAsk, projectLabel } from "./missionControl/identity";
import { PLACEHOLDER_LABELS } from "./sidebarChats";
import { createBackgroundWorkStore } from "../backgroundTasks";
import { fixPrompt } from "../workSummary";
import { useChatPullRequests } from "./ChatPullRequests";
import { ChatWorkShelf } from "./WorkShelf";

export type MissionControlProps = {
  sessions: Session[];
  workspaces: Workspace[];
  projects?: Project[];
  events: AgentEvent[];
  activeSessionId?: string;
  onFocusSession: (sessionId: string) => void;
  onStopWorker?: (childSessionId: string) => Promise<void>;
  onNewChat?: (workspaceId: string) => void;
  /** Open a chat on its Work pane, on this PR when the shelf was about exactly one. */
  onOpenWork?: (sessionId: string, number?: number) => void;
  /** Workflow runs, for the strip of active ones; opening one goes to its run view. */
  workflowRuns?: WorkflowRun[];
  workflowNames?: Map<string, string>;
  onOpenWorkflowRun?: (run: WorkflowRun) => void;
};

const ACTIVE_STATUSES = new Set<Session["status"]>(["working", "waiting", "starting", "resuming", "checkpointing"]);
const FOREST_DEBOUNCE_MS = 300;

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

// a waiting lifecycle is an approval or a question addressed to you. a worker's
// blocked or unreadable result goes back to its orchestrator, so counting it here
// would dilute the one signal the board has (and why `isWaiting` is too broad).
const needsYou = (status: WorkerStatus) => status.tone === "waiting";
const sentenceCase = (label: string) => label.charAt(0) + label.slice(1).toLowerCase();

type TileActions = {
  sessions: Map<string, Session>;
  workspaces: Map<string, Workspace>;
  events: AgentEvent[];
  statusOf: (id: string) => WorkerStatus | undefined;
  projectOf: (id: string) => string | null;
  highlightedProject: string | null;
  activeSessionId?: string;
  expandedId: string | null;
  dragging: string | null;
  now: number;
  onFocusSession: (sessionId: string) => void;
  onStopWorker?: (childSessionId: string) => Promise<void>;
  onOpenWork?: (sessionId: string, number?: number) => void;
  onForest: (sessionId: string, forest: SessionForestSnapshot) => void;
  toggleExpanded: (id: string) => void;
  dropChat: (id: string, fromSidebar: boolean, target?: string, place?: DropPlace) => void;
  setDragging: (id: string | null) => void;
  /** Moves a tile one place earlier (-1) or later (1) on the board; absent at that end. */
  move: (id: string, step: -1 | 1) => void;
  canMove: (id: string, step: -1 | 1) => boolean;
  pinnedSessionIds: string[];
  pin: (id: string) => void;
  unpin: (id: string) => void;
  dismiss: (id: string) => void;
  drafts: Record<string, string>;
  setDraft: (id: string, draft: string) => void;
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
  const [drop, setDrop] = useState<DropPlace>();
  useEffect(() => {
    const clear = () => setDrop(undefined);
    window.addEventListener("dragend", clear);
    window.addEventListener("drop", clear);
    return () => { window.removeEventListener("dragend", clear); window.removeEventListener("drop", clear); };
  }, []);
  const events = useMemo(() => actions.events.filter(event => event.sessionId === id), [actions.events, id]);
  const forest = useSessionForest(id, events, actions.onForest);
  const recordedAsk = useMemo(() => latestAsk(forest?.entries, events), [forest?.entries, events]);
  // what you just sent shows at once, and gives way the moment the stream records any ask.
  const [sent, setSent] = useState<{ text: string; over: string | null } | null>(null);
  const ask = sent && sent.over === recordedAsk ? sent.text : recordedAsk;
  // the tile's own background-work slot: the global store belongs to the open
  // chat, and two tiles sharing one slot would show each other's tasks.
  const [work] = useState(createBackgroundWorkStore);
  const prs = useChatPullRequests(id, session?.workspaceId ?? undefined);
  if (!session) return null;
  const status = actions.statusOf(id) ?? tileStatus(session);
  const ink = TONE_INK[status.tone];
  const attention = needsYou(status);
  const working = session.status === "working" || session.activeTurnId != null;
  const isWorker = !!session.parentSessionId;
  const expanded = actions.expandedId === id;
  const lifted = actions.dragging === id;
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
    const shown = askText(text);
    setSent(shown ? { text: shown, over: recordedAsk } : null);
    try { await bridgeApi.submitInput(session!.id, text); setDraft(""); } catch (cause) { setSent(null); setError(cause instanceof Error ? cause.message : String(cause)); } finally { setSending(false); }
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
    onDragOver={event => {
      if (!isChatDrag(event.dataTransfer)) return;
      event.preventDefault(); event.stopPropagation();
      event.dataTransfer.dropEffect = event.dataTransfer.types.includes(SIDEBAR_CHAT_DRAG) ? "copy" : "move";
      setDrop(lifted ? undefined : dropPlace(event.currentTarget.getBoundingClientRect(), event.clientX));
    }}
    onDragLeave={event => { if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setDrop(undefined); }}
    onDrop={(event: DragEvent) => {
      setDrop(undefined);
      const source = readChatDrag(event.dataTransfer);
      if (!source.id || !actions.sessions.has(source.id)) return;
      event.preventDefault();
      event.stopPropagation();
      actions.dropChat(source.id, source.fromSidebar, id, dropPlace(event.currentTarget.getBoundingClientRect(), event.clientX));
    }}
    className={cn("relative flex h-full min-h-0 min-w-0 flex-col overflow-hidden rounded-lg border bg-background transition-opacity duration-200",
      attention ? "border-foreground/45" : "border-border",
      focused && "ring-1 ring-ring/60",
      dimmed && "opacity-40",
      lifted && "opacity-50")}>
    <header draggable title="Drag to reorder. Drop on the left or right half of another tile." onDragStart={event => { event.dataTransfer.setData(TILE_DRAG, id); event.dataTransfer.effectAllowed = "move"; actions.setDragging(id); }}
      className="group/header flex shrink-0 cursor-grab select-none flex-col gap-0.5 border-b border-border bg-card py-1.5 pl-3 pr-1.5 active:cursor-grabbing">
      <div className="relative flex h-6 min-w-0 items-center gap-2">
        {project && <span title={workspace?.branch ? `${project} · ${workspace.branch}` : project} className="max-w-[40%] shrink-0 truncate rounded bg-accent px-1.5 py-px text-[11px] font-medium leading-4 text-muted-foreground">{project}</span>}
        <h2 className="min-w-0 flex-1 truncate text-[13px] font-medium text-foreground" title={stored}>{title}</h2>
        <span className={cn("flex shrink-0 items-center gap-1.5 text-[11px]", ink.text)} title={status.detail ?? undefined}>
          {pinned && <Pin size={11} aria-label="Pinned" className="text-muted-foreground" />}
          {attention
            ? <span className="inline-flex items-center gap-1 rounded-full bg-primary px-2 py-0.5 font-medium text-primary-foreground"><Hand size={11} aria-hidden="true" />{sentenceCase(status.label)}</span>
            : <><span aria-hidden="true" className={cn("h-1.5 w-1.5 rounded-full", ink.dot)} /><span className="font-medium">{sentenceCase(status.label)}</span></>}
          <span className="font-mono text-muted-foreground group-hover/header:hidden group-focus-within/header:hidden" title="Elapsed">{formatElapsed(session.startedAt, actions.now)}</span>
        </span>
        {/* on header hover or keyboard focus the actions take the elapsed time's
            place and the title gives up width; the status pill never hides, since
            a waiting tile is exactly the one you are about to act on. collapsed,
            the buttons are clipped rather than removed, so Tab still reaches them. */}
        <div className="flex w-0 shrink-0 items-center gap-0.5 overflow-hidden opacity-0 transition-opacity group-hover/header:w-auto group-hover/header:opacity-100 group-focus-within/header:w-auto group-focus-within/header:opacity-100">
          <IconButton title="Focus chat" onClick={() => actions.onFocusSession(id)}><ArrowUpRight size={13} /></IconButton>
          {actions.canMove(id, -1) && <IconButton title="Move earlier" onClick={() => actions.move(id, -1)}><ArrowLeft size={13} /></IconButton>}
          {actions.canMove(id, 1) && <IconButton title="Move later" onClick={() => actions.move(id, 1)}><ArrowRight size={13} /></IconButton>}
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
        activeProcessEntries={forest?.activeProcessEntries}
        entryWindow={forest?.entryWindow}
        activeLeafId={forest?.head?.activeEntryId}
        repositoryDivergence={forest?.repositoryDivergence.status}
        completion={forest?.completion}
        continuationFidelity={session.continuationFidelity}
        working={working}
        onResolve={resolve}
        onAnswerQuestion={answer}
        onOpenSession={actions.onFocusSession}
        onInterrupt={interrupt}
        stopping={stopping}
        preview={false}
        density="compact"
        onBackgroundWork={work.publish}
      />
    </div>
    <div className="shrink-0 border-t border-border">
      {error && <p role="alert" className="px-3 pt-2 text-[11px] text-destructive">{error}</p>}
      <ComposerPill layout="inline" value={draft} onChange={setDraft} onSubmit={() => { void submit(); }} onKeyDown={onKeyDown}
        placeholder={working ? "Steer this chat…" : "Reply…"} working={working} activeAction="steer" disabled={sending} onStop={() => { void interrupt(); }} stopping={stopping}
        header={<ChatWorkShelf sessionId={id} store={work} harness={session.harness} prs={prs.prs}
          onOpen={number => (actions.onOpenWork ? actions.onOpenWork(id, number) : actions.onFocusSession(id))}
          onFix={pr => setDraft(fixPrompt(pr))} />} />
    </div>
    {/* where the dragged chat will land; nothing splits or resizes. */}
    {drop && <div aria-hidden="true" data-drop-place={drop} className={cn("pointer-events-none absolute inset-y-1 z-10 w-1 rounded-full bg-ring", drop === "before" ? "left-0.5" : "right-0.5")} />}
  </section>;
}

// left half lands before the tile, right half after it.
function dropPlace(rect: { left: number; width: number }, clientX: number): DropPlace {
  return rect.width && clientX - rect.left >= rect.width / 2 ? "after" : "before";
}

// the board's real size, so the tile count follows the window, the sidebar and zoom.
function useMeasuredSize(ref: RefObject<HTMLElement | null>) {
  const [size, setSize] = useState({ width: 0, height: 0 });
  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    const update = (width: number, height: number) => setSize(prev => prev.width === width && prev.height === height ? prev : { width, height });
    const rect = element.getBoundingClientRect();
    update(rect.width, rect.height);
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(entries => { const box = entries[0]?.contentRect; if (box) update(box.width, box.height); });
    observer.observe(element);
    return () => observer.disconnect();
  }, [ref]);
  return size;
}

type QueueCardProps = { id: string; actions: TileActions; strip: boolean; onPromote: (id: string) => void };

// a chat waiting off stage: who, what, and whether it needs you, in one line or a chip.
function QueueCard({ id, actions, strip, onPromote }: QueueCardProps) {
  const session = actions.sessions.get(id);
  const events = useMemo(() => actions.events.filter(event => event.sessionId === id), [actions.events, id]);
  if (!session) return null;
  const status = actions.statusOf(id) ?? tileStatus(session);
  const attention = needsYou(status);
  const ink = TONE_INK[status.tone];
  const project = actions.projectOf(id);
  const ask = latestAsk(undefined, events);
  const stored = session.title?.trim() || session.label;
  const unnamed = !session.title?.trim() && PLACEHOLDER_LABELS.has(session.label.trim().toLowerCase());
  const title = unnamed && ask ? ask : displayTitle(stored, project);
  const activity = status.detail ?? ask ?? [harnessLabel(session.harness), session.model].filter(Boolean).join(" · ");
  const dimmed = !!actions.highlightedProject && actions.highlightedProject !== project;
  const pill = attention
    ? <span className="inline-flex shrink-0 items-center gap-1 rounded-full bg-primary px-1.5 py-px text-[10px] font-medium text-primary-foreground"><Hand size={10} aria-hidden="true" />{sentenceCase(status.label)}</span>
    : <span className={cn("inline-flex shrink-0 items-center gap-1 text-[10px]", ink.text)}><span aria-hidden="true" className={cn("h-1.5 w-1.5 rounded-full", ink.dot)} />{sentenceCase(status.label)}</span>;
  return <button type="button" data-queue-id={id} data-attention={attention || undefined} draggable
    title={`Bring ${title} onto the stage`}
    onClick={() => onPromote(id)}
    onDragStart={event => { event.dataTransfer.setData(TILE_DRAG, id); event.dataTransfer.effectAllowed = "move"; actions.setDragging(id); }}
    className={cn("min-w-0 cursor-grab rounded-md border bg-background text-left transition-[opacity,background-color] hover:bg-accent focus-visible:outline-2 focus-visible:outline-ring active:cursor-grabbing",
      attention ? "border-foreground/45" : "border-border",
      dimmed && "opacity-40",
      actions.dragging === id && "opacity-50",
      strip ? "flex h-full max-w-56 shrink-0 items-center gap-1.5 px-2" : "flex w-full flex-col gap-1 px-2.5 py-2")}>
    {strip ? <>
      <HarnessMark harness={session.harness} size={11} />
      <span className="min-w-0 truncate text-[12px] font-medium text-foreground">{title}</span>
      {pill}
    </> : <>
      <span className="flex min-w-0 items-center gap-1.5">
        {project && <span className="max-w-[45%] shrink-0 truncate rounded bg-accent px-1.5 py-px text-[10px] font-medium leading-4 text-muted-foreground">{project}</span>}
        <span className="min-w-0 flex-1 truncate text-[12px] font-medium text-foreground">{title}</span>
      </span>
      <span className="flex min-w-0 items-center gap-1.5 text-[11px] text-muted-foreground">
        <HarnessMark harness={session.harness} size={11} />
        <span className="min-w-0 flex-1 truncate">{activity}</span>
        {pill}
      </span>
    </>}
  </button>;
}

export function MissionControl({ sessions, workspaces, projects = [], events, activeSessionId, onFocusSession, onStopWorker, onNewChat, onOpenWork, workflowRuns = [], workflowNames, onOpenWorkflowRun }: MissionControlProps) {
  const [forests, setForests] = useState<Record<string, SessionForestSnapshot>>({});
  const [stored, setStored] = useState(() => readLayout());
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [dropOnCanvas, setDropOnCanvas] = useState(false);
  const [dragging, setDragging] = useState<string | null>(null);
  const [hoveredProject, setHoveredProject] = useState<string | null>(null);
  const [heldProject, setHeldProject] = useState<string | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const newChatMenu = useMenuPanel<HTMLButtonElement>({ width: 220, height: 280 });
  const board = useRef<HTMLElement>(null);
  const area = useRef<HTMLDivElement>(null);
  const size = useMeasuredSize(area);
  const lastFocusedTile = useRef<string | null>(null);
  const [jumpTarget, setJumpTarget] = useState<string | null>(null);
  useEffect(() => { const timer = setInterval(() => setNow(Date.now()), 30_000); return () => clearInterval(timer); }, []);
  useEffect(() => {
    const clear = () => { setDropOnCanvas(false); setDragging(null); };
    window.addEventListener("dragend", clear);
    window.addEventListener("drop", clear);
    return () => { window.removeEventListener("dragend", clear); window.removeEventListener("drop", clear); };
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
  const isActive = useCallback((id: string) => { const session = sessionMap.get(id); return !!session && isActiveSession(session); }, [sessionMap]);
  // a closed chat only needs remembering while it would otherwise come back by
  // itself, so the list is bounded by what is running right now.
  const dismissedSessionIds = useMemo(() => stored.dismissedSessionIds.filter(isActive), [stored.dismissedSessionIds, isActive]);
  const visible = useCallback((id: string) => { const session = sessionMap.get(id); return !!session && (showWorkerChats || !session.parentSessionId); }, [sessionMap, showWorkerChats]);
  const live = useMemo(
    () => sessions.filter(session => isActiveSession(session) && visible(session.id) && !dismissedSessionIds.includes(session.id)),
    [sessions, visible, dismissedSessionIds],
  );
  const pinnedSessionIds = useMemo(() => stored.pinnedSessionIds.filter(id => sessionMap.has(id) && !dismissedSessionIds.includes(id)), [stored.pinnedSessionIds, sessionMap, dismissedSessionIds]);
  const incoming = useMemo(() => [...new Set([...live.map(session => session.id), ...pinnedSessionIds])], [live, pinnedSessionIds]);
  // chats already on the board keep their place; a finished one stays, showing
  // that it is done, until you close it or a newly active chat takes its slot.
  const order = useMemo(() => reconcileOrder(stored.order, {
    keep: id => sessionMap.has(id) && !dismissedSessionIds.includes(id) && (pinnedSessionIds.includes(id) || visible(id)),
    incoming,
    replaceable: id => !isActive(id) && !pinnedSessionIds.includes(id),
  }), [stored.order, sessionMap, dismissedSessionIds, pinnedSessionIds, visible, incoming, isActive]);
  const orderKey = order.join("\u0000");
  const expandedId = stored.expandedId && order.includes(stored.expandedId) ? stored.expandedId : null;
  const layout: MissionLayout = useMemo(() => ({ version: 2, order, pinnedSessionIds, dismissedSessionIds, expandedId, stageLimit: stored.stageLimit }),
    [orderKey, pinnedSessionIds.join("\u0000"), dismissedSessionIds.join("\u0000"), expandedId, stored.stageLimit]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => { setStored(prev => sameLayout(prev, layout) ? prev : layout); writeLayout(layout); }, [layout]);

  const statuses = useMemo(() => new Map(order.flatMap(id => { const session = sessionMap.get(id); return session ? [[id, tileStatus(session, runtimes.get(id))] as const] : []; })), [order, sessionMap, runtimes]);
  const statusOf = useCallback((id: string) => statuses.get(id), [statuses]);
  const waits = useCallback((id: string) => { const status = statuses.get(id); return !!status && needsYou(status); }, [statuses]);
  // what needs you leads, so it is on the stage whenever one tile fits.
  const shownOrder = useMemo(() => displayOrder(order, waits), [order, waits]);
  const waitingIds = shownOrder.filter(waits);
  // Includes active pinned workers the visibility filter kept out of `live` —
  // otherwise a pinned worker can be visible and working while this reads 0.
  const workingCount = order.filter(id => isActive(id) && !waitingIds.includes(id)).length;
  const legend = useMemo(() => {
    const counts = new Map<string, number>();
    for (const id of order) { const project = projectOf(id); if (project) counts.set(project, (counts.get(project) ?? 0) + 1); }
    return [...counts];
  }, [order, projectOf]);
  const highlightedProject = hoveredProject ?? (heldProject && legend.some(([name]) => name === heldProject) ? heldProject : null);

  const arranged = expandedId ? [expandedId, ...shownOrder.filter(id => id !== expandedId)] : shownOrder;
  const plan = planBoard(size.width, size.height, arranged.length, expandedId ? 1 : stored.stageLimit);
  const stageIds = arranged.slice(0, plan.shown);
  const queueIds = arranged.slice(plan.shown);

  const update = (change: (prev: MissionLayout) => Partial<MissionLayout>) => setStored(prev => ({ ...prev, order, pinnedSessionIds, dismissedSessionIds, expandedId, ...change({ ...prev, order, pinnedSessionIds, dismissedSessionIds, expandedId }) }));

  function dropChat(id: string, fromSidebar: boolean, target?: string, place: DropPlace = "before") {
    setDropOnCanvas(false);
    setDragging(null);
    if (!sessionMap.has(id) || (!fromSidebar && !order.includes(id)) || (!fromSidebar && target === id)) return;
    // on a tile: that slot. on open board: a tile goes to the end, a sidebar chat
    // already on the board stays put.
    const placed = (prev: readonly string[]) => target ? moveInOrder(prev, id, target, place)
      : !fromSidebar ? moveToEnd(prev, id)
      : prev.includes(id) ? [...prev] : [...prev, id];
    update(prev => ({
      order: placed(prev.order),
      pinnedSessionIds: fromSidebar ? [...new Set([...prev.pinnedSessionIds, id])] : prev.pinnedSessionIds,
      // Bringing a chat back in by hand is what un-closes it.
      dismissedSessionIds: prev.dismissedSessionIds.filter(dismissed => dismissed !== id),
    }));
  }

  // steps through what you see, so a press always moves the tile one visible place.
  function neighbour(id: string, step: -1 | 1) { return shownOrder[shownOrder.indexOf(id) + step]; }
  function move(id: string, step: -1 | 1) {
    const other = neighbour(id, step);
    if (other) update(prev => ({ order: moveInOrder(prev.order, id, other, step < 0 ? "before" : "after") }));
  }

  // the next waiting tile after the one you were last in, in board order,
  // wrapping. anchored to a tile rather than a counter, so tiles resolving
  // between presses never make it skip or repeat.
  function jumpToAttention() {
    if (!waitingIds.length) return;
    const from = lastFocusedTile.current ? shownOrder.indexOf(lastFocusedTile.current) : -1;
    const target = waitingIds.find(id => shownOrder.indexOf(id) > from) ?? waitingIds[0];
    if (expandedId && expandedId !== target) update(() => ({ expandedId: null }));
    setJumpTarget(target);
  }
  // runs after the commit that restored the grid, so the target tile is mounted.
  useEffect(() => {
    if (!jumpTarget) return;
    const tile = [...board.current?.querySelectorAll<HTMLElement>("[data-session-id]") ?? []].find(element => element.dataset.sessionId === jumpTarget);
    tile?.querySelector<HTMLTextAreaElement>("textarea")?.focus();
    setJumpTarget(null);
  }, [jumpTarget]);

  const actions: TileActions = {
    sessions: sessionMap, workspaces: workspaceMap, events, statusOf, projectOf, highlightedProject, activeSessionId, expandedId, dragging, now, onFocusSession, onStopWorker, onOpenWork, onForest,
    toggleExpanded: id => update(prev => ({ expandedId: prev.expandedId === id ? null : id })),
    dropChat, setDragging, move, canMove: (id, step) => !expandedId && !!neighbour(id, step), pinnedSessionIds, drafts,
    setDraft: (id, draft) => setDrafts(prev => ({ ...prev, [id]: draft })),
    pin: id => update(prev => ({ pinnedSessionIds: [...new Set([...prev.pinnedSessionIds, id])], dismissedSessionIds: prev.dismissedSessionIds.filter(dismissed => dismissed !== id) })),
    unpin: id => update(prev => ({ pinnedSessionIds: prev.pinnedSessionIds.filter(pinned => pinned !== id) })),
    dismiss: id => update(prev => ({ order: prev.order.filter(other => other !== id), pinnedSessionIds: prev.pinnedSessionIds.filter(pinned => pinned !== id), dismissedSessionIds: isActive(id) ? [...new Set([...prev.dismissedSessionIds, id])] : prev.dismissedSessionIds })),
  };
  const promote = (id: string) => update(prev => ({ order: moveToFront(prev.order, id), expandedId: null }));
  const setLimit = (limit: StageLimit | null) => update(() => ({ stageLimit: limit, expandedId: null }));
  const hasBoard = order.length > 0;
  const showRuns = !!onOpenWorkflowRun && activeWorkflowRuns(workflowRuns).length > 0;
  const toolButton = "inline-flex shrink-0 items-center gap-1.5 rounded-md px-2 py-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring";

  const queue = queueIds.length > 0 && <div role="list" aria-label="More chats"
    className={cn("flex shrink-0 gap-2", plan.queue === "rail" ? "w-64 flex-col overflow-y-auto" : "h-9 overflow-x-auto")}>
    {queueIds.map(id => <div role="listitem" key={id} className={plan.queue === "rail" ? "" : "h-full"}><QueueCard id={id} actions={actions} strip={plan.queue !== "rail"} onPromote={promote} /></div>)}
  </div>;

  return <main ref={board} aria-label="Mission Control"
    onFocusCapture={event => { const tile = (event.target as HTMLElement).closest<HTMLElement>("[data-session-id]")?.dataset.sessionId; if (tile) lastFocusedTile.current = tile; }}
    onDragOver={event => { if (isChatDrag(event.dataTransfer)) { event.preventDefault(); event.dataTransfer.dropEffect = event.dataTransfer.types.includes(SIDEBAR_CHAT_DRAG) ? "copy" : "move"; setDropOnCanvas(true); } }}
    onDragLeave={event => { if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setDropOnCanvas(false); }}
    onDrop={event => {
      setDropOnCanvas(false);
      if (!isChatDrag(event.dataTransfer)) return;
      event.preventDefault();
      const source = readChatDrag(event.dataTransfer);
      dropChat(source.id, source.fromSidebar);
    }}
    className={cn("flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-sidebar", dropOnCanvas && !hasBoard && "ring-2 ring-inset ring-ring")}>
    {/* the window title already reads "Mission Control"; this bar is the board's state. */}
    <h1 className="sr-only">Mission Control</h1>
    {(hasBoard || showRuns) && <div role="toolbar" aria-label="Board" className="@container flex h-10 shrink-0 items-center gap-3 border-b border-border bg-background px-3 text-xs">
      {hasBoard && <div className="relative flex shrink-0 items-center gap-3">
        {/* announces only what needs you; the working count changes too often to speak. */}
        <span aria-live="polite" className="sr-only">{waitingIds.length > 0 ? `${waitingIds.length} needs you` : ""}</span>
        {waitingIds.length > 0 && <button type="button" onClick={jumpToAttention} title="Go to the next chat waiting for you"
          className="inline-flex items-center gap-1.5 rounded-full bg-primary px-2.5 py-1 font-medium text-primary-foreground transition-opacity hover:opacity-90 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring">
          <Hand size={12} aria-hidden="true" />{waitingIds.length} needs you
        </button>}
        <span className="inline-flex items-center gap-1.5 text-muted-foreground"><span aria-hidden="true" className="h-1.5 w-1.5 rounded-full bg-muted-foreground" />{workingCount} working</span>
        {pinnedSessionIds.length > 0 && <span className="inline-flex items-center gap-1 text-muted-foreground"><Pin size={11} aria-hidden="true" />{pinnedSessionIds.length} pinned</span>}
      </div>}
      {showRuns && <div className={cn("flex min-w-0 items-center", hasBoard && "border-l border-border pl-3")}><WorkflowRunsStrip variant="chips" runs={workflowRuns} names={workflowNames ?? new Map()} onOpenRun={onOpenWorkflowRun!}/></div>}
      {legend.length > 1 && <div role="group" aria-label="Projects on the board" className="flex min-w-0 items-center gap-1 overflow-hidden border-l border-border pl-3" onMouseLeave={() => setHoveredProject(null)}>
        {legend.map(([name, count]) => <button key={name} type="button" aria-pressed={heldProject === name}
          onMouseEnter={() => setHoveredProject(name)} onFocus={() => setHoveredProject(name)} onBlur={() => setHoveredProject(null)}
          onClick={() => setHeldProject(prev => prev === name ? null : name)}
          title={heldProject === name ? `Stop highlighting ${name}` : `Highlight ${name}'s chats`}
          className={cn("inline-flex max-w-40 shrink-0 items-center gap-1.5 rounded-md px-2 py-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring", heldProject === name && "bg-accent text-foreground")}>
          <span className="truncate font-medium">{name}</span><span className="font-mono text-[10px] text-muted-foreground">{count}</span>
        </button>)}
      </div>}
      {/* below 600px only one tile fits anyway, so the control would only crowd the bar. */}
      {hasBoard && <div role="group" aria-label="Tiles on the stage" className="ml-auto flex shrink-0 items-center rounded-md bg-muted p-0.5 @max-[600px]:hidden">
        {([null, ...STAGE_LIMITS] as const).map(limit => <button key={limit ?? "auto"} type="button" aria-pressed={stored.stageLimit === limit}
          title={limit == null ? "Fit as many tiles as the board holds" : `Show at most ${limit} ${limit === 1 ? "tile" : "tiles"}`}
          onClick={() => setLimit(limit)}
          className={cn("h-6 min-w-6 rounded px-1.5 font-medium text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring", stored.stageLimit === limit && "bg-background text-foreground shadow-xs")}>
          {limit ?? "Auto"}
        </button>)}
      </div>}
      {onNewChat && workspaces.length > 0 && <>
        <button ref={newChatMenu.triggerRef} type="button" aria-haspopup="menu" aria-expanded={newChatMenu.open}
          onClick={() => workspaces.length === 1 ? onNewChat(workspaces[0].id) : newChatMenu.toggle()} title="Start a new chat"
          className={cn(toolButton, "@max-[600px]:ml-auto", !hasBoard && "ml-auto")}>
          <MessageSquarePlus size={13} aria-hidden="true" /><span className="@max-[480px]:sr-only">New chat</span>
        </button>
        <MenuPanel controller={newChatMenu} label="Choose a project">
          <p className="px-2 py-1 text-[11px] font-medium text-muted-foreground">New chat in…</p>
          {workspaces.map(workspace => <button key={workspace.id} type="button" role="menuitem"
            onClick={() => { newChatMenu.close(); onNewChat(workspace.id); }}
            className="flex h-7 w-full items-center rounded-md px-2 text-left text-[13px] transition-colors hover:bg-accent">
            <span className="min-w-0 flex-1 truncate">{projectLabel({} as Session, workspace, projects) ?? workspace.title}</span>
          </button>)}
        </MenuPanel>
      </>}
    </div>}
    <div className="flex min-h-0 min-w-0 flex-1 p-2">
      <div ref={area} data-board-area className={cn("flex min-h-0 min-w-0 flex-1 gap-2 overflow-hidden", plan.queue === "strip" ? "flex-col" : "flex-row")}>
        {hasBoard ? <>
          {plan.queue === "strip" && queue}
          {/* the column and row counts are measured at runtime, so the track template is the one inline value. */}
          <div data-stage data-cols={plan.cols} data-rows={plan.rows} className="grid min-h-0 min-w-0 flex-1 gap-2 overflow-hidden"
            style={{ gridTemplateColumns: `repeat(${plan.cols}, minmax(0, 1fr))`, gridTemplateRows: `repeat(${plan.rows}, minmax(0, 1fr))` }}>
            {stageIds.map(id => <Tile key={id} id={id} actions={actions} />)}
          </div>
          {plan.queue === "rail" && queue}
        </> : <div role="status" className="flex flex-1 flex-col items-center justify-center gap-3 px-8 text-center">
          <LayoutGrid size={28} strokeWidth={1.2} className="text-muted-foreground" aria-hidden="true" />
          <h2 className="text-lg font-medium">Nothing running right now</h2>
          <p className="max-w-sm text-sm leading-relaxed text-muted-foreground">Chats and agents appear here automatically while they work or wait for you. Drag any chat from the sidebar to keep it here, even when idle.</p>
        </div>}
      </div>
    </div>
  </main>;
}

const sameList = (a: readonly string[], b: readonly string[]) => a.length === b.length && a.every((id, index) => id === b[index]);
function sameLayout(a: MissionLayout, b: MissionLayout) {
  return sameList(a.order, b.order) && sameList(a.pinnedSessionIds, b.pinnedSessionIds) && sameList(a.dismissedSessionIds, b.dismissedSessionIds) && a.expandedId === b.expandedId && a.stageLimit === b.stageLimit;
}
