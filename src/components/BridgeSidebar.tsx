import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import {  Archive,
 BarChart3, TerminalSquare, ChartNoAxesColumn, ChevronRight, Folder, FolderGit2, FolderPlus, GitBranch, Home, Pin, Plus, RotateCw, Search, Settings2, SquarePen, Store, type LucideIcon, LayoutGrid } from "lucide-react";
import { WindowNavButtons } from "./WindowNavButtons";
import { HarnessMark } from "./harnessMarks";
import type { Session, SessionStatus, Workspace } from "../types";
import { chordLabel, type CommandId } from "../keymap";
import { cn } from "@/lib/utils";
import { MOTION_DURATION, useMotionTransition } from "../motion";
import { harnessLabel } from "../utils";
import { SidebarFilterMenu } from "./SidebarFilterMenu";
import {
  GROUP_ROW_CAP,
  NO_PROJECT_GROUP_KEY,
  agentOptions,
  chatListTime,
  chatName,
  chatTimestamp,
  filterChats,
  groupChats,
  readChatView,
  statusBucket,
  writeChatView,
  type ChatView,
} from "./sidebarChats";

const MIN_WIDTH = 200;
const MAX_WIDTH = 400;
const DEFAULT_WIDTH = 248;
const WIDTH_KEY = "bridge.sidebar.width";
const COLLAPSED_KEY = "bridge.sidebar.collapsed";

function readWidth(): number {
  const raw = localStorage.getItem(WIDTH_KEY);
  const value = raw ? Number(raw) : DEFAULT_WIDTH;
  if (!Number.isFinite(value)) return DEFAULT_WIDTH;
  return Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, value));
}

// Working, waiting, and failure states pair a status dot with readable text.
function rowStatus(status: SessionStatus): { label: string; dot: string } | null {
  const bucket = statusBucket(status);
  if (bucket === "active") return { label: "working", dot: "bg-success" };
  if (bucket === "waiting") return { label: "needs you", dot: "bg-warning" };
  if (bucket === "failed") return { label: "failed", dot: "bg-destructive" };
  return null;
}

function ChatRow({
  chat,
  active,
  indented,
  time,
  onClick,
  onArchive,
}: {
  chat: Session;
  active: boolean;
  indented: boolean;
  time: string | null;
  onClick: () => void;
  onArchive?: () => void;
}) {
  const name = chatName(chat);
  const detail = `${name} — ${harnessLabel(chat.harness)}${chat.model ? ` · ${chat.model}` : ""}`;
  const status = rowStatus(chat.status);
  return (
    <span className={cn(
      "group/row relative flex items-center rounded-[7px] transition-colors",
      active ? "bg-selection text-selection-foreground" : "hover:bg-accent",
    )}>
    <button
      type="button"
      onClick={onClick}
      title={detail}
      aria-current={active ? "page" : undefined}
      className={cn(
        "flex h-11 min-w-0 flex-1 items-center gap-2 rounded-[7px] pr-2 text-left font-sans transition-colors active:scale-[0.99]",
        indented ? "pl-7" : "pl-2",
      )}
    >
      <span className="flex min-w-0 flex-1 flex-col justify-center gap-0.5">
        <span className={cn("truncate text-[13px] leading-4 tracking-[-0.008em] text-foreground", active && "font-medium")}>{name}</span>
        <span className="flex min-w-0 items-center gap-1.5 truncate text-[11px] leading-3.5 tracking-[-0.004em] text-muted-foreground">
          {status && <><span className={cn("h-1.5 w-1.5 shrink-0 rounded-full", status.dot)} aria-hidden="true" /><span>{status.label}</span><span aria-hidden="true">·</span></>}
          {time && <span className="shrink-0 tabular-nums text-faint">{time}</span>}
        </span>
      </span>
      {/* Muted at rest: a column of full-tint marks is the loudest thing in the
          rail and the chrome stays achromatic. The active row earns its tint. */}
      <HarnessMark harness={chat.harness} size={13} className={cn("shrink-0", !active && "text-muted-foreground")} />
    </button>
    {/* Revealed on hover or keyboard focus, never at rest: a column of archive
        buttons down the rail would compete with the chat names for attention,
        and this is a rarely-wanted action. Achromatic like the rest of the
        chrome — it is not a warning, it is filing something away. */}
    {onArchive && (
      <button
        type="button"
        onClick={event => { event.stopPropagation(); onArchive(); }}
        title={`Archive ${name}`}
        aria-label={`Archive ${name}`}
        className="mr-1 grid size-7 shrink-0 place-items-center rounded-md text-muted-foreground opacity-0 transition-opacity hover:bg-accent hover:text-foreground focus-visible:opacity-100 group-hover/row:opacity-100"
      >
        <Archive size={12} strokeWidth={1.7} aria-hidden="true" />
      </button>
    )}
    </span>
  );
}

