import { useCallback, useEffect, useRef, useState } from "react";
import { ChevronRight, FolderGit2, GitBranch, MessageSquarePlus, PanelLeft, Plus } from "lucide-react";
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
  return harness ? harness[0].toUpperCase() + harness.slice(1) : "Agent";
}

function SidebarChatRow({ chat, active, collapsed, onClick }: { chat: Session; active: boolean; collapsed: boolean; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={collapsed ? chat.title || chat.label : undefined}
      className={cn(
        "group my-0.5 flex w-full items-center text-left font-sans transition-colors duration-200 active:scale-[0.98]",
        collapsed ? "h-10 justify-center rounded-xl px-0" : "min-h-[38px] gap-2.5 rounded-2xl px-2.5 py-2",
        active ? "bg-neutral-800/70 text-neutral-100 shadow-[inset_0_1px_0_rgba(255,255,255,0.04)]" : "text-neutral-400 hover:bg-neutral-800/45 hover:text-neutral-200",
      )}
    >
      <StatusDot status={chat.status} />
      {!collapsed && (
        <span className="min-w-0 flex-1">
          <span className="block truncate text-[13px] font-medium leading-tight">{chat.title || chat.label}</span>
          <span className="block truncate text-[10px] font-normal text-neutral-600">{harnessLabel(chat.harness)}{chat.model ? ` · ${chat.model}` : ""}</span>
        </span>
      )}
    </button>
  );
}

