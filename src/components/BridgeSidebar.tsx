import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ChevronRight, Code2, FolderGit2, ListChecks, MessagesSquare, Package, PanelLeft, Plus, Search, Settings2, X } from "lucide-react";
import type { Session, SessionStatus, Workspace } from "../types";
import { cn } from "@/lib/utils";
import { harnessLabel } from "../utils";
import { SidebarFilterMenu } from "./SidebarFilterMenu";
import {
  GROUP_ROW_CAP,
  agentOptions,
  chatName,
  filterChats,
  groupChats,
  chatScope,
  inScope,
  readChatScope,
  readChatView,
  writeChatScope,
  writeChatView,
  type ChatScope,
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

function ChatRow({ chat, active, collapsed, onClick }: { chat: Session; active: boolean; collapsed: boolean; onClick: () => void }) {
  const name = chatName(chat);
  // Harness and model used to sit under every title, which is what made the list
  // read as a wall. They stay one hover away, and on the active row only.
  const detail = `${name} — ${harnessLabel(chat.harness)}${chat.model ? ` · ${chat.model}` : ""}`;
  return (
    <button
      type="button"
      onClick={onClick}
      title={detail}
      className={cn(
        "flex w-full items-center text-left font-sans transition-colors active:scale-[0.99]",
        collapsed ? "h-9 justify-center rounded-md px-0" : "h-7 gap-2 rounded-md px-2 text-[13px] tracking-[-0.006em]",
        active ? "bg-accent font-medium text-foreground" : "text-muted-foreground hover:bg-accent hover:text-foreground",
      )}
    >
      <StatusDot status={chat.status} />
      {!collapsed && (
        <>
          <span className="min-w-0 flex-1 truncate">{name}</span>
          {active && chat.model && <span className="shrink-0 font-mono text-[10px] text-muted-foreground">{chat.model}</span>}
        </>
      )}
    </button>
  );
}

// Work is where plain conversations live; Code is work inside a project. One
// switch, so the two never interleave in one list.
function ScopeSwitch({ scope, collapsed, onChange }: { scope: ChatScope; collapsed: boolean; onChange: (scope: ChatScope) => void }) {
  const options: { id: ChatScope; label: string; icon: typeof Code2 }[] = [
    { id: "work", label: "Work", icon: MessagesSquare },
    { id: "code", label: "Code", icon: Code2 },
  ];
  return (
    <div
      role="tablist"
      aria-label="Chat scope"
      className={cn(
        "flex shrink-0 rounded-lg border border-border bg-muted p-0.5",
        collapsed ? "mb-3 flex-col gap-0.5" : "mb-3 h-8 gap-0.5",
      )}
    >
      {options.map(option => {
        const active = option.id === scope;
        const Icon = option.icon;
        return (
          <button
            key={option.id}
            type="button"
            role="tab"
            aria-selected={active}
            aria-label={option.label}
            title={option.label}
            onClick={() => onChange(option.id)}
            className={cn(
              "flex items-center justify-center gap-1.5 rounded-md text-[12.5px] font-medium transition-colors",
              collapsed ? "h-8 w-full" : "h-7 flex-1",
              active ? "bg-card text-foreground" : "text-muted-foreground hover:text-foreground",
            )}
          >
            <Icon size={14} strokeWidth={1.7} aria-hidden="true" />
            {!collapsed && option.label}
          </button>
        );
      })}
    </div>
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

function GroupLabel({ label, count, folded, onToggle }: { label: string; count: number; folded: boolean; onToggle: () => void }) {
  return (
    <button
      type="button"
      onClick={onToggle}
      aria-expanded={!folded}
      title={folded ? `Show ${label}` : `Hide ${label}`}
      className="sticky top-0 z-[1] flex h-6 w-full items-center gap-1.5 rounded-md bg-sidebar px-2 text-left transition-colors hover:bg-accent"
    >
      <ChevronRight
        size={11}
        strokeWidth={2}
        aria-hidden="true"
        className={cn("shrink-0 text-muted-foreground/60 transition-transform", !folded && "rotate-90")}
      />
      <span className="min-w-0 truncate text-[11px] text-muted-foreground/80">{label}</span>
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

export type BridgeSidebarProps = {
  /** Every top-level session. The rail derives both the project tree and the
   * history from this one list, so a chat cannot be visible in one and missing
   * from the other. */
  chats: Session[];
  /** Only for `Group by → Project` labels; the tree itself lives on the projects
   * screen now. */
  workspaces: Workspace[];
  activeSessionId?: string;
  /** True while the Work board is the surface on the right. */
  workBoardActive: boolean;
  /** How many facts are waiting, for the rail's count. Blocking and attention only,
   * so it can actually reach zero. */
  workNeedsYouCount: number;
  projectsActive: boolean;
  marketplaceActive: boolean;
  settingsActive: boolean;
  /** Drawer state below the sm breakpoint, where the rail is off-canvas. */
  mobileOpen?: boolean;
  onCloseMobile?: () => void;
  /** Opens the new-chat dialog, which asks for project and worktree. */
  onOpenNewChat: () => void;
  /** Show the Work board. Called when the pill flips to Work, and when the rail's
   * own row is clicked. */
  onOpenWorkBoard: () => void;
  onOpenProjects: () => void;
  onOpenMarketplace: () => void;
  onOpenSettings: () => void;
  onOpenSession: (id: string) => void;
};

export function BridgeSidebar({
  chats,
  workspaces,
  activeSessionId,
  workBoardActive,
  workNeedsYouCount,
  projectsActive,
  marketplaceActive,
  settingsActive,
  mobileOpen = false,
  onCloseMobile,
  onOpenNewChat,
  onOpenWorkBoard,
  onOpenProjects,
  onOpenMarketplace,
  onOpenSettings,
  onOpenSession,
}: BridgeSidebarProps) {
  const [width, setWidth] = useState(readWidth);
  const [collapsed, setCollapsed] = useState(() => localStorage.getItem(COLLAPSED_KEY) === "1");
  const [resizing, setResizing] = useState(false);
  const [skipWidthTransition, setSkipWidthTransition] = useState(false);
  const [view, setView] = useState<ChatView>(readChatView);
  const [scope, setScope] = useState<ChatScope>(readChatScope);
  const [searchOpen, setSearchOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [shownInFull, setShownInFull] = useState<Set<string>>(new Set());
  const [foldedGroups, setFoldedGroups] = useState<Set<string>>(new Set());
  const [now, setNow] = useState(() => Date.now());
  const widthRef = useRef(width);
  const resizeHandleRef = useRef<HTMLDivElement>(null);
  const followedRef = useRef<string | undefined>(undefined);
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

  // Follow the chat that just opened. Starting a plain chat from Code, or opening
  // a project chat from the projects screen, would otherwise leave the rail
  // showing a list the active chat is not in. Once per id, so a manual switch
  // afterwards survives the next poll — and so a chat that arrives a render later
  // than its id is still followed.
  useEffect(() => {
    if (!activeSessionId || followedRef.current === activeSessionId) return;
    const active = chats.find(chat => chat.id === activeSessionId);
    if (!active) return;
    followedRef.current = activeSessionId;
    const next = chatScope(active);
    setScope(current => {
      if (current === next) return current;
      writeChatScope(next);
      return next;
    });
  }, [activeSessionId, chats]);

  const changeScope = useCallback((next: ChatScope) => {
    setScope(next);
    writeChatScope(next);
    setShownInFull(new Set());
    setFoldedGroups(new Set());
    // Work's surface is the board and Code's is a conversation, so the pill is the
    // gesture that opens the board. Flipping to Code does not pick a chat — the
    // effect above already follows whichever one is active.
    if (next === "work") onOpenWorkBoard();
  }, [onOpenWorkBoard]);

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
  }, []);

  const closeSearch = useCallback(() => {
    setSearchOpen(false);
    setQuery("");
  }, []);

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
      const next = Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, startWidth + moveEvent.clientX - startX));
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

  // A project grouping persisted from Code is meaningless in Home, so it is
  // corrected rather than left to render one "No project" group.
  useEffect(() => {
    if (scope === "work" && view.groupBy === "project") changeView({ ...view, groupBy: "date" });
  }, [scope, view, changeView]);

  const toggleFold = useCallback((key: string) => {
    setFoldedGroups(current => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }, []);

  const scoped = useMemo(() => inScope(chats, scope), [chats, scope]);
  const agents = useMemo(() => agentOptions(scoped), [scoped]);
  const workspaceTitle = useMemo(() => {
    const titles = new Map(workspaces.map(workspace => [workspace.id, workspace.title]));
    return (id: string | null | undefined) => (id ? titles.get(id) : undefined);
  }, [workspaces]);
  const visible = useMemo(
    () => filterChats(scoped, { query, status: view.status, agent: view.agent, workspaceTitle }),
    [scoped, query, view.status, view.agent, workspaceTitle],
  );
  const groups = useMemo(
    () => groupChats(visible, { groupBy: view.groupBy, sortBy: view.sortBy, workspaces, now }),
    [visible, view.groupBy, view.sortBy, workspaces, now],
  );

  const sidebarWidth = collapsed ? COLLAPSED_WIDTH : width;
  const animateWidth = !resizing && !skipWidthTransition;

  return (
    <>
      {/* Below sm the rail is an off-canvas drawer, so narrow windows keep
          their navigation instead of losing it entirely. */}
      {mobileOpen && (
        <button
          type="button"
          className="fixed inset-0 z-30 bg-scrim sm:hidden"
          onClick={onCloseMobile}
          aria-label="Close navigation"
        />
      )}
      <aside
        className={cn(
          "z-40 flex shrink-0 flex-col overflow-hidden font-sans antialiased",
          "fixed inset-y-0 left-0 w-[min(84vw,20rem)] transition-transform duration-300 ease-[cubic-bezier(0.22,1,0.36,1)]",
          mobileOpen ? "translate-x-0" : "-translate-x-full",
          "sm:relative sm:z-20 sm:w-(--sidebar-w) sm:translate-x-0",
          "border-r border-sidebar-border bg-sidebar",
          animateWidth ? "sm:transition-[width] sm:duration-300 sm:ease-[cubic-bezier(0.22,1,0.36,1)]" : "sm:transition-none",
        )}
        style={{ "--sidebar-w": `${sidebarWidth}px` } as React.CSSProperties}
      >

      <div className={cn("flex min-h-0 h-full flex-col", collapsed ? "px-2 py-3" : "px-2 py-3")}>
        <div
          className={cn(
            "mb-3 flex h-7 shrink-0 items-center",
            collapsed ? "justify-start pl-0.5" : "justify-between",
          )}
          data-tauri-drag-region="deep"
        >
          <button
            type="button"
            onClick={toggleCollapsed}
            className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground active:scale-95"
            aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
          >
            <PanelLeft className={cn("h-4 w-4 transition-transform duration-200", collapsed && "rotate-180")} strokeWidth={1.75} />
          </button>
          {!collapsed && (
            <RailIconButton
              label={searchOpen ? "Close search" : "Search chats"}
              onClick={() => (searchOpen ? closeSearch() : setSearchOpen(true))}
            >
              {searchOpen
                ? <X size={14} strokeWidth={1.7} aria-hidden="true" />
                : <Search size={14} strokeWidth={1.7} aria-hidden="true" />}
            </RailIconButton>
          )}
        </div>

        <ScopeSwitch scope={scope} collapsed={collapsed} onChange={changeScope} />

        <div className="mb-4 shrink-0">
          <button
            type="button"
            onClick={onOpenNewChat}
            title={collapsed ? "New chat" : undefined}
            className={cn(
              "flex items-center font-medium transition-all active:scale-[0.98]",
              collapsed
                ? "mx-auto h-9 w-9 justify-center rounded-lg bg-primary text-primary-foreground hover:opacity-90"
                : "h-8 w-full gap-2 rounded-lg bg-primary px-2.5 text-[13px] tracking-[-0.006em] text-primary-foreground hover:opacity-90",
            )}
          >
            <Plus size={15} strokeWidth={1.9} aria-hidden="true" />
            {!collapsed && "New chat"}
          </button>
        </div>

        {!collapsed && searchOpen && (
          <div className="relative mb-3 shrink-0">
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

        {scope === "work" && (
          <div className="mb-2 shrink-0">
            <button
              type="button"
              onClick={onOpenWorkBoard}
              aria-current={workBoardActive ? "page" : undefined}
              title={collapsed ? "Needs you" : undefined}
              className={cn(
                "flex shrink-0 items-center rounded-md transition-colors",
                collapsed ? "mx-auto h-9 w-9 justify-center" : "h-7.5 w-full gap-2 px-2 text-[11.5px] font-medium",
                workBoardActive ? "bg-accent text-foreground" : "text-muted-foreground hover:bg-accent hover:text-foreground",
              )}
            >
              <ListChecks size={14} strokeWidth={1.7} aria-hidden="true" />
              {!collapsed && (
                <>
                  Needs you
                  {/* No badge at zero. A count that is always present is a count that
                      stops being read. */}
                  {workNeedsYouCount > 0 && (
                    <span className={cn(
                      "ml-auto flex h-4 min-w-4 items-center justify-center rounded-full px-1 text-[10px] font-semibold",
                      workBoardActive ? "bg-primary text-primary-foreground" : "bg-muted text-muted-foreground",
                    )}>
                      {workNeedsYouCount}
                    </span>
                  )}
                </>
              )}
            </button>
            <div className="mt-2 h-px bg-sidebar-border" />
          </div>
        )}

        <div className="flex-1 overflow-y-auto">
          {!collapsed && (
            <SectionLabel action={<SidebarFilterMenu view={view} agents={agents} allowProjectGrouping={scope === "code"} onChange={changeView} />}>
              Chats
            </SectionLabel>
          )}

          {groups.map(group => {
            // The icon rail has nowhere to put the reveal control, so it must not
            // cap either — a cap without its control puts chats out of reach.
            const capped = !collapsed && !shownInFull.has(group.key) && group.chats.length > GROUP_ROW_CAP;
            // Folding needs a header to unfold from, so the icon rail never folds.
            const folded = !collapsed && !!group.label && foldedGroups.has(group.key);
            const rows = folded ? [] : capped ? group.chats.slice(0, GROUP_ROW_CAP) : group.chats;
            return (
              <div key={group.key}>
                {!collapsed && group.label && (
                  <GroupLabel
                    label={group.label}
                    count={group.chats.length}
                    folded={folded}
                    onToggle={() => toggleFold(group.key)}
                  />
                )}
                {rows.map(chat => (
                  <ChatRow key={chat.id} chat={chat} active={chat.id === activeSessionId} collapsed={collapsed} onClick={() => onOpenSession(chat.id)} />
                ))}
                {capped && !folded && (
                  <button
                    type="button"
                    onClick={() => setShownInFull(current => new Set(current).add(group.key))}
                    className="flex h-6 w-full items-center rounded-md px-2 text-left text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                  >
                    Show {group.chats.length - GROUP_ROW_CAP} more
                  </button>
                )}
              </div>
            );
          })}
          {!visible.length && !collapsed && (
            <p className="px-2 py-1 text-[11px] leading-relaxed text-muted-foreground/70">
              {scoped.length
                ? "No chat matches this filter."
                : scope === "code"
                  ? "No project chats yet. New chat asks which project to run in."
                  : "No chats yet."}
            </p>
          )}
        </div>

        <div className={cn("mt-2 shrink-0 border-t border-sidebar-border pt-2", collapsed && "flex flex-col items-center")}>
          <button
            type="button"
            onClick={onOpenProjects}
            title={collapsed ? "Projects" : undefined}
            className={cn(
              "flex shrink-0 items-center rounded-md transition-colors",
              collapsed ? "h-9 w-9 justify-center" : "h-7 w-full gap-2 px-2 text-[11px] font-medium",
              projectsActive ? "bg-accent text-foreground" : "text-muted-foreground hover:bg-accent hover:text-foreground",
            )}
          >
            <FolderGit2 size={14} strokeWidth={1.6} aria-hidden="true" />
            {!collapsed && "Projects"}
          </button>
          <button
            type="button"
            onClick={onOpenMarketplace}
            title={collapsed ? "Marketplace" : undefined}
            className={cn(
              "mt-0.5 flex shrink-0 items-center rounded-md transition-colors",
              collapsed ? "h-9 w-9 justify-center" : "h-7 w-full gap-2 px-2 text-[11px] font-medium",
              marketplaceActive ? "bg-accent text-foreground" : "text-muted-foreground hover:bg-accent hover:text-foreground",
            )}
          >
            <Package size={14} strokeWidth={1.6} aria-hidden="true" />
            {!collapsed && "Marketplace"}
          </button>
          <button
            type="button"
            onClick={onOpenSettings}
            title={collapsed ? "Settings" : undefined}
            className={cn(
              "mt-0.5 flex shrink-0 items-center rounded-md transition-colors",
              collapsed ? "h-9 w-9 justify-center" : "h-7 w-full gap-2 px-2 text-[11px] font-medium",
              settingsActive ? "bg-accent text-foreground" : "text-muted-foreground hover:bg-accent hover:text-foreground",
            )}
          >
            <Settings2 size={14} strokeWidth={1.6} aria-hidden="true" />
            {!collapsed && "Settings"}
          </button>
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
            "absolute inset-y-0 right-0 z-30 hidden w-3 cursor-col-resize touch-none select-none sm:block",
            "after:absolute after:inset-y-4 after:right-0 after:w-px after:transition-colors",
            resizing ? "after:bg-ring/60" : "after:bg-transparent hover:after:bg-border",
          )}
        />
      )}
      </aside>
    </>
  );
}
