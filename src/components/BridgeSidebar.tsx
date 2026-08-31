import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { ChevronRight, ClipboardList, Folder, FolderGit2, FolderPlus, GitBranch, Home, LayoutGrid, Pin, Search, Settings2, SquarePen, Store, type LucideIcon } from "lucide-react";
import { WindowNavButtons, WindowPanelButton } from "./WindowNavButtons";
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
  writeChatView,
  type ChatView,
} from "./sidebarChats";

const MIN_WIDTH = 200;
const MAX_WIDTH = 400;
const DEFAULT_WIDTH = 248;
const COLLAPSED_WIDTH = 68;
const WIDTH_KEY = "bridge.sidebar.width";
const COLLAPSED_KEY = "bridge.sidebar.collapsed";

function readWidth(): number {
  const raw = localStorage.getItem(WIDTH_KEY);
  const value = raw ? Number(raw) : DEFAULT_WIDTH;
  if (!Number.isFinite(value)) return DEFAULT_WIDTH;
  return Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, value));
}

// Every row carried a full-strength dot before, so a list of forty read as forty
// signals. Only a state worth acting on keeps its hue; the rest is a placeholder
// that holds the title's left edge.
function StatusDot({ status }: { status: SessionStatus }) {
  const color = status === "working" ? "bg-success"
    : status === "waiting" ? "bg-warning"
    : status === "failed" ? "bg-destructive"
    : status === "ready" ? "bg-info"
    : "bg-muted-foreground/25";
  return <span className={cn("h-1.5 w-1.5 shrink-0 rounded-full", color)} />;
}

function ChatRow({
  chat,
  active,
  collapsed,
  indented,
  branch,
  time,
  onClick,
}: {
  chat: Session;
  active: boolean;
  collapsed: boolean;
  indented: boolean;
  branch: string | null | undefined;
  time: string | null;
  onClick: () => void;
}) {
  const name = chatName(chat);
  const detail = `${name} — ${harnessLabel(chat.harness)}${chat.model ? ` · ${chat.model}` : ""}`;
  const onBranch = !!branch;
  return (
    <button
      type="button"
      onClick={onClick}
      title={detail}
      className={cn(
        "flex w-full items-center text-left font-sans transition-colors active:scale-[0.99]",
        collapsed ? "h-9 justify-center rounded-md px-0" : cn("h-[26px] gap-1.5 rounded-md pr-2 text-[13px] tracking-[-0.008em]", indented ? "pl-7" : "pl-2"),
        active ? "bg-accent font-medium text-foreground" : "text-foreground/80 hover:bg-accent/70 hover:text-foreground",
      )}
    >
      <StatusDot status={chat.status} />
      {!collapsed && (
        <>
          <span className="min-w-0 flex-1 truncate">{name}</span>
          {onBranch && <GitBranch size={11} strokeWidth={1.7} className="shrink-0 text-ring" aria-label="On a git branch" />}
          {time && <span className="shrink-0 text-[11px] tabular-nums text-muted-foreground/65">{time}</span>}
        </>
      )}
    </button>
  );
}

function SectionLabel({ children, action }: { children: React.ReactNode; action?: React.ReactNode }) {
  return (
    <div className="flex h-7 items-center gap-1 px-2">
      <span className="text-[11px] font-semibold tracking-[-0.004em] text-muted-foreground">{children}</span>
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
}: {
  label: string;
  count: number;
  folded: boolean;
  active: boolean;
  icon?: LucideIcon;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onToggle}
      aria-expanded={!folded}
      title={folded ? `Show ${label}` : `Hide ${label}`}
      className={cn(
        "flex h-7 w-full items-center gap-2 rounded-md px-2 text-left text-[13px] tracking-[-0.008em] text-foreground/90 transition-colors",
        active ? "bg-accent font-medium text-foreground" : "hover:bg-accent/70 hover:text-foreground",
      )}
    >
      <ChevronRight
        size={11}
        strokeWidth={2}
        aria-hidden="true"
        className={cn("shrink-0 text-muted-foreground/60 transition-transform", !folded && "rotate-90")}
      />
      {Icon && <Icon size={14} strokeWidth={1.5} className="shrink-0 text-muted-foreground" aria-hidden="true" />}
      <span className="min-w-0 truncate">{label}</span>
      <span className="ml-auto shrink-0 font-mono text-[10px] text-muted-foreground/60">{count}</span>
    </button>
  );
}

function RailIconButton({ label, onClick, children }: { label: string; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={label}
      aria-label={label}
      className="inline-flex h-6 w-6 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
    >
      {children}
    </button>
  );
}

function ActionRow({
  icon: Icon,
  label,
  onClick,
  collapsed,
  active = false,
  disabled = false,
  chord,
}: {
  icon: LucideIcon;
  label: string;
  onClick: () => void;
  collapsed: boolean;
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
        "flex items-center rounded-md text-[13px] tracking-[-0.008em] transition-colors",
        collapsed ? "mx-auto size-9 justify-center" : "h-7 w-full gap-2.5 px-2",
        active ? "bg-accent font-medium text-foreground" : "text-foreground/85 hover:bg-accent hover:text-foreground",
        disabled && "cursor-default opacity-50 hover:bg-transparent hover:text-foreground/85",
      )}
    >
      <Icon size={15} strokeWidth={1.5} className="shrink-0 text-muted-foreground" aria-hidden="true" />
      {!collapsed && label}
    </button>
  );
}

