import { useCallback, useEffect, useRef, useState } from "react";
import { ChevronRight, FolderGit2, FolderOpen, GitBranch, MessageSquarePlus, Package, PanelLeft, Plus, Settings2, Sparkles } from "lucide-react";
import type { Session, SessionStatus, Workspace } from "../types";
import { cn } from "@/lib/utils";

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

function StatusDot({ status }: { status: SessionStatus }) {
  const color = status === "working" ? "bg-emerald-400" : status === "waiting" ? "bg-amber-400" : status === "ready" ? "bg-sky-400" : status === "failed" ? "bg-red-400" : "bg-neutral-500";
  return <span className={cn("h-1.5 w-1.5 shrink-0 rounded-full", color)} />;
}

function harnessLabel(harness?: string | null): string {
  if (harness === "claude") return "Claude";
  if (harness === "codex") return "Codex";
  if (harness === "opencode") return "OpenCode";
  return harness ? harness[0].toUpperCase() + harness.slice(1) : "Agent";
}

function SidebarChatRow({ chat, active, collapsed, onClick }: { chat: Session; active: boolean; collapsed: boolean; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={collapsed ? chat.title || chat.label : undefined}
      className={cn(
        "group my-0.5 flex w-full items-center text-left font-sans transition-all duration-200 active:scale-[0.98]",
        collapsed ? "h-10 justify-center rounded-xl px-0" : "min-h-[36px] gap-2.5 rounded-xl border border-transparent px-2.5 py-1.5",
        active
          ? "border-white/[0.07] bg-white/[0.07] text-neutral-100 shadow-[inset_0_1px_0_rgba(255,255,255,0.05)]"
          : "text-neutral-400 hover:bg-white/[0.045] hover:text-neutral-200",
      )}
    >
      <StatusDot status={chat.status} />
      {!collapsed && (
        <span className="min-w-0 flex-1">
          <span className="block truncate text-[13px] font-medium leading-tight tracking-[-0.006em]">{chat.title || chat.label}</span>
          <span className="mt-px block truncate text-[10px] font-normal text-neutral-600">{harnessLabel(chat.harness)}{chat.model ? ` · ${chat.model}` : ""}</span>
        </span>
      )}
    </button>
  );
}

function SectionLabel({ children, action }: { children: React.ReactNode; action?: React.ReactNode }) {
  return (
    <div className="mb-1 flex items-center gap-2 px-2.5 pb-1.5 pt-1">
      <span className="text-[10px] font-semibold uppercase tracking-[0.16em] text-neutral-600">{children}</span>
      <span className="h-px flex-1 bg-gradient-to-r from-white/[0.06] to-transparent" />
      {action}
    </div>
  );
}

