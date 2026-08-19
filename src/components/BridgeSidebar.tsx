import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ChevronRight, FolderGit2, FolderOpen, Package, PanelLeft, Plus, Search, Settings2, Sparkles, X } from "lucide-react";
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

function SectionLabel({ children, action }: { children: React.ReactNode; action?: React.ReactNode }) {
  return (
    <div className="flex h-7 items-center gap-1 px-2">
      <span className="text-[11px] font-semibold tracking-[-0.004em] text-muted-foreground">{children}</span>
      {action && <span className="ml-auto flex items-center gap-0.5">{action}</span>}
    </div>
  );
}

function GroupLabel({ label, count }: { label: string; count: number }) {
  return (
    <div className="sticky top-0 z-[1] flex h-6 items-center gap-2 bg-sidebar px-2">
      <span className="text-[11px] text-muted-foreground/80">{label}</span>
      <span className="ml-auto font-mono text-[10px] text-muted-foreground/60">{count}</span>
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
  workspaces: Workspace[];
  activeSessionId?: string;
  marketplaceActive: boolean;
  settingsActive: boolean;
  expanded: Set<string>;
  busy: boolean;
  /** Drawer state below the sm breakpoint, where the rail is off-canvas. */
  mobileOpen?: boolean;
  onCloseMobile?: () => void;
  onOpenNewChat: () => void;
  onOpenMarketplace: () => void;
  onOpenSettings: () => void;
  onOpenSession: (id: string) => void;
  onToggleWorkspace: (id: string) => void;
  onNewWorkspace: () => void;
  onNewWorkspaceSession: (workspaceId: string) => void;
  onConnectFolder: (workspaceId: string) => void;
};

export function BridgeSidebar({
  chats,
  workspaces,
  activeSessionId,
  marketplaceActive,
  settingsActive,
  expanded,
  busy,
  mobileOpen = false,
  onCloseMobile,
  onOpenNewChat,
  onOpenMarketplace,
  onOpenSettings,
  onOpenSession,
  onToggleWorkspace,
  onNewWorkspace,
  onNewWorkspaceSession,
  onConnectFolder,
}: BridgeSidebarProps) {
  const [width, setWidth] = useState(readWidth);
  const [collapsed, setCollapsed] = useState(() => localStorage.getItem(COLLAPSED_KEY) === "1");
  const [resizing, setResizing] = useState(false);
  const [skipWidthTransition, setSkipWidthTransition] = useState(false);
  const [view, setView] = useState<ChatView>(readChatView);
  const [searchOpen, setSearchOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [shownInFull, setShownInFull] = useState<Set<string>>(new Set());
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
    // A cap belongs to a group key, and the keys change meaning with the grouping.
    setShownInFull(new Set());
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
  const projects = useMemo(
    () => workspaces
      .map(workspace => ({ workspace, chats: visible.filter(chat => chat.workspaceId === workspace.id) }))
      .filter(entry => !searching || entry.chats.length > 0 || entry.workspace.title.toLowerCase().includes(needle)),
    [workspaces, visible, searching, needle],
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
            "mb-2.5 grid h-7 shrink-0 items-center",
            collapsed ? "grid-cols-1 justify-items-start pl-0.5" : "grid-cols-[40px_28px_minmax(0,1fr)_auto] gap-1",
          )}
          data-tauri-drag-region
        >
          {!collapsed && <div className="h-full" data-tauri-drag-region aria-hidden="true" />}
          <button
            type="button"
            onClick={toggleCollapsed}
            className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground active:scale-95"
            aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
          >
            <PanelLeft className={cn("h-4 w-4 transition-transform duration-200", collapsed && "rotate-180")} strokeWidth={1.75} />
          </button>
          {!collapsed && (
            <>
              <p className="min-w-0 truncate font-display text-[14px] font-semibold tracking-[-0.012em] text-foreground">
                bridge
              </p>
              <RailIconButton
                label={searchOpen ? "Close search" : "Search chats"}
                onClick={() => (searchOpen ? closeSearch() : setSearchOpen(true))}
              >
                {searchOpen
                  ? <X size={14} strokeWidth={1.7} aria-hidden="true" />
                  : <Search size={14} strokeWidth={1.7} aria-hidden="true" />}
              </RailIconButton>
            </>
          )}
        </div>

        <div className="mb-2 shrink-0">
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

        <div className="flex-1 overflow-y-auto">
          {!collapsed && (
            <SectionLabel
              action={
                <RailIconButton label="New project" onClick={onNewWorkspace}>
                  <Plus size={13} strokeWidth={1.9} aria-hidden="true" />
                </RailIconButton>
              }
            >
              Projects
            </SectionLabel>
          )}

          {projects.map(({ workspace, chats: projectChats }) => {
            // A search is a temporary view of the tree, so it opens what it
            // matches without touching the caller's expansion state.
            const open = searching || expanded.has(workspace.id);
            if (collapsed) {
              return (
                <button
                  key={workspace.id}
                  type="button"
                  title={workspace.title}
                  onClick={() => onToggleWorkspace(workspace.id)}
                  className="mx-auto my-0.5 flex h-9 w-9 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                >
                  <FolderGit2 size={16} strokeWidth={1.5} aria-hidden="true" />
                </button>
              );
            }
            return (
              <section key={workspace.id}>
                <div className="group/ws flex h-7 items-center gap-1 rounded-md px-2 transition-colors hover:bg-accent">
                  <button type="button" className="flex min-w-0 flex-1 items-center gap-1.5 text-left" onClick={() => onToggleWorkspace(workspace.id)}>
                    <ChevronRight size={12} strokeWidth={1.75} className={cn("shrink-0 text-muted-foreground/70 transition-transform", open && "rotate-90")} aria-hidden="true" />
                    <FolderGit2 size={13} strokeWidth={1.5} className="shrink-0 text-muted-foreground" aria-hidden="true" />
                    <span className="min-w-0 flex-1 truncate text-[13px] tracking-[-0.006em] text-foreground">{workspace.title}</span>
                  </button>
                  <span className="font-mono text-[10px] text-muted-foreground/60 group-hover/ws:hidden">{projectChats.length || ""}</span>
                  <button type="button" className="hidden rounded-md p-0.5 text-muted-foreground transition-colors hover:text-foreground group-hover/ws:flex" title="New agent" aria-label="New agent" disabled={busy} onClick={() => onNewWorkspaceSession(workspace.id)}>
                    <Plus size={13} strokeWidth={1.9} aria-hidden="true" />
                  </button>
                </div>
                {open && (
                  <div className="ml-[15px] border-l border-border pl-1">
                    {projectChats.map(chat => (
                      <ChatRow key={chat.id} chat={chat} active={chat.id === activeSessionId} collapsed={false} onClick={() => onOpenSession(chat.id)} />
                    ))}
                    <div className="flex items-center gap-1.5 py-1 pl-1">
                      <button
                        type="button"
                        className="inline-flex h-6 items-center gap-1.5 rounded-md px-1.5 text-[11px] font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-40"
                        disabled={busy}
                        onClick={() => onNewWorkspaceSession(workspace.id)}
                      >
                        <Sparkles size={11} strokeWidth={1.75} aria-hidden="true" /> New agent
                      </button>
                      {!workspace.path && (
                        <button
                          type="button"
                          className="inline-flex h-6 items-center gap-1.5 rounded-md px-1.5 text-[11px] font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                          onClick={() => onConnectFolder(workspace.id)}
                        >
                          <FolderOpen size={11} strokeWidth={1.75} aria-hidden="true" /> Connect folder
                        </button>
                      )}
                    </div>
                  </div>
                )}
              </section>
            );
          })}
          {!projects.length && !collapsed && (
            <p className="px-2 py-1 text-[11px] leading-relaxed text-muted-foreground/70">
              {searching ? "No project matches." : "Group chats and connect a repo with a workspace."}
            </p>
          )}

          {!collapsed && (
            <div className="mt-1">
              <SectionLabel action={<SidebarFilterMenu view={view} agents={agents} onChange={changeView} />}>
                Chats
              </SectionLabel>
            </div>
          )}

          {groups.map(group => {
            const capped = !shownInFull.has(group.key) && group.chats.length > GROUP_ROW_CAP;
            const rows = capped ? group.chats.slice(0, GROUP_ROW_CAP) : group.chats;
            return (
              <div key={group.key}>
                {!collapsed && group.label && <GroupLabel label={group.label} count={group.chats.length} />}
                {rows.map(chat => (
                  <ChatRow key={chat.id} chat={chat} active={chat.id === activeSessionId} collapsed={collapsed} onClick={() => onOpenSession(chat.id)} />
                ))}
                {capped && !collapsed && (
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
            <p className="px-2 py-1 text-[11px] text-muted-foreground/70">
              {chats.length ? "No chat matches this filter." : "No chats yet."}
            </p>
          )}
        </div>

        <div className={cn("mt-2 shrink-0 border-t border-sidebar-border pt-2", collapsed && "flex flex-col items-center")}>
          <button
            type="button"
            onClick={onOpenMarketplace}
            title={collapsed ? "Marketplace" : undefined}
            className={cn(
              "flex shrink-0 items-center rounded-md transition-colors",
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