function AccountRow({
  name,
  collapsed,
  active,
  onOpenSettings,
}: {
  name: string;
  collapsed: boolean;
  active: boolean;
  onOpenSettings: () => void;
}) {
  const initial = name.trim().charAt(0).toUpperCase() || "U";
  return (
    <button
      type="button"
      onClick={onOpenSettings}
      title={`Settings · ${name}  ${chordLabel("open-settings")}`}
      aria-label={`Open settings for ${name}`}
      aria-current={active ? "page" : undefined}
      className={cn(
        "flex items-center rounded-lg transition-colors",
        collapsed ? "mx-auto size-10 justify-center" : "h-11 w-full gap-2.5 px-2",
        active ? "bg-accent text-foreground" : "text-foreground/85 hover:bg-accent hover:text-foreground",
      )}
    >
      <span className="grid size-7 shrink-0 place-items-center rounded-full bg-foreground text-[11px] font-semibold text-background">
        {initial}
      </span>
      {!collapsed && (
        <>
          <span className="min-w-0 flex-1 truncate text-left text-[13px] font-medium tracking-[-0.008em]">{name}</span>
          <Settings2 size={16} strokeWidth={1.6} className="shrink-0 text-muted-foreground" aria-hidden="true" />
        </>
      )}
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
  missionControlActive: boolean;
  workActive?: boolean;
  settingsActive: boolean;
  accountName: string;
  newChatBusy?: boolean;
  /** Drawer state below the sm breakpoint, where the rail is off-canvas. */
  mobileOpen?: boolean;
  onCloseMobile?: () => void;
  onOpenNewChat: () => void;
  onOpenProjects: () => void;
  onOpenMarketplace: () => void;
  onOpenMissionControl: () => void;
  onOpenWorkBoard: () => void;
  /** Account memory. Not workspace-gated: a plain chat reaches it identically. */
  onOpenMemory: () => void;
  onOpenSettings: () => void;
  onOpenSession: (id: string) => void;
  /** When set, the rail uses this collapse state instead of its own. */
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
  missionControlActive,
  workActive = false,
  settingsActive,
  accountName,
  newChatBusy = false,
  mobileOpen = false,
  onCloseMobile,
  onOpenNewChat,
  onOpenProjects,
  onOpenMarketplace,
  onOpenMissionControl,
  onOpenWorkBoard,
  onOpenMemory,
  onOpenSettings,
  onOpenSession,
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
  const [skipWidthTransition, setSkipWidthTransition] = useState(false);
  const [view, setView] = useState<ChatView>(readChatView);
  const [searchOpen, setSearchOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [shownInFull, setShownInFull] = useState<Set<string>>(new Set());
  const [foldedGroups, setFoldedGroups] = useState<Set<string>>(new Set());
  const [now, setNow] = useState(() => Date.now());
  const widthRef = useRef(width);
  const resizeHandleRef = useRef<HTMLDivElement>(null);
  widthRef.current = width;

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

  const toggleCollapsed = useCallback(() => {
    setSkipWidthTransition(true);
    setCollapsed(value => !value);
    window.requestAnimationFrame(() => {
      window.requestAnimationFrame(() => setSkipWidthTransition(false));
    });
  }, [setCollapsed]);

  const closeSearch = useCallback(() => {
    setSearchOpen(false);
    setQuery("");
  }, []);

  const toggleSearch = useCallback(() => {
    if (searchOpen) {
      closeSearch();
      return;
    }
    if (collapsed) setCollapsed(false);
    setSearchOpen(true);
  }, [searchOpen, closeSearch, collapsed, setCollapsed]);

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
  const workspaceById = useMemo(
    () => new Map(workspaces.map(workspace => [workspace.id, workspace])),
    [workspaces],
  );
  const visible = useMemo(
    () => filterChats(chats, { query, status: view.status, agent: view.agent, workspaceTitle }),
    [chats, query, view.status, view.agent, workspaceTitle],
  );
  const groups = useMemo(
    () => groupChats(visible, { groupBy: view.groupBy, sortBy: view.sortBy, workspaces, now }),
    [visible, view.groupBy, view.sortBy, workspaces, now],
  );

  const sidebarWidth = collapsed ? COLLAPSED_WIDTH : width;
  const animateWidth = !resizing && !skipWidthTransition;
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
        className={cn(
          "z-40 flex shrink-0 flex-col font-sans antialiased",
          "fixed inset-y-0 left-0 w-[min(84vw,20rem)] transition-transform duration-300 ease-[cubic-bezier(0.22,1,0.36,1)]",
          mobileOpen ? "translate-x-0" : "-translate-x-full",
          "sm:relative sm:z-20 sm:w-(--sidebar-w) sm:translate-x-0",
          "u-vibrancy-sidebar border-r border-sidebar-border bg-sidebar",
          animateWidth ? "sm:transition-[width] sm:duration-300 sm:ease-[cubic-bezier(0.22,1,0.36,1)]" : "sm:transition-none",
        )}
        style={{ "--sidebar-w": `${sidebarWidth}px` } as React.CSSProperties}
      >

      {showWindowNav && (
        collapsed ? (
          <div className="flex h-11 shrink-0 items-center u-traffic-inset pl-24 pr-1.5" data-tauri-drag-region="deep">
            <WindowPanelButton collapsed={collapsed} onToggleCollapsed={toggleCollapsed} />
          </div>
        ) : (
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
        )
      )}
      <div className={cn("flex min-h-0 flex-1 flex-col overflow-hidden px-2 pb-3", showWindowNav ? "pt-1" : "pt-3")}>
        <div className={cn("mb-2 shrink-0", collapsed && "flex flex-col items-center")}>
          <ActionRow icon={SquarePen} label="New Chat" chord="new-chat" collapsed={collapsed} disabled={newChatBusy} onClick={onOpenNewChat} />
          <ActionRow icon={Search} label="Search" collapsed={collapsed} onClick={toggleSearch} />
          <ActionRow icon={Store} label="Marketplace" collapsed={collapsed} onClick={onOpenMarketplace} active={marketplaceActive} />
          <ActionRow icon={LayoutGrid} label="Mission Control" collapsed={collapsed} onClick={onOpenMissionControl} active={missionControlActive} />
          <ActionRow icon={FolderGit2} label="Projects" chord="open-projects" collapsed={collapsed} onClick={onOpenProjects} active={projectsActive} />
          <ActionRow icon={Pin} label="Memory" collapsed={collapsed} onClick={onOpenMemory} active={memoryActive} />
          <ActionRow icon={ClipboardList} label="Work board" collapsed={collapsed} onClick={onOpenWorkBoard} active={workActive} />
        </div>

        {!collapsed && searchOpen && (
          <div className="relative mb-2 shrink-0">
            <Search size={13} strokeWidth={1.7} aria-hidden="true" className="pointer-events-none absolute left-2 top-2 text-muted-foreground" />
            <input
              type="text"
              value={query}
              autoFocus
              onChange={event => setQuery(event.target.value)}
              onKeyDown={event => { if (event.key === "Escape") closeSearch(); }}
              placeholder="Filter chats and projects…"
              aria-label="Filter chats and projects"
              className="h-7 w-full rounded-lg border border-border bg-background pl-7 pr-2 text-[13px] text-foreground outline-none transition-colors placeholder:text-muted-foreground/70 focus:border-ring"
            />
          </div>
        )}

        <div className="-mr-2 min-h-0 flex-1 overflow-y-auto pr-2">
          {!collapsed && (
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
          )}

          {groups.map(group => {
            // The icon rail has nowhere to put the reveal control, so it must not
            // cap either — a cap without its control puts chats out of reach.
            const capped = !collapsed && !searching && !shownInFull.has(group.key) && group.chats.length > GROUP_ROW_CAP;
            // Folding needs a header to unfold from, so the icon rail never folds.
            const folded = !collapsed && !!group.label && foldedGroups.has(group.key);
            const rows = folded ? [] : capped ? group.chats.slice(0, GROUP_ROW_CAP) : group.chats;
            const projectIcon = view.groupBy === "project"
              ? (group.key === NO_PROJECT_GROUP_KEY ? Home : Folder)
              : undefined;
            return (
              <div key={group.key} className={cn("flex flex-col", !collapsed && "mb-0.5 gap-0.5")}>
                {!collapsed && group.label && (
                  <GroupLabel
                    label={group.label}
                    count={group.chats.length}
                    folded={folded}
                    active={group.chats.some(chat => chat.id === activeSessionId)}
                    icon={projectIcon}
                    onToggle={() => toggleFold(group.key)}
                  />
                )}
                {rows.map(chat => (
                  <ChatRow
                    key={chat.id}
                    chat={chat}
                    active={chat.id === activeSessionId}
                    collapsed={collapsed}
                    indented={!!group.label}
                    branch={chat.workspaceId ? workspaceById.get(chat.workspaceId)?.branch : undefined}
                    time={chatListTime(chatTimestamp(chat), now)}
                    onClick={() => onOpenSession(chat.id)}
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
          {!visible.length && !collapsed && (
            <p className="px-2 py-1 text-[11px] leading-relaxed text-muted-foreground/70">
              {chats.length ? "No chat matches this filter." : "No chats yet. New Chat opens in the repo you were last in."}
            </p>
          )}
        </div>

        <div className="mt-1 shrink-0 border-t border-sidebar-border pt-1.5">
          <AccountRow name={accountName} collapsed={collapsed} active={settingsActive} onOpenSettings={onOpenSettings} />
        </div>
      </div>

      {!collapsed && (
        <div
          ref={resizeHandleRef}
          role="separator"
          aria-orientation="vertical"
          aria-label="Resize sidebar"
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