function SectionLabel({ children, action }: { children: React.ReactNode; action?: React.ReactNode }) {
  return (
    <div className="flex h-7 items-center gap-1.5 px-2">
      <Folder size={13} strokeWidth={1.6} className="shrink-0 text-muted-foreground" aria-hidden="true" />
      <span className="text-[12px] font-medium tracking-[-0.004em] text-muted-foreground">{children}</span>
      {action && <span className="ml-auto flex items-center gap-0.5">{action}</span>}
    </div>
  );
}

function GroupLabel({
  label,
  count,
  folded,
  active,
  icon: Icon,
  onToggle,
  action,
}: {
  label: string;
  count: number;
  folded: boolean;
  active: boolean;
  icon?: LucideIcon;
  onToggle: () => void;
  action?: React.ReactNode;
}) {
  return (
    <div
      className={cn(
        "group flex h-7 w-full items-center gap-2 rounded-md pl-2 pr-1 text-left text-[13px] tracking-[-0.008em] text-foreground/90 transition-colors",
        active ? "bg-accent font-medium text-foreground" : "hover:bg-accent/70 hover:text-foreground",
      )}
    >
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={!folded}
        title={folded ? `Show ${label}` : `Hide ${label}`}
        className="flex h-full min-w-0 flex-1 items-center gap-2 text-left"
      >
        <ChevronRight
          size={11}
          strokeWidth={2}
          aria-hidden="true"
          className={cn("shrink-0 text-muted-foreground/60 transition-transform", !folded && "rotate-90")}
        />
        {Icon && <Icon size={13} strokeWidth={1.5} className="shrink-0 text-muted-foreground" aria-hidden="true" />}
        <span className="min-w-0 truncate text-[12px] text-muted-foreground">{label}</span>
      </button>
      <span className="ml-auto shrink-0 text-[11px] tabular-nums text-muted-foreground/60">{count}</span>
      {action}
    </div>
  );
}

function RailIconButton({ label, onClick, children }: { label: string; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={label}
      aria-label={label}
      className="inline-flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
    >
      {children}
    </button>
  );
}

// The bottom rail is achromatic on purpose: settings, source control, usage,
// and refresh sit at rest in muted ink and only warm to the foreground on hover
// or when their screen is the current one.
function RailBottomButton({
  label,
  onClick,
  active = false,
  children,
}: {
  label: string;
  onClick: () => void;
  active?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={label}
      aria-label={label}
      aria-current={active ? "page" : undefined}
      className={cn(
        "inline-flex h-8 w-8 items-center justify-center rounded-[7px] transition-colors",
        active ? "bg-card text-foreground" : "text-muted-foreground hover:bg-card hover:text-foreground",
      )}
    >
      {children}
    </button>
  );
}

function ActionRow({
  icon: Icon,
  label,
  onClick,
  active = false,
  disabled = false,
  chord,
}: {
  icon: LucideIcon;
  label: string;
  onClick: () => void;
  active?: boolean;
  disabled?: boolean;
  /** Advertise the row's binding in its tooltip, read from the keymap so the
   *  two cannot disagree. The accessible name stays the plain label. */
  chord?: CommandId;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      title={chord ? `${label}  ${chordLabel(chord)}` : label}
      aria-label={label}
      aria-current={active ? "page" : undefined}
      className={cn(
        "flex h-8 w-full items-center gap-2.5 rounded-lg px-2 text-[13px] tracking-[-0.008em] transition-colors",
        active ? "bg-selection font-medium text-selection-foreground" : "text-foreground/85 hover:bg-accent hover:text-foreground",
        disabled && "cursor-default opacity-50 hover:bg-transparent hover:text-foreground/85",
      )}
    >
      <Icon size={15} strokeWidth={1.5} className="shrink-0 text-muted-foreground" aria-hidden="true" />
      {label}
    </button>
  );
}

