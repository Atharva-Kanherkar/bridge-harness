import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Archive, AtSign, Blocks, Check, CheckCheck, ChevronRight, CircleDot, Clock, Copy, Folder, FolderPlus, GitFork, Home, MoreHorizontal, Plus, Search, Settings2, SquarePen, Workflow as WorkflowIcon, type LucideIcon } from "lucide-react";
import { WindowNavButtons } from "./WindowNavButtons";
import { appById, type AppId, type BadgeText } from "../apps/registry";
import { HarnessMark } from "./harnessMarks";
import { FittedTitle } from "./FittedTitle";
import type { AttentionKind, Session, SessionStatus, Workspace } from "../types";
import type { SessionPullRequest } from "../protocol/generated/protocol";
import { PullRequestGlyph, type SidebarPullRequests } from "./ChatPullRequests";
import { SHELF_TONE } from "./WorkShelf";
import { sidebarPrChip } from "../workSummary";
import { chordLabel, type CommandId } from "../keymap";
import { cn } from "@/lib/utils";
import { MOTION_DURATION, useMotionTransition } from "../motion";
import { harnessLabel } from "../utils";
import { NOTIFY_LEVELS, useDelegationNotify } from "../delegationNotify";
import { mentionToken, toPublicAlias } from "../referenceChip";
import { MenuPanel, MenuSeparator, useMenuPanel } from "@/components/ui/menu-panel";
import { SidebarFilterMenu } from "./SidebarFilterMenu";
import { ChatSearchResults } from "./ChatSearchResults";
import { CHAT_SEARCH_MIN_CHARS, enterAction, useChatSearch } from "../chatSearch";
import { SIDEBAR_CHAT_DRAG } from "./missionControl/drag";
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
  type LiveAgents,
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

function toggleKey(keys: Set<string>, key: string): Set<string> {
  const next = new Set(keys);
  if (next.has(key)) next.delete(key);
  else next.add(key);
  return next;
}

// Working, waiting, and failure states pair a status dot with readable text.
// An unread request anywhere in the chat's tree reads as waiting too: workers
// never get a row, so their approvals surface on the chat that started them.
function rowStatus(status: SessionStatus, liveAgents = 0, unread?: AttentionKind): { label: string; dot: string } | null {
  const bucket = statusBucket(status);
  if (bucket === "waiting" || unread === "needsYou") return { label: "needs you", dot: "bg-warning" };
  if (bucket === "active") return { label: "working", dot: "bg-success" };
  if (bucket === "failed") return { label: "failed", dot: "bg-destructive" };
  // The orchestrator's turn is over but the agents it started are not. A
  // different dot, because the chat is busy without being the one working.
  if (liveAgents > 0) return { label: `${liveAgents} ${liveAgents === 1 ? "agent" : "agents"} working`, dot: "bg-info" };
  return null;
}

/** How long the pointer rests on a row before its history is read. Long
 * enough that sweeping across the list reads nothing, short enough to finish
 * before the click. */
export const PREFETCH_INTENT_MS = 90;

/** Hover intent for one row: a resting pointer or keyboard focus triggers the
 * callback once; a pointer that moves on before the delay does not. */
function usePrefetchIntent(onPrefetch?: () => void) {
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const latest = useRef(onPrefetch);
  latest.current = onPrefetch;
  useEffect(() => () => clearTimeout(timer.current), []);
  return useMemo(() => ({
    start: () => {
      clearTimeout(timer.current);
      timer.current = setTimeout(() => latest.current?.(), PREFETCH_INTENT_MS);
    },
    cancel: () => clearTimeout(timer.current),
    now: () => { clearTimeout(timer.current); latest.current?.(); },
  }), []);
}