export type BridgeSidebarProps = {
  standaloneChats: Session[];
  workspaces: Workspace[];
  workspaceChats: (workspaceId: string) => Session[];
  activeSessionId?: string;
  marketplaceActive: boolean;
  settingsActive: boolean;
  expanded: Set<string>;
  busy: boolean;
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
  standaloneChats,
  workspaces,
  workspaceChats,
  activeSessionId,
  marketplaceActive,
  settingsActive,
  expanded,
  busy,
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
  const widthRef = useRef(width);
  const resizeHandleRef = useRef<HTMLDivElement>(null);
  widthRef.current = width;

  useEffect(() => {
    if (!collapsed) localStorage.setItem(WIDTH_KEY, String(width));
  }, [width, collapsed]);

  useEffect(() => {
    localStorage.setItem(COLLAPSED_KEY, collapsed ? "1" : "0");
  }, [collapsed]);

  const toggleCollapsed = useCallback(() => {
    setSkipWidthTransition(true);
    setCollapsed(value => !value);
    window.requestAnimationFrame(() => {
      window.requestAnimationFrame(() => setSkipWidthTransition(false));
    });
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

  const sidebarWidth = collapsed ? COLLAPSED_WIDTH : width;
  const animateWidth = !resizing && !skipWidthTransition;

  return (
    <aside
      className={cn(
        "relative z-20 hidden shrink-0 flex-col overflow-hidden font-sans antialiased sm:flex",
        "border-r border-white/[0.055] bg-[#0a0a0d]/55 backdrop-blur-2xl backdrop-saturate-[1.8]",
        animateWidth ? "transition-[width] duration-300 ease-[cubic-bezier(0.22,1,0.36,1)]" : "transition-none",
      )}
      style={{ width: sidebarWidth }}
    >
      {/* Top light — the frosted-glass light source. */}
      <div className="pointer-events-none absolute inset-x-0 top-0 h-24 bg-gradient-to-b from-white/[0.045] to-transparent" />

      <div className={cn("flex min-h-0 h-full flex-col", collapsed ? "px-2 py-4" : "px-3 py-4")}>
        <div
          className={cn(
            "mb-4 grid h-8 shrink-0 items-center",
            collapsed ? "grid-cols-1 justify-items-start pl-0.5" : "grid-cols-[40px_auto_minmax(0,1fr)] gap-1.5 pl-0",
          )}
          data-tauri-drag-region
        >
          {!collapsed && <div className="h-full" data-tauri-drag-region aria-hidden="true" />}
          <button
            type="button"
            onClick={toggleCollapsed}
            className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-neutral-500 transition-colors hover:bg-white/[0.06] hover:text-neutral-200 active:scale-95"
            aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
          >
            <PanelLeft className={cn("h-4 w-4 transition-transform duration-200", collapsed && "rotate-180")} strokeWidth={1.75} />
          </button>
          {!collapsed && (
            <p className="min-w-0 truncate font-display text-[15px] font-semibold tracking-[-0.01em] text-neutral-100">
              bridge
            </p>
          )}
        </div>

        <div className="mb-3 shrink-0">
          <button
            type="button"
            onClick={onOpenNewChat}
            title={collapsed ? "New chat" : undefined}
            className={cn(
              "flex items-center justify-center font-medium transition-all active:scale-[0.98]",
              collapsed
                ? "mx-auto h-10 w-10 rounded-xl bg-neutral-100 text-neutral-900 hover:bg-white"
                : "w-full gap-2 rounded-xl bg-neutral-100 px-3 py-2 text-[13px] tracking-[-0.006em] text-neutral-900 shadow-[inset_0_1px_0_rgba(255,255,255,0.5),0_10px_28px_-14px_rgba(0,0,0,0.6)] hover:bg-white",
            )}
          >
            <MessageSquarePlus size={15} strokeWidth={1.75} aria-hidden="true" />
            {!collapsed && "New chat"}
          </button>
        </div>

        <div className="flex-1 overflow-auto">
          {!collapsed && <SectionLabel>Chats</SectionLabel>}
          {standaloneChats.map(chat => (
            <SidebarChatRow key={chat.id} chat={chat} active={chat.id === activeSessionId} collapsed={collapsed} onClick={() => onOpenSession(chat.id)} />
          ))}
          {!standaloneChats.length && !collapsed && <div className="px-2.5 py-2 text-[11px] text-neutral-600">No chats yet.</div>}

          {!collapsed && (
            <div className="mt-5">
              <SectionLabel
                action={
                  <button type="button" className="rounded-md p-1 text-neutral-600 transition-colors hover:bg-white/[0.06] hover:text-neutral-200" onClick={onNewWorkspace} title="New workspace" aria-label="New workspace">
                    <Plus size={12} strokeWidth={1.75} aria-hidden="true" />
                  </button>
                }
              >
                Workspaces
              </SectionLabel>
            </div>
          )}

          {workspaces.map(ws => {
            const chats = workspaceChats(ws.id);
            const open = expanded.has(ws.id);
            if (collapsed) {
              return (
                <button
                  key={ws.id}
                  type="button"
                  title={ws.title}
                  onClick={() => onToggleWorkspace(ws.id)}
                  className="mx-auto my-1 flex h-10 w-10 items-center justify-center rounded-xl text-neutral-500 transition-all hover:bg-white/[0.06] hover:text-neutral-200"
                >
                  <FolderGit2 size={16} strokeWidth={1.5} aria-hidden="true" />
                </button>
              );
            }
            return (
              <section key={ws.id} className="mb-0.5">
                <div className="group/ws flex h-[34px] items-center gap-1 rounded-xl px-2 transition-colors hover:bg-white/[0.045]">
                  <button type="button" className="flex min-w-0 flex-1 items-center gap-2 text-left" onClick={() => onToggleWorkspace(ws.id)}>
                    <ChevronRight size={12} strokeWidth={1.75} className={cn("text-neutral-600 transition-transform", open && "rotate-90")} aria-hidden="true" />
                    <FolderGit2 size={13} strokeWidth={1.5} className="shrink-0 text-neutral-500" aria-hidden="true" />
                    <span className="min-w-0 flex-1 truncate text-[13px] font-medium tracking-[-0.006em] text-neutral-200">{ws.title}</span>
                  </button>
                  <span className="font-mono text-[10px] text-neutral-600 group-hover/ws:hidden">{chats.length || ""}</span>
                  <button type="button" className="hidden rounded-md p-1 text-neutral-500 transition-colors hover:bg-white/[0.08] hover:text-neutral-200 group-hover/ws:flex" title="New agent" aria-label="New agent" disabled={busy} onClick={() => onNewWorkspaceSession(ws.id)}>
                    <Plus size={13} strokeWidth={1.75} aria-hidden="true" />
                  </button>
                </div>
                {open && (
                  <div className="ml-[17px] border-l border-white/[0.06] pl-1.5">
                    {chats.map(chat => <SidebarChatRow key={chat.id} chat={chat} active={chat.id === activeSessionId} collapsed={false} onClick={() => onOpenSession(chat.id)} />)}
                    <div className="flex items-center gap-1.5 py-1.5 pl-1">
                      <button
                        type="button"
                        className="inline-flex h-7 items-center gap-1.5 rounded-lg border border-white/[0.06] bg-white/[0.03] px-2.5 text-[11px] font-medium text-neutral-400 shadow-[inset_0_1px_0_rgba(255,255,255,0.04)] transition-all hover:border-white/[0.1] hover:bg-white/[0.06] hover:text-neutral-100 active:scale-[0.97] disabled:opacity-40"
                        disabled={busy}
                        onClick={() => onNewWorkspaceSession(ws.id)}
                      >
                        <Sparkles size={11} strokeWidth={1.75} aria-hidden="true" /> New agent
                      </button>
                      {!ws.path && (
                        <button
                          type="button"
                          className="inline-flex h-7 items-center gap-1.5 rounded-lg border border-white/[0.06] bg-white/[0.03] px-2.5 text-[11px] font-medium text-neutral-400 shadow-[inset_0_1px_0_rgba(255,255,255,0.04)] transition-all hover:border-white/[0.1] hover:bg-white/[0.06] hover:text-neutral-100 active:scale-[0.97]"
                          onClick={() => onConnectFolder(ws.id)}
                        >
                          <FolderOpen size={11} strokeWidth={1.75} aria-hidden="true" /> Connect folder
                        </button>
                      )}
                    </div>
                    {ws.path && (
                      <div className="flex items-center gap-1.5 truncate px-2.5 pb-1.5 font-mono text-[10px] text-neutral-600">
                        <GitBranch size={10} strokeWidth={1.5} aria-hidden="true" />
                        {ws.branch ?? "folder"} · {ws.dirtyFiles ? `${ws.dirtyFiles} changed` : "clean"}
                      </div>
                    )}
                  </div>
                )}
              </section>
            );
          })}
          {!workspaces.length && !collapsed && <div className="px-2.5 py-2 text-[11px] leading-relaxed text-neutral-600">Group chats and connect a repo with a workspace.</div>}
        </div>

        <button
          type="button"
          onClick={onOpenMarketplace}
          title={collapsed ? "Marketplace" : undefined}
          className={cn(
            "mt-3 flex shrink-0 items-center rounded-xl transition-all",
            collapsed ? "mx-auto h-10 w-10 justify-center" : "h-9 gap-2.5 border border-transparent px-2.5 text-[12px] font-medium",
            marketplaceActive
              ? "border-white/[0.07] bg-white/[0.07] text-neutral-100"
              : "text-neutral-500 hover:bg-white/[0.045] hover:text-neutral-200",
          )}
        >
          <Package size={15} strokeWidth={1.6} aria-hidden="true" />
          {!collapsed && "Marketplace"}
        </button>
        <button
          type="button"
          onClick={onOpenSettings}
          title={collapsed ? "Settings" : undefined}
          className={cn(
            "mt-1 flex shrink-0 items-center rounded-xl transition-all",
            collapsed ? "mx-auto h-10 w-10 justify-center" : "h-9 gap-2.5 border border-transparent px-2.5 text-[12px] font-medium",
            settingsActive
              ? "border-white/[0.07] bg-white/[0.07] text-neutral-100"
              : "text-neutral-500 hover:bg-white/[0.045] hover:text-neutral-200",
          )}
        >
          <Settings2 size={15} strokeWidth={1.6} aria-hidden="true" />
          {!collapsed && "Settings"}
        </button>
      </div>

      {!collapsed && (
        <div
          ref={resizeHandleRef}
          role="separator"
          aria-orientation="vertical"
          aria-label="Resize sidebar"
          onPointerDown={startResize}
          className={cn(
            "absolute inset-y-0 right-0 z-30 w-3 cursor-col-resize touch-none select-none",
            "after:absolute after:inset-y-4 after:right-0 after:w-px after:transition-colors",
            resizing ? "after:bg-white/[0.25]" : "after:bg-transparent hover:after:bg-white/[0.15]",
          )}
        />
      )}
    </aside>
  );
}