export type BridgeSidebarProps = {
  /** Every top-level session. The rail derives both the project tree and the
   * history from this one list, so a chat cannot be visible in one and missing
   * from the other. */
  chats: Session[];
  /** Only for `Group by → Project` labels; the tree itself lives on the projects
   * screen now. */
  workspaces: Workspace[];
  activeSessionId?: string;
  projectsActive: boolean;
  memoryActive?: boolean;
  marketplaceActive: boolean;
  usageActive?: boolean;
  agentFleetActive: boolean;
  missionControlActive: boolean;
  workActive?: boolean;
  settingsActive: boolean;
  accountName: string;
  newChatBusy?: boolean;
  /** Drawer state below the sm breakpoint, where the rail is off-canvas. */
  mobileOpen?: boolean;
  onCloseMobile?: () => void;
  onOpenNewChat: () => void;
  /** Per-project "+" on a project group header, grouped-by-project only —
   * skips the picker step since the group already names the workspace. */
  onNewChatInProject?: (workspaceId: string) => void;
  onOpenProjects: () => void;
  onOpenMarketplace: () => void;
  onOpenAgentFleet: () => void;
  onOpenMissionControl: () => void;
  onOpenWorkBoard: () => void;
  /** Account memory. Not workspace-gated: a plain chat reaches it identically. */
  onOpenMemory: () => void;
  /** Token and cost usage across harnesses. */
  onOpenUsage?: () => void;
  onOpenSettings: () => void;
  onOpenSession: (id: string) => void;
  /** Absent when the host cannot archive — the row then shows no action. */
  onArchiveChat?: (chat: Session) => void;
  /** Hidden, not shrunk: from `sm` up a collapsed rail gives back every pixel
   * and leaves nothing on screen. When set, the rail uses this state instead of
   * its own. */
  collapsed?: boolean;
  onCollapsedChange?: (collapsed: boolean) => void;
  /** Panel + history chevrons. Hidden when those controls live on the title bar. */
  showWindowNav?: boolean;
  canBack?: boolean;
  canForward?: boolean;
  onBack?: () => void;
  onForward?: () => void;
};