function ChatRow({
  chat,
  active,
  indented,
  time,
  onClick,
  onArchive,
  onMention,
  onFork,
  onSaveAsWorkflow,
  forkLabel,
  onJumpToParent,
  liveAgents,
  unread,
  pullRequests,
  onPrefetch,
  onMarkRead,
  onMarkUnread,
}: {
  chat: Session;
  active: boolean;
  indented: boolean;
  time: string | null;
  onClick: () => void;
  /** Warm this chat's history before the click lands. */
  onPrefetch?: () => void;
  onArchive?: () => void;
  onMarkRead?: () => void;
  onMarkUnread?: () => void;
  onMention?: () => void;
  onFork?: () => void;
  onSaveAsWorkflow?: () => void;
  forkLabel?: string;
  onJumpToParent?: () => void;
  /** Live agents under this chat, counted by the host from every session. */
  liveAgents?: number;
  /** The strongest unread kind in this chat's tree, from the attention ledger. */
  unread?: AttentionKind;
  /** PRs linked to this chat; the row shows the most urgent one. */
  pullRequests?: SessionPullRequest[];
}) {
  const name = chatName(chat);
  // The hover names the chat in full: the ladder's longest rung, whatever width shows.
  const full = chat.titleLadder?.at(-1) ?? name;
  const detail = `${full} — ${harnessLabel(chat.harness)}${chat.model ? ` · ${chat.model}` : ""}`;
  const status = rowStatus(chat.status, liveAgents, unread);
  const pr = sidebarPrChip(pullRequests);
  const intent = usePrefetchIntent(active ? undefined : onPrefetch);
  return (
    <span className={cn(
      "group/row relative flex items-center rounded-[7px] transition-colors",
      active ? "bg-selection text-selection-foreground" : "hover:bg-accent",
    )}>
    <button
      type="button"
      draggable
      onDragStart={event => {
        event.dataTransfer.setData(SIDEBAR_CHAT_DRAG, chat.id);
        event.dataTransfer.effectAllowed = "copy";
      }}
      onClick={onClick}
      onPointerEnter={intent.start}
      onPointerLeave={intent.cancel}
      onFocus={intent.now}
      title={detail}
      aria-description="Drag into Mission Control to keep this chat in the grid"
      aria-current={active ? "page" : undefined}
      className={cn(
        "flex h-11 min-w-0 flex-1 items-center gap-2 rounded-[7px] text-left font-sans transition-colors active:scale-[0.99]",
        indented ? "pl-7" : "pl-2",
      )}
    >
      <span className="flex min-w-0 flex-1 flex-col justify-center gap-0.5">
        {unread && <span className="sr-only">Unread: </span>}
        <span className="flex min-w-0 items-center gap-1.5">
          {unread && <span className="size-1.5 shrink-0 rounded-full bg-foreground" aria-hidden="true" />}
          <FittedTitle
            title={name}
            ladder={chat.title ? chat.titleLadder : undefined}
            className={cn("min-w-0 flex-1 truncate text-[13px] leading-4 tracking-[-0.008em] text-foreground", (active || unread) && "font-medium")}
          />
        </span>
        {/* The fork origin is extra context, not a replacement for the status
            line: a fork still has to show whether it is working and when it
            last moved, like any other chat. */}
        <span className="flex min-w-0 items-center gap-1.5 truncate text-[11px] leading-3.5 tracking-[-0.004em] text-muted-foreground">
          {status && <><span className={cn("h-1.5 w-1.5 shrink-0 rounded-full", status.dot)} aria-hidden="true" /><span>{status.label}</span></>}
          {/* The PR comes before the time: on a narrow rail the time is what
              truncates, never the word saying whether the PR landed. The
              number lives in the hover and the screen-reader text. */}
          {pr && <>
            {status && <span aria-hidden="true">·</span>}
            <span data-sidebar-pr={pr.pr.number} title={pr.title} className="flex shrink-0 items-center gap-1">
              <PullRequestGlyph pr={pr.pr} size={11} />
              <span className="sr-only">Pull request #{pr.pr.number} </span>
              <span className={pr.tone === "danger" ? SHELF_TONE.danger : undefined}>{pr.text}</span>
              {pr.more > 0 && <span className="text-faint" aria-label={`and ${pr.more} more`}>+{pr.more}</span>}
            </span>
          </>}
          {/* Working plus a PR already fills the line, and a working chat is
              current anyway, so the time steps aside rather than clip. */}
          {time && !(status && pr) && <>
            {(status || pr) && <span aria-hidden="true">·</span>}
            <span className="shrink-0 tabular-nums text-faint">{time}</span>
          </>}
          {forkLabel && <>
            {(status || time || pr) && <span aria-hidden="true">·</span>}
            <GitFork size={9} strokeWidth={1.6} className="shrink-0 text-faint" aria-hidden="true" />
            <span className="truncate">forked from {forkLabel}</span>
          </>}
        </span>
      </span>
    </button>
    {/* Revealed on hover or keyboard focus: one three-dot menu holding every
        row action, so the row itself stays a single click target. The copy
        action puts the public `brio_…` alias on the clipboard — the token the
        composer recognizes — and the mention action drops the same alias
        straight into the open chat's draft. Sibling of the row button, never
        a descendant of it: a <button> inside a <button> is invalid HTML. */}
    <ChatRowMenu
      chat={chat}
      name={name}
      onMention={onMention}
      onFork={onFork}
      onSaveAsWorkflow={onSaveAsWorkflow}
      onArchive={onArchive}
      forkLabel={forkLabel}
      onJumpToParent={onJumpToParent}
      onMarkRead={unread ? onMarkRead : undefined}
      onMarkUnread={unread ? undefined : onMarkUnread}
    />
    <HarnessMark harness={chat.harness} size={13} className={cn("mr-2 shrink-0", !active && "text-muted-foreground")} />
    </span>
  );
}