export type BridgeSidebarProps = {
  standaloneChats: Session[];
  workspaces: Workspace[];
  workspaceChats: (workspaceId: string) => Session[];
  activeSessionId?: string;
  expanded: Set<string>;
  busy: boolean;
  onOpenNewChat: () => void;
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
  expanded,
  busy,
  onOpenNewChat,
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
        "relative z-20 hidden shrink-0 flex-col overflow-hidden border-r border-neutral-800/70 font-sans antialiased sm:flex",
        "bg-gradient-to-b from-[#1e1e21] via-[#18181b] to-[#141416]",
        "shadow-[inset_-1px_0_0_rgba(255,255,255,0.03)]",
        animateWidth ? "transition-[width] duration-300 ease-[cubic-bezier(0.22,1,0.36,1)]" : "transition-none",
      )}
      style={{ width: sidebarWidth }}
    >
      <div className="pointer-events-none absolute inset-y-0 right-0 w-px bg-gradient-to-b from-transparent via-neutral-600/25 to-transparent" />

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
            className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-xl text-neutral-500 transition-colors hover:bg-neutral-800/60 hover:text-neutral-200 active:scale-95"
            aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
          >
            <PanelLeft className={cn("h-4 w-4 transition-transform duration-200", collapsed && "rotate-180")} strokeWidth={1.75} />
          </button>
          {!collapsed && (
            <p className="min-w-0 truncate text-sm font-semibold tracking-tight text-neutral-200">
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
                ? "mx-auto h-10 w-10 rounded-xl bg-neutral-200 text-neutral-900 hover:bg-neutral-100"
                : "w-full gap-2 rounded-2xl bg-neutral-200 px-3 py-2.5 text-[13px] text-neutral-900 shadow-[0_1px_0_rgba(255,255,255,0.25)_inset,0_8px_24px_-12px_rgba(0,0,0,0.45)] hover:bg-neutral-100",
            )}
          >
            <MessageSquarePlus size={16} strokeWidth={1.75} aria-hidden="true" />
            {!collapsed && "New chat"}
          </button>
        </div>

        <div className="flex-1 overflow-auto scrollbar-thin scrollbar-thumb-neutral-700/60">
          {!collapsed && (
            <div className="mb-1 flex items-center gap-2 px-2 py-1.5">
              <span className="text-[10px] font-semibold uppercase tracking-wider text-neutral-500">Chats</span>
              <span className="h-px flex-1 bg-gradient-to-r from-neutral-700/50 to-transparent" />
            </div>
          )}
          {standaloneChats.map(chat => (
            <SidebarChatRow key={chat.id} chat={chat} active={chat.id === activeSessionId} collapsed={collapsed} onClick={() => onOpenSession(chat.id)} />
          ))}
          {!standaloneChats.length && !collapsed && <div className="px-2.5 py-2 text-[11px] text-neutral-600">No chats yet.</div>}

          {!collapsed && (
            <div className="mt-4 mb-1 flex items-center gap-2 px-2 py-1.5">
              <span className="text-[10px] font-semibold uppercase tracking-wider text-neutral-500">Workspaces</span>
              <span className="h-px flex-1 bg-gradient-to-r from-neutral-700/50 to-transparent" />
              <button type="button" className="text-neutral-500 transition-colors hover:text-neutral-200" onClick={onNewWorkspace} title="New workspace" aria-label="New workspace">
                <Plus size={13} strokeWidth={1.5} aria-hidden="true" />
              </button>
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
                  className="mx-auto my-1 flex h-10 w-10 items-center justify-center rounded-xl text-neutral-500 transition-all hover:bg-neutral-800/50 hover:text-neutral-200"
                >
                  <FolderGit2 size={16} strokeWidth={1.5} aria-hidden="true" />
                </button>
              );
            }
            return (
              <section key={ws.id} className="mb-1">
                <div className="group/ws flex h-[36px] items-center gap-1 rounded-2xl px-2 transition-colors hover:bg-neutral-800/40">
                  <button type="button" className="flex min-w-0 flex-1 items-center gap-2 text-left" onClick={() => onToggleWorkspace(ws.id)}>
                    <ChevronRight size={13} strokeWidth={1.5} className={cn("text-neutral-500 transition-transform", open && "rotate-90")} aria-hidden="true" />
                    <FolderGit2 size={13} strokeWidth={1.5} className="text-neutral-500" aria-hidden="true" />
                    <span className="min-w-0 flex-1 truncate text-[13px] font-medium text-neutral-200">{ws.title}</span>
                  </button>
                  <span className="font-mono text-[10px] text-neutral-600 group-hover/ws:hidden">{chats.length || ""}</span>
                  <button type="button" className="hidden text-neutral-500 transition-colors hover:text-neutral-200 group-hover/ws:flex" title="New agent" aria-label="New agent" disabled={busy} onClick={() => onNewWorkspaceSession(ws.id)}>
                    <Plus size={14} strokeWidth={1.5} aria-hidden="true" />
                  </button>
                </div>
                {open && (
                  <div className="ml-4 border-l border-neutral-700/50 pl-2">
                    {chats.map(chat => <SidebarChatRow key={chat.id} chat={chat} active={chat.id === activeSessionId} collapsed={false} onClick={() => onOpenSession(chat.id)} />)}
                    <div className="flex items-center gap-1 py-1">
                      <button type="button" className="flex h-[26px] items-center gap-1.5 rounded-xl px-2 text-[13px] font-medium text-neutral-500 transition-colors hover:bg-neutral-800/50 hover:text-neutral-200" disabled={busy} onClick={() => onNewWorkspaceSession(ws.id)}>
                        <Plus size={12} strokeWidth={1.5} aria-hidden="true" /> New agent
                      </button>
                      {!ws.path && (
                        <button type="button" className="flex h-[26px] items-center gap-1.5 rounded-xl px-2 text-[13px] font-medium text-neutral-500 transition-colors hover:bg-neutral-800/50 hover:text-neutral-200" onClick={() => onConnectFolder(ws.id)}>
                          <FolderGit2 size={12} strokeWidth={1.5} aria-hidden="true" /> Connect folder
                        </button>
                      )}
                    </div>
                    {ws.path && (
                      <div className="flex items-center gap-1 truncate px-2 py-1 font-mono text-[9.5px] text-neutral-600">
                        <GitBranch size={10} strokeWidth={1.5} aria-hidden="true" />
                        {ws.branch ?? "folder"} · {ws.dirtyFiles ? `${ws.dirtyFiles} changed` : "clean"}
                      </div>
                    )}
                  </div>
                )}
              </section>
            );
          })}
          {!workspaces.length && !collapsed && <div className="px-2.5 py-2 text-[11px] text-neutral-600">Group chats and connect a repo with a workspace.</div>}
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
            "absolute inset-y-0 right-0 z-30 w-3 cursor-col-resize touch-none select-none",
            "after:absolute after:inset-y-4 after:right-0 after:w-px after:transition-colors",
            resizing ? "after:bg-neutral-500/50" : "after:bg-transparent hover:after:bg-neutral-500/35",
          )}
        />
      )}
    </aside>
  );
}