export function BridgeSidebar({
  chats,
  workspaces,
  activeSessionId,
  projectsActive,
  memoryActive = false,
  marketplaceActive,
  usageActive = false,
  agentFleetActive,
  missionControlActive,
  settingsActive,
  accountName,
  newChatBusy = false,
  mobileOpen = false,
  onCloseMobile,
  onOpenNewChat,
  onNewChatInProject,
  onOpenProjects,
  onOpenMarketplace,
  onOpenAgentFleet,
  onOpenMissionControl,
  onOpenMemory,
  onOpenUsage,
  onOpenSettings,
  onOpenSession,
  onArchiveChat,
  collapsed: collapsedProp,
  onCollapsedChange,
  showWindowNav = true,
  canBack = false,
  canForward = false,
  onBack,
  onForward,
}: BridgeSidebarProps) {
  const [width, setWidth] = useState(readWidth);
  const [internalCollapsed, setInternalCollapsed] = useState(() => localStorage.getItem(COLLAPSED_KEY) === "1");
  const collapsed = collapsedProp ?? internalCollapsed;
  const setCollapsed = useCallback((next: boolean | ((value: boolean) => boolean)) => {
    const resolved = typeof next === "function" ? next(collapsed) : next;
    if (onCollapsedChange) onCollapsedChange(resolved);
    else setInternalCollapsed(resolved);
  }, [collapsed, onCollapsedChange]);
  const [resizing, setResizing] = useState(false);
  const [view, setView] = useState<ChatView>(readChatView);
  const [searchOpen, setSearchOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [shownInFull, setShownInFull] = useState<Set<string>>(new Set());
  const [foldedGroups, setFoldedGroups] = useState<Set<string>>(new Set());
  const [now, setNow] = useState(() => Date.now());
  const widthRef = useRef(width);
  const resizeHandleRef = useRef<HTMLDivElement>(null);
  widthRef.current = width;

  // A hidden rail keeps the width it was last dragged to, so reopening lands
  // where the user left it rather than back at the default.
  useEffect(() => {
    if (!collapsed) localStorage.setItem(WIDTH_KEY, String(width));
  }, [width, collapsed]);

  // Day headers are relative, so a rail left open past midnight would keep
  // calling yesterday's chats "Today" until the list next changed.
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 60_000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    localStorage.setItem(COLLAPSED_KEY, collapsed ? "1" : "0");
  }, [collapsed]);

  const changeView = useCallback((next: ChatView) => {
    setView(next);
    writeChatView(next);
    // A cap and a fold both belong to a group key, and the keys change meaning
    // with the grouping.
    setShownInFull(new Set());
    setFoldedGroups(new Set());
  }, []);

  const toggleCollapsed = useCallback(() => setCollapsed(value => !value), [setCollapsed]);

  const closeSearch = useCallback(() => {
    setSearchOpen(false);
    setQuery("");
  }, []);

  // While filtering, the field takes the wide slot and New Chat stays
  // available beside it. Escape or an empty blur restores the main action.
  const openSearch = useCallback(() => setSearchOpen(true), []);

  const stopResize = useCallback((pointerId?: number) => {
    setResizing(false);
    document.body.style.cursor = "";
    document.body.style.userSelect = "";
    if (pointerId !== undefined && resizeHandleRef.current?.hasPointerCapture(pointerId)) {
      resizeHandleRef.current.releasePointerCapture(pointerId);
    }
  }, []);

  const startResize = useCallback((event: React.PointerEvent<HTMLDivElement>) => {
    if (collapsed) return;
    event.preventDefault();
    event.stopPropagation();

    const handle = resizeHandleRef.current;
    if (!handle) return;

    const pointerId = event.pointerId;
    const startX = event.clientX;
    const startWidth = widthRef.current;

    handle.setPointerCapture(pointerId);
    setResizing(true);
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";

    const onMove = (moveEvent: PointerEvent) => {
      if (moveEvent.pointerId !== pointerId) return;
      const next = Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, startWidth + (moveEvent.clientX - startX)));
      setWidth(next);
    };

    const onEnd = (endEvent: PointerEvent) => {
      if (endEvent.pointerId !== pointerId) return;
      handle.removeEventListener("pointermove", onMove);
      handle.removeEventListener("pointerup", onEnd);
      handle.removeEventListener("pointercancel", onEnd);
      stopResize(pointerId);
    };

    handle.addEventListener("pointermove", onMove);
    handle.addEventListener("pointerup", onEnd);
    handle.addEventListener("pointercancel", onEnd);
  }, [collapsed, stopResize]);

  useEffect(() => () => stopResize(), [stopResize]);

  const needle = query.trim().toLowerCase();
  const searching = needle.length > 0;

  const toggleFold = useCallback((key: string) => {
    setFoldedGroups(current => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }, []);

  const agents = useMemo(() => agentOptions(chats), [chats]);
  const workspaceTitle = useMemo(() => {
    const titles = new Map(workspaces.map(workspace => [workspace.id, workspace.title]));
    return (id: string | null | undefined) => (id ? titles.get(id) : undefined);
  }, [workspaces]);
  const visible = useMemo(
    () => filterChats(chats, { query, status: view.status, agent: view.agent, workspaceTitle }),
    [chats, query, view.status, view.agent, workspaceTitle],
  );
  const groups = useMemo(
    () => groupChats(visible, { groupBy: view.groupBy, sortBy: view.sortBy, workspaces, now }),
    [visible, view.groupBy, view.sortBy, workspaces, now],
  );

  // Collapsing takes the rail off the screen entirely; the panel controls move
  // to the canvas' chrome row, which is the only way back in besides ⌘B. The
  // drawer below `sm` is a second, independent axis, so an open drawer still
  // shows the whole rail even while the desktop side is put away.
  const hidden = collapsed && !mobileOpen;
  const animateWidth = !resizing;
  const scrimTransition = useMotionTransition(MOTION_DURATION.overlay);

  return (
    <>
      {/* Below sm the rail is an off-canvas drawer, so narrow windows keep
          their navigation instead of losing it entirely. */}
      {/* The drawer itself keeps its Tailwind `transition-transform` slide — it
          stays mounted, so CSS can carry it both ways. The scrim is the half
          that unmounts, which is why it needs AnimatePresence to fade out at
          all instead of blinking away. */}
      <AnimatePresence>
        {mobileOpen && (
          <motion.button
            type="button"
            className="fixed inset-0 z-30 bg-scrim sm:hidden"
            onClick={onCloseMobile}
            aria-label="Close navigation"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={scrimTransition}
          />
        )}
      </AnimatePresence>
      <aside
        // `inert` is what makes "hidden" true rather than merely invisible:
        // nothing inside can be clicked, focused, or read out while the rail is
        // mid-wipe or fully away. React 18 has no typing for it, so it is
        // spread in as the plain attribute it is.
        {...(hidden ? { inert: "" } : {})}
        aria-hidden={hidden || undefined}
        className={cn(
          "z-40 flex shrink-0 flex-col font-sans antialiased",
          "fixed inset-y-0 left-0 w-[min(84vw,20rem)] transition-transform duration-300 ease-[cubic-bezier(0.22,1,0.36,1)]",
          mobileOpen ? "visible translate-x-0" : "invisible -translate-x-full",
          "sm:visible sm:relative sm:z-20 sm:w-(--sidebar-w) sm:translate-x-0",
          "u-vibrancy-sidebar bg-sidebar",
          // Hidden means hidden: no rail, no icons, not even the seam.
          hidden ? "overflow-hidden" : "border-r border-sidebar-border",
          animateWidth ? "sm:transition-[width] sm:duration-300 sm:ease-[cubic-bezier(0.22,1,0.36,1)]" : "sm:transition-none",
        )}
        style={{
          "--sidebar-w": `${collapsed ? 0 : width}px`,
          "--sidebar-panel-w": `${width}px`,
        } as React.CSSProperties}
      >
      {/* The panel keeps its open width while the aside animates to zero, so the
          rail wipes off the edge instead of reflowing every row on the way out. */}
      {/* The contents fade as the width goes, so the outgoing header does not
          sit beside the chrome row's incoming panel button for the whole wipe. */}
      <div className={cn(
        "flex w-full min-h-0 flex-1 flex-col sm:w-(--sidebar-panel-w) sm:transition-opacity sm:duration-200",
        hidden && "sm:opacity-0",
      )}>
      {showWindowNav && (
        <div className="flex h-11 shrink-0 items-center gap-0.5 u-traffic-inset pl-24 pr-1.5" data-tauri-drag-region="deep">
          <WindowNavButtons
            spread
            collapsed={collapsed}
            onToggleCollapsed={toggleCollapsed}
            canBack={canBack}
            canForward={canForward}
            onBack={onBack ?? (() => {})}
            onForward={onForward ?? (() => {})}
          />
        </div>
      )}
      <div className={cn("flex min-h-0 flex-1 flex-col overflow-hidden px-3 pb-3", showWindowNav ? "pt-1" : "pt-3")}>
        {/* New Chat leads the row; Search expands into the wide slot only
            while filtering, with a compact compose button beside the field. */}
        <div
          className="mb-3 flex shrink-0 items-center gap-1.5"
          onBlur={event => {
            // Focusing compose is part of its click. Keep it in place until
            // the click completes; only dismiss on focus leaving the row.
            if (!query.trim() && !event.currentTarget.contains(event.relatedTarget)) closeSearch();
          }}
        >
          {searchOpen && (
            <div className="relative h-8 min-w-0 flex-1">
              <Search size={14} strokeWidth={1.6} aria-hidden="true" className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-muted-foreground" />
              <input
                type="text"
                value={query}
                autoFocus
                onChange={event => setQuery(event.target.value)}
                onKeyDown={event => { if (event.key === "Escape") closeSearch(); }}
                placeholder="Filter chats and projects…"
                aria-label="Filter chats and projects"
                className="h-8 w-full rounded-[7px] border border-ring/50 bg-background pl-8 pr-2.5 text-[13px] text-foreground outline-none transition-colors placeholder:text-muted-foreground/60 focus:border-ring"
              />
            </div>
          )}
          <button
            type="button"
            onClick={() => { closeSearch(); onOpenNewChat(); }}
            disabled={newChatBusy}
            aria-label="New Chat"
            title={`New Chat  ${chordLabel("new-chat")}`}
            className={cn(
              "inline-flex h-8 items-center gap-2 rounded-[7px] border border-border-card bg-card text-foreground shadow-control text-[13px] font-medium transition-colors enabled:hover:bg-accent disabled:cursor-default disabled:opacity-50",
              searchOpen ? "w-8 shrink-0 justify-center" : "min-w-0 flex-1 px-2.5 text-left",
            )}
          >
            <SquarePen size={15} strokeWidth={1.6} className="shrink-0" aria-hidden="true" />
            {!searchOpen && <><span className="min-w-0 flex-1 truncate">New Chat</span><span aria-hidden="true" className="text-[11px] font-normal text-muted-foreground">{chordLabel("new-chat")}</span></>}
          </button>
          {!searchOpen && (
            <button
              type="button"
              onClick={openSearch}
              aria-label="Search"
              title="Search"
              className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-[7px] border border-border text-muted-foreground transition-colors hover:bg-card hover:text-foreground"
            >
              <Search size={15} strokeWidth={1.6} aria-hidden="true" />
            </button>
          )}
        </div>

        <nav aria-label="Main navigation" className="mb-4 shrink-0 space-y-0.5">
          <ActionRow icon={Store} label="Marketplace" onClick={onOpenMarketplace} active={marketplaceActive} />
          {/* The work-board stays off the nav for now. Routing props remain on the
           * type (and wired in App) so the screens and their data plumbing are
           * untouched. */}
          <ActionRow icon={FolderGit2} label="Projects" chord="open-projects" onClick={onOpenProjects} active={projectsActive} />
          <ActionRow icon={TerminalSquare} label="Agent Fleet" onClick={onOpenAgentFleet} active={agentFleetActive} />
          <ActionRow icon={LayoutGrid} label="Mission Control" onClick={onOpenMissionControl} active={missionControlActive} />
          <ActionRow icon={Pin} label="Memory" onClick={onOpenMemory} active={memoryActive} />
          {onOpenUsage && <ActionRow icon={ChartNoAxesColumn} label="Usage" onClick={onOpenUsage} active={usageActive} />}
        </nav>

        <div className="-mr-2 min-h-0 flex-1 overflow-y-auto pr-2">
          <SectionLabel action={
            <span className="flex items-center gap-0.5">
              <SidebarFilterMenu view={view} agents={agents} allowProjectGrouping onChange={changeView} />
              <RailIconButton label="New folder" onClick={onOpenProjects}>
                <FolderPlus size={13} strokeWidth={1.5} aria-hidden="true" />
              </RailIconButton>
            </span>
          }>
            Repositories
          </SectionLabel>

          {groups.map(group => {
            // A search is already the short list, so capping it would hide the
            // very rows the query asked for.
            const capped = !searching && !shownInFull.has(group.key) && group.chats.length > GROUP_ROW_CAP;
            // Folding needs a header to unfold from.
            const folded = !!group.label && foldedGroups.has(group.key);
            const rows = folded ? [] : capped ? group.chats.slice(0, GROUP_ROW_CAP) : group.chats;
            const isProjectGroup = view.groupBy === "project" && group.key !== NO_PROJECT_GROUP_KEY;
            const projectIcon = view.groupBy === "project"
              ? (group.key === NO_PROJECT_GROUP_KEY ? Home : Folder)
              : undefined;
            return (
              <div key={group.key} className="mb-2 flex flex-col gap-0.5">
                {group.label && (
                  <GroupLabel
                    label={group.label}
                    count={group.chats.length}
                    folded={folded}
                    active={group.chats.some(chat => chat.id === activeSessionId)}
                    action={isProjectGroup && onNewChatInProject && (
                      <button
                        type="button"
                        onClick={() => onNewChatInProject(group.key)}
                        title={`New chat in ${group.label}`}
                        aria-label={`New chat in ${group.label}`}
                        className="grid size-7 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                      >
                        <Plus size={12} strokeWidth={1.7} aria-hidden="true" />
                      </button>
                    )}
                    icon={projectIcon}
                    onToggle={() => toggleFold(group.key)}
                  />
                )}
                {rows.map(chat => (
                  <ChatRow
                    key={chat.id}
                    chat={chat}
                    active={chat.id === activeSessionId}
                    indented={!!group.label}
                    time={chatListTime(chatTimestamp(chat), now)}
                    onClick={() => onOpenSession(chat.id)}
                    onArchive={onArchiveChat && (() => onArchiveChat(chat))}
                  />
                ))}
                {capped && !folded && (
                  <button
                    type="button"
                    onClick={() => setShownInFull(current => new Set(current).add(group.key))}
                    className={cn(
                      "flex h-6 w-full items-center rounded-md text-left text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground",
                      group.label ? "pl-7" : "px-2",
                    )}
                  >
                    Show {group.chats.length - GROUP_ROW_CAP} more
                  </button>
                )}
              </div>
            );
          })}
          {!visible.length && (
            <p className="px-2 py-1 text-[11px] leading-relaxed text-muted-foreground/70">
              {chats.length ? "No chat matches this filter." : "No chats yet. New Chat opens in the repo you were last in."}
            </p>
          )}
        </div>

        {/* A rail of achromatic icon buttons pinned to the bottom: settings
            (the account's settings entry), source control, usage, and refresh. */}
        <div className="mt-1 flex shrink-0 items-center gap-0.5 border-t border-sidebar-border pt-1.5">
          <button type="button" onClick={onOpenSettings} aria-label={`Open settings for ${accountName}`} aria-current={settingsActive ? "page" : undefined} title={`Open settings for ${accountName}`} className={cn("flex h-8 min-w-0 flex-1 items-center gap-2 rounded-lg px-2 text-[12px] transition-colors", settingsActive ? "bg-selection text-selection-foreground" : "text-muted-foreground hover:bg-accent hover:text-foreground")}><Settings2 size={16} strokeWidth={1.6} aria-hidden="true" /><span className="truncate">Settings</span></button>
          <RailBottomButton label="Source control" onClick={onOpenProjects}>
            <GitBranch size={16} strokeWidth={1.6} aria-hidden="true" />
          </RailBottomButton>
          <RailBottomButton label="Usage" active={usageActive} onClick={() => onOpenUsage?.()}>
            <BarChart3 size={16} strokeWidth={1.6} aria-hidden="true" />
          </RailBottomButton>
          <RailBottomButton label="Refresh" onClick={() => window.location.reload()}>
            <RotateCw size={15} strokeWidth={1.6} aria-hidden="true" />
          </RailBottomButton>
        </div>
      </div>
      </div>

      {!collapsed && (
        <div
          ref={resizeHandleRef}
          role="separator"
          aria-orientation="vertical"
          aria-label="Resize sidebar"
          tabIndex={0}
          aria-valuemin={MIN_WIDTH}
          aria-valuemax={MAX_WIDTH}
          aria-valuenow={width}
          onKeyDown={event => {
            const delta = event.key === "ArrowLeft" ? -16 : event.key === "ArrowRight" ? 16 : 0;
            if (!delta && event.key !== "Home" && event.key !== "End") return;
            event.preventDefault();
            setWidth(current => event.key === "Home" ? MIN_WIDTH : event.key === "End" ? MAX_WIDTH : Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, current + delta)));
          }}
          onPointerDown={startResize}
          className={cn(
            "absolute inset-y-0 -right-3 z-30 hidden w-3 cursor-col-resize touch-none select-none sm:block",
            "after:absolute after:inset-y-4 after:left-0 after:w-px after:transition-colors",
            resizing ? "after:bg-ring/60" : "after:bg-transparent hover:after:bg-border",
          )}
        />
      )}
      </aside>
    </>
  );
}