const ROW_MENU_WIDTH = 224;
const ROW_MENU_HEIGHT_ESTIMATE = 300;

function ChatRowMenu({
  chat,
  name,
  onMention,
  onFork,
  onSaveAsWorkflow,
  onArchive,
  forkLabel,
  onJumpToParent,
  onMarkRead,
  onMarkUnread,
}: {
  chat: Session;
  name: string;
  onMention?: () => void;
  onFork?: () => void;
  onSaveAsWorkflow?: () => void;
  onArchive?: () => void;
  forkLabel?: string;
  onJumpToParent?: () => void;
  /** Offered while the chat is unread; clears its dot. */
  onMarkRead?: () => void;
  /** Offered while it is not; brings the dot back until you open it. */
  onMarkUnread?: () => void;
}) {
  const menu = useMenuPanel<HTMLButtonElement>({ width: ROW_MENU_WIDTH, height: ROW_MENU_HEIGHT_ESTIMATE });
  const alias = toPublicAlias(chat.id);
  const item = "flex h-7 w-full items-center gap-2 rounded-md px-2 text-left text-[13px] transition-colors hover:bg-accent";
  const run = (action: () => void) => () => { action(); menu.close(); };
  return (
    <>
      <button
        ref={menu.triggerRef}
        type="button"
        onClick={event => { event.stopPropagation(); menu.toggle(); }}
        title={`Chat actions for ${name}`}
        aria-label={`Chat actions for ${name}`}
        aria-haspopup="menu"
        aria-expanded={menu.open}
        className={cn(
          "mr-1 grid size-7 shrink-0 place-items-center rounded-md text-muted-foreground transition-opacity hover:bg-accent hover:text-foreground focus-visible:opacity-100 group-hover/row:opacity-100",
          menu.open ? "opacity-100" : "opacity-0",
        )}
      >
        <MoreHorizontal size={13} strokeWidth={1.7} aria-hidden="true" />
      </button>
      <MenuPanel controller={menu} label={`Chat actions for ${name}`}>
        {onMarkRead && (
          <button type="button" role="menuitem" className={item} onClick={run(onMarkRead)}>
            <CheckCheck size={12} strokeWidth={1.7} className="shrink-0 text-muted-foreground" aria-hidden="true" />
            <span className="min-w-0 flex-1 truncate">Mark as read</span>
          </button>
        )}
        {onMarkUnread && (
          <button type="button" role="menuitem" className={item} onClick={run(onMarkUnread)}>
            <CircleDot size={12} strokeWidth={1.7} className="shrink-0 text-muted-foreground" aria-hidden="true" />
            <span className="min-w-0 flex-1 truncate">Mark as unread</span>
          </button>
        )}
        {(onMarkRead || onMarkUnread) && <MenuSeparator />}
        <button type="button" role="menuitem" className={item} onClick={run(() => { void navigator.clipboard?.writeText(alias); })}>
          <Copy size={12} strokeWidth={1.7} className="shrink-0 text-muted-foreground" aria-hidden="true" />
          <span className="min-w-0 flex-1 truncate">Copy chat ID</span>
          <span className="shrink-0 font-mono text-[11px] text-muted-foreground">{alias}</span>
        </button>
        {onMention && (
          <button type="button" role="menuitem" className={item} onClick={run(onMention)}>
            <AtSign size={12} strokeWidth={1.7} className="shrink-0 text-muted-foreground" aria-hidden="true" />
            <span className="min-w-0 flex-1 truncate">Mention in current chat</span>
          </button>
        )}
        {onFork && (
          <button type="button" role="menuitem" className={item} onClick={run(onFork)}>
            <GitFork size={12} strokeWidth={1.7} className="shrink-0 text-muted-foreground" aria-hidden="true" />
            <span className="min-w-0 flex-1 truncate">Fork chat…</span>
          </button>
        )}
        {onSaveAsWorkflow && (
          <button type="button" role="menuitem" className={item} onClick={run(onSaveAsWorkflow)}>
            <WorkflowIcon size={12} strokeWidth={1.7} className="shrink-0 text-muted-foreground" aria-hidden="true" />
            <span className="min-w-0 flex-1 truncate">Save as workflow…</span>
          </button>
        )}
        {onJumpToParent && forkLabel && (
          <button type="button" role="menuitem" className={item} onClick={run(onJumpToParent)}>
            <GitFork size={12} strokeWidth={1.7} className="shrink-0 text-muted-foreground" aria-hidden="true" />
            <span className="min-w-0 flex-1 truncate">Open parent: {forkLabel}</span>
          </button>
        )}
        {chat.kind !== "worker" && <ChatNoticeLevel chatId={chat.id} item={item} />}
        {onArchive && (
          <>
            <MenuSeparator />
            <button type="button" role="menuitem" aria-label={`Archive ${name}`} className={item} onClick={run(onArchive)}>
              <Archive size={12} strokeWidth={1.7} className="shrink-0 text-muted-foreground" aria-hidden="true" />
              <span className="min-w-0 flex-1 truncate">Archive</span>
            </button>
          </>
        )}
      </MenuPanel>
    </>
  );
}

/** Per-orchestrator override of which worker notices cost it a model turn. */
function ChatNoticeLevel({ chatId, item }: { chatId: string; item: string }) {
  const { view, set } = useDelegationNotify(chatId);
  return (
    <>
      <MenuSeparator />
      <div className="px-2 pb-0.5 pt-1 text-[11px] text-muted-foreground">Worker notices</div>
      {NOTIFY_LEVELS.map(option => {
        const current = view?.level === option.value;
        const inherited = current && view?.sessionOverride == null;
        return (
          <button
            key={option.value}
            type="button"
            role="menuitemradio"
            aria-checked={current}
            disabled={!view}
            className={item}
            title={option.description}
            onClick={() => set(inherited ? null : option.value)}
          >
            <Check size={12} strokeWidth={1.7} className={cn("shrink-0", current ? "text-foreground" : "opacity-0")} aria-hidden="true" />
            <span className="min-w-0 flex-1 truncate">{option.label}</span>
            {inherited && <span className="shrink-0 text-[11px] text-muted-foreground">default</span>}
          </button>
        );
      })}
    </>
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

function ActionRow({
  icon: Icon,
  label,
  onClick,
  active = false,
  disabled = false,
  chord,
  badge,
}: {
  icon: LucideIcon;
  label: string;
  onClick: () => void;
  active?: boolean;
  disabled?: boolean;
  /** A count of things waiting on the person, read after the label. */
  badge?: { count: string; label: string };
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
      aria-label={badge ? `${label}, ${badge.label}` : label}
      aria-current={active ? "page" : undefined}
      className={cn(
        "flex h-8 w-full items-center gap-2.5 rounded-lg px-2 text-[13px] tracking-[-0.008em] transition-colors",
        active ? "bg-selection font-medium text-selection-foreground" : "text-foreground/85 hover:bg-accent hover:text-foreground",
        disabled && "cursor-default opacity-50 hover:bg-transparent hover:text-foreground/85",
      )}
    >
      <Icon size={15} strokeWidth={1.5} className="shrink-0 text-muted-foreground" aria-hidden="true" />
      {label}
      {badge && <span aria-hidden="true" className="ml-auto rounded-full bg-foreground/10 px-1.5 text-caption tabular-nums text-muted-foreground">{badge.count}</span>}
    </button>
  );
}


export type BridgeSidebarProps = {
  /** Every top-level session. The rail derives both the project tree and the
   * history from this one list, so a chat cannot be visible in one and missing
   * from the other. */
  chats: Session[];
  /** Each chat's live descendant agents. Workers never reach `chats`, so the
   * host counts them from every session and hands the answer in. */
  liveAgents?: LiveAgents;
  /** Top-level chat id to the strongest unread kind in its tree
   * (`attention/unread_summary`'s `byChat`). */
  unread?: Readonly<Record<string, AttentionKind>>;
  /** Session id to the pull requests linked to that chat. */
  pullRequests?: SidebarPullRequests;
  /** Only for `Group by → Project` labels; the tree itself lives on the projects
   * screen now. */
  workspaces: Workspace[];
  activeSessionId?: string;
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
  /** The Projects header's "New folder" action. */
  onOpenProjects: () => void;
  /** The Apps row opens the launcher, where every app lives. */
  onOpenApps: () => void;
  /** Everything every app is waiting on, summed. Hidden when absent. */
  appsBadge?: BadgeText;
  /** Apps the person pinned, in pin order, drawn as rows under Apps. */
  pinnedApps?: readonly { id: AppId; badge?: BadgeText }[];
  /** The app on screen, so its pin reads as current. */
  activeApp?: AppId | null;
  onOpenApp: (id: AppId) => void;
  /** Drawn at the end of the bottom rail, after Settings: the usage dot. */
  railTrailing?: React.ReactNode;
  onOpenSettings: () => void;
  onOpenSession: (id: string) => void;
  /** The pointer is resting on (or focus reached) a chat: warm its history. */
  onPrefetchSession?: (id: string) => void;
  /** Open the search field on `query` and search deeper, as `/find` does.
   * A new `nonce` repeats the request for the same query. */
  searchRequest?: { query: string; nonce: number };
  /** Absent when the host cannot archive — the row then shows no action. */
  onArchiveChat?: (chat: Session) => void;
  /** Clear a chat's unread dot by hand, workers' rows included. */
  onMarkChatRead?: (chatId: string) => void;
  /** Bring a chat's unread dot back by hand. */
  onMarkChatUnread?: (chatId: string) => void;
  /** Drop `@session:<alias>` for this chat into the open chat's draft. */
  onMentionChat?: (chat: Session) => void;
  /** Open the fork dialog at this chat's head. */
  onForkChat?: (chat: Session) => void;
  /** Draft a workflow from this chat and open it in the Workflows page. */
  onSaveChatAsWorkflow?: (chat: Session) => void;
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
  liveAgents,
  unread,
  pullRequests,
  workspaces,
  activeSessionId,
  settingsActive,
  accountName,
  newChatBusy = false,
  mobileOpen = false,
  onCloseMobile,
  onOpenNewChat,
  onNewChatInProject,
  onOpenProjects,
  onOpenApps,
  appsBadge,
  pinnedApps = [],
  activeApp,
  onOpenApp,
  railTrailing,
  onOpenSettings,
  onOpenSession,
  onPrefetchSession,
  searchRequest,
  onArchiveChat,
  onMarkChatRead,
  onMarkChatUnread,
  onMentionChat,
  onForkChat,
  onSaveChatAsWorkflow,
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

  // Content search across every chat, beside the title filter. Only while
  // the field is open: a closed field must not keep a deep search alive.
  const chatSearch = useChatSearch(searchOpen ? query : "");
  const { runDeep } = chatSearch;
  const [pendingDeep, setPendingDeep] = useState<string | null>(null);
  useEffect(() => {
    if (!searchRequest) return;
    setSearchOpen(true);
    setQuery(searchRequest.query);
    setPendingDeep(searchRequest.query.trim());
  }, [searchRequest]);
  // `/find` asks for the deep stage once the open field holds its query.
  useEffect(() => {
    if (pendingDeep === null || !searchOpen || pendingDeep !== query.trim()) return;
    setPendingDeep(null);
    runDeep();
  }, [pendingDeep, searchOpen, query, runDeep]);

  const onSearchEnter = useCallback(() => {
    const trimmed = query.trim();
    if (trimmed.length < CHAT_SEARCH_MIN_CHARS) return;
    const current = chatSearch.result?.query === trimmed && !chatSearch.loading ? chatSearch.result : undefined;
    // No fresh index answer yet: ask for the deep one, which the host skips
    // the model for when the index turns out to be sure.
    const action = current ? enterAction(current, chatSearch.deepRunning) : chatSearch.deepRunning ? "none" : "deep";
    if (action === "deep") runDeep();
    if (action === "open" && current?.hits[0]) {
      onOpenSession(current.hits[0].sessionId);
      closeSearch();
    }
  }, [query, chatSearch.result, chatSearch.loading, chatSearch.deepRunning, runDeep, onOpenSession, closeSearch]);

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

  const toggleFold = useCallback((key: string) => setFoldedGroups(current => toggleKey(current, key)), []);
  const toggleShownInFull = useCallback((key: string) => setShownInFull(current => toggleKey(current, key)), []);

  const agents = useMemo(() => agentOptions(chats), [chats]);
  const chatsById = useMemo(() => new Map(chats.map(chat => [chat.id, chat])), [chats]);
  const workspaceTitle = useMemo(() => {
    const titles = new Map(workspaces.map(workspace => [workspace.id, workspace.title]));
    return (id: string | null | undefined) => (id ? titles.get(id) : undefined);
  }, [workspaces]);
  const visible = useMemo(
    () => filterChats(chats, { query, status: view.status, agent: view.agent, workspaceTitle, liveAgents }),
    [chats, query, view.status, view.agent, workspaceTitle, liveAgents],
  );
  const groups = useMemo(
    () => groupChats(visible, { groupBy: view.groupBy, sortBy: view.sortBy, workspaces, now, liveAgents }),
    [visible, view.groupBy, view.sortBy, workspaces, now, liveAgents],
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
                onKeyDown={event => {
                  if (event.key === "Escape") closeSearch();
                  if (event.key === "Enter" && !event.nativeEvent.isComposing) {
                    event.preventDefault();
                    onSearchEnter();
                  }
                }}
                placeholder="Search chats and what was said…"
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

        {/* One row for every app, then the few the person pinned. Everything
            else lives in the launcher, so a new app costs the sidebar nothing. */}
        <nav aria-label="Main navigation" className="mb-4 shrink-0 space-y-0.5">
          <ActionRow icon={Blocks} label="Apps" chord="open-launcher" onClick={onOpenApps} badge={appsBadge} />
          {pinnedApps.map(pin => {
            const app = appById(pin.id);
            return <ActionRow key={app.id} icon={app.icon} label={app.name} onClick={() => onOpenApp(app.id)} active={activeApp === app.id} badge={pin.badge} />;
          })}
        </nav>

        <div className="-mr-2 min-h-0 flex-1 overflow-y-auto pr-2">
          {searchOpen && query.trim().length >= CHAT_SEARCH_MIN_CHARS && (
            <ChatSearchResults
              result={chatSearch.result}
              loading={chatSearch.loading}
              deepRunning={chatSearch.deepRunning}
              error={chatSearch.error}
              activeSessionId={activeSessionId}
              now={now}
              onOpen={id => { onOpenSession(id); closeSearch(); }}
            />
          )}
          <SectionLabel action={
            <span className="flex items-center gap-0.5">
              <SidebarFilterMenu view={view} agents={agents} allowProjectGrouping onChange={changeView} />
              <RailIconButton label="New folder" onClick={onOpenProjects}>
                <FolderPlus size={13} strokeWidth={1.5} aria-hidden="true" />
              </RailIconButton>
            </span>
          }>
            Projects
          </SectionLabel>

          {groups.map(group => {
            // Capped while searching too: a one-letter query matches nearly
            // every chat, and mounting all of them froze the field.
            const overCap = group.chats.length > GROUP_ROW_CAP;
            const capped = overCap && !shownInFull.has(group.key);
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
                {rows.map(chat => {
                // `forkParentSessionId`, not `parentSessionId`: the latter
                // names a delegated worker, and workers never reach this list.
                const forkedFrom = chat.forkParentSessionId;
                const source = forkedFrom ? chatsById.get(forkedFrom) : undefined;
                return (
                  <ChatRow
                    key={chat.id}
                    chat={chat}
                    active={chat.id === activeSessionId}
                    indented={!!group.label}
                    time={chatListTime(chatTimestamp(chat), now)}
                    onClick={() => onOpenSession(chat.id)}
                    onPrefetch={onPrefetchSession && (() => onPrefetchSession(chat.id))}
                    onArchive={onArchiveChat && (() => onArchiveChat(chat))}
                    onMention={onMentionChat && (() => onMentionChat(chat))}
                    onFork={onForkChat && chat.kind !== "worker" ? () => onForkChat(chat) : undefined}
                    onSaveAsWorkflow={onSaveChatAsWorkflow && chat.kind !== "worker" ? () => onSaveChatAsWorkflow(chat) : undefined}
                    forkLabel={forkedFrom ? source?.label ?? "session" : undefined}
                    onJumpToParent={forkedFrom ? () => onOpenSession(forkedFrom) : undefined}
                    liveAgents={liveAgents?.get(chat.id)?.length}
                    unread={unread?.[chat.id]}
                    pullRequests={pullRequests?.[chat.id]}
                    onMarkRead={onMarkChatRead && (() => onMarkChatRead(chat.id))}
                    onMarkUnread={onMarkChatUnread && (() => onMarkChatUnread(chat.id))}
                  />
                );
              })}
                {overCap && !folded && (
                  <button
                    type="button"
                    aria-expanded={!capped}
                    onClick={() => toggleShownInFull(group.key)}
                    className={cn(
                      "flex h-6 w-full items-center rounded-md text-left text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground",
                      group.label ? "pl-7" : "px-2",
                    )}
                  >
                    {capped ? `Show ${group.chats.length - GROUP_ROW_CAP} more` : "Show less"}
                  </button>
                )}
              </div>
            );
          })}
          {!visible.length && !(searching && chatSearch.result?.hits.length) && (
            <p className="px-2 py-1 text-[11px] leading-relaxed text-muted-foreground/70">
              {chats.length ? "No chat matches this filter." : "No chats yet. New Chat opens in the repo you were last in."}
            </p>
          )}
        </div>

        {/* A rail pinned to the bottom: the account's settings entry and the
            usage dot. */}
        <div className="mt-1 flex shrink-0 items-center gap-0.5 border-t border-sidebar-border pt-1.5">
          <button type="button" onClick={onOpenSettings} aria-label={`Open settings for ${accountName}`} aria-current={settingsActive ? "page" : undefined} title={`Open settings for ${accountName}`} className={cn("flex h-8 min-w-0 flex-1 items-center gap-2 rounded-lg px-2 text-[12px] transition-colors", settingsActive ? "bg-selection text-selection-foreground" : "text-muted-foreground hover:bg-accent hover:text-foreground")}><Settings2 size={16} strokeWidth={1.6} aria-hidden="true" /><span className="truncate">Settings</span></button>
          {/* The usage dot: a quick look at account quota, with the Usage app
              for the full screen. */}
          {railTrailing}
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
