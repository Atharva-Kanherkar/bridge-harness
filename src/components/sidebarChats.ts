import type { Session, SessionStatus, Workspace } from "../types";
import { harnessLabel } from "../utils";

// How the rail's history list is filtered, grouped and ordered. The rail holds
// dozens of same-named orchestrators, so finding one is a search-and-group
// problem, not a scrolling problem — this module owns that logic so the
// component stays presentation.

/** The session kind a briefing run uses. Mirrors
 * `bridge_core::work_briefing_config::BRIEFING_SESSION_KIND`. */
export const BRIEFING_SESSION_KIND = "briefing";
/** Mirrors `bridge_core::memory_extraction::EXTRACTION_SESSION_KIND`. */
export const EXTRACTION_SESSION_KIND = "extraction";

/** The session kind the composer typeahead's hidden warm session uses. Mirrors
 * `bridge_core::suggestion_engine::SUGGESTION_SESSION_KIND`. */
export const SUGGESTION_SESSION_KIND = "suggestion";

/** The session kind a bounded outcome evaluation runs in. Mirrors
 * `bridge_core::routing_evaluation::EVALUATION_SESSION_KIND`. */
export const EVALUATION_SESSION_KIND = "outcome_evaluation";

/** The session kind a memory consolidation run uses. Mirrors
 * `bridge_core::memory_consolidation::CONSOLIDATION_SESSION_KIND`. */
export const CONSOLIDATION_SESSION_KIND = "consolidation";

/** Is this a session Bridge runs for itself, that a human should never meet in a list?
 *
 * A predicate rather than an ordering rule: a run that merely sorted last would
 * still be one keystroke from being opened, resumed, or sent a turn. Its
 * transcript exists to be inspected after a run, not joined during one. */
const HIDDEN_SESSION_KINDS = [
  BRIEFING_SESSION_KIND,
  SUGGESTION_SESSION_KIND,
  EXTRACTION_SESSION_KIND,
  EVALUATION_SESSION_KIND,
  CONSOLIDATION_SESSION_KIND,
];

export function isHiddenSession(chat: Pick<Session, "kind">): boolean {
  return !!chat.kind && HIDDEN_SESSION_KINDS.includes(chat.kind);
}

/** Everything a human may see, from everything Bridge is running. */
export function visibleChats(chats: Session[]): Session[] {
  return chats.filter(chat => !isHiddenSession(chat));
}

export type ChatGroupBy = "date" | "project" | "agent" | "status" | "none";
export type ChatSortBy = "recency" | "name";
export type ChatStatusFilter = "all" | "active" | "waiting" | "failed";

export type ChatView = {
  status: ChatStatusFilter;
  /** A harness id, or "all". */
  agent: string;
  groupBy: ChatGroupBy;
  sortBy: ChatSortBy;
};

export type ChatGroup = { key: string; label: string; chats: Session[] };

export const CHAT_VIEW_KEY = "bridge.sidebar.chatView";

export const DEFAULT_CHAT_VIEW: ChatView = { status: "all", agent: "all", groupBy: "project", sortBy: "recency" };

/** Rows shown per group before a "Show more" control takes over. */
export const GROUP_ROW_CAP = 12;

const GROUP_BY_VALUES: ChatGroupBy[] = ["date", "project", "agent", "status", "none"];
const SORT_BY_VALUES: ChatSortBy[] = ["recency", "name"];
const STATUS_VALUES: ChatStatusFilter[] = ["all", "active", "waiting", "failed"];

export const CHAT_GROUP_BY_LABELS: Record<ChatGroupBy, string> = {
  date: "Date",
  project: "Project",
  agent: "Agent",
  status: "Status",
  none: "None",
};

export const CHAT_SORT_BY_LABELS: Record<ChatSortBy, string> = {
  recency: "Recency",
  name: "Name",
};

export const CHAT_STATUS_LABELS: Record<ChatStatusFilter, string> = {
  all: "All",
  active: "Active",
  waiting: "Waiting",
  failed: "Failed",
};

export type StatusBucket = "active" | "waiting" | "failed" | "idle";

const STATUS_BUCKETS: Record<StatusBucket, SessionStatus[]> = {
  active: ["working", "starting", "resuming", "restored", "checkpointing", "warm"],
  waiting: ["waiting"],
  failed: ["failed"],
  idle: [],
};

const STATUS_BUCKET_ORDER: StatusBucket[] = ["active", "waiting", "failed", "idle"];

const STATUS_BUCKET_LABELS: Record<StatusBucket, string> = {
  active: "Active",
  waiting: "Waiting on you",
  failed: "Failed",
  idle: "Idle",
};

export function statusBucket(status: SessionStatus): StatusBucket {
  for (const bucket of STATUS_BUCKET_ORDER) {
    if (STATUS_BUCKETS[bucket].includes(status)) return bucket;
  }
  return "idle";
}

export function chatName(chat: Session): string {
  return chat.title || chat.label;
}

/** Session carries no last-activity field, so the day a chat sorts under is when
 * it started, or when it ended for a session that never recorded a start. */
export function chatTimestamp(chat: Session): number | null {
  const raw = chat.startedAt ?? chat.endedAt;
  if (!raw) return null;
  const value = Date.parse(raw);
  return Number.isFinite(value) ? value : null;
}

/** Compact age for a chat row. Null means the row should omit the time entirely. */
export function chatListTime(at: number | null, now: number): string | null {
  if (at === null || !Number.isFinite(at)) return null;
  const seconds = Math.max(0, Math.floor((now - at) / 1000));
  if (seconds < 60) return "now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h`;
  return `${Math.floor(hours / 24)}d`;
}

function startOfDay(value: number): number {
  const date = new Date(value);
  date.setHours(0, 0, 0, 0);
  return date.getTime();
}

const monthDay = new Intl.DateTimeFormat("en-US", { month: "short", day: "numeric" });
const monthDayYear = new Intl.DateTimeFormat("en-US", { month: "short", day: "numeric", year: "numeric" });

export function dayLabel(value: number, now: number): string {
  const day = startOfDay(value);
  const today = startOfDay(now);
  if (day === today) return "Today";
  // Decrement the calendar date rather than subtracting 24h, so the day either
  // side of a DST shift is still labelled "Yesterday".
  const yesterday = new Date(today);
  yesterday.setDate(yesterday.getDate() - 1);
  if (day === yesterday.getTime()) return "Yesterday";
  const sameYear = new Date(day).getFullYear() === new Date(today).getFullYear();
  return (sameYear ? monthDay : monthDayYear).format(new Date(day));
}

export function agentOptions(chats: Session[]): { id: string; label: string }[] {
  const seen = new Map<string, string>();
  for (const chat of chats) {
    if (!seen.has(chat.harness)) seen.set(chat.harness, harnessLabel(chat.harness));
  }
  return [...seen.entries()]
    .map(([id, label]) => ({ id, label }))
    .sort((a, b) => a.label.localeCompare(b.label));
}

export function filterChats(
  chats: Session[],
  {
    query = "",
    status = "all",
    agent = "all",
    workspaceTitle,
  }: {
    query?: string;
    status?: ChatStatusFilter;
    agent?: string;
    /** Resolves a chat's project name into the search haystack, so looking up a
     * project by name surfaces its chats instead of an empty project. */
    workspaceTitle?: (workspaceId: string | null | undefined) => string | undefined;
  } = {},
): Session[] {
  const needle = query.trim().toLowerCase();
  return chats.filter(chat => {
    if (status !== "all" && statusBucket(chat.status) !== status) return false;
    if (agent !== "all" && chat.harness !== agent) return false;
    if (!needle) return true;
    const project = workspaceTitle?.(chat.workspaceId) ?? "";
    const haystack = `${chatName(chat)} ${chat.label} ${harnessLabel(chat.harness)} ${chat.model ?? ""} ${project}`.toLowerCase();
    return haystack.includes(needle);
  });
}

function byRecency(a: Session, b: Session): number {
  const left = chatTimestamp(a);
  const right = chatTimestamp(b);
  if (left === null && right === null) return 0;
  // An undated chat has no place in a newest-first order, so it sinks.
  if (left === null) return 1;
  if (right === null) return -1;
  return right - left;
}

function byName(a: Session, b: Session): number {
  return chatName(a).localeCompare(chatName(b));
}

function sortChats(chats: Session[], sortBy: ChatSortBy): Session[] {
  return [...chats].sort(sortBy === "name" ? byName : byRecency);
}

function bucketBy(chats: Session[], key: (chat: Session) => string): Map<string, Session[]> {
  const buckets = new Map<string, Session[]>();
  for (const chat of chats) {
    const id = key(chat);
    const bucket = buckets.get(id);
    if (bucket) bucket.push(chat);
    else buckets.set(id, [chat]);
  }
  return buckets;
}

function mostRecent(chats: Session[]): number {
  return chats.reduce((latest, chat) => Math.max(latest, chatTimestamp(chat) ?? Number.NEGATIVE_INFINITY), Number.NEGATIVE_INFINITY);
}

export const NO_PROJECT_GROUP_KEY = "__no_project__";
const UNDATED_KEY = "__earlier__";

export function groupChats(
  chats: Session[],
  {
    groupBy,
    sortBy,
    workspaces = [],
    now,
  }: { groupBy: ChatGroupBy; sortBy: ChatSortBy; workspaces?: Workspace[]; now: number },
): ChatGroup[] {
  if (groupBy === "none") return [{ key: "all", label: "", chats: sortChats(chats, sortBy) }];

  if (groupBy === "date") {
    const buckets = bucketBy(chats, chat => {
      const at = chatTimestamp(chat);
      return at === null ? UNDATED_KEY : String(startOfDay(at));
    });
    const days = [...buckets.keys()]
      .filter(key => key !== UNDATED_KEY)
      .sort((a, b) => Number(b) - Number(a))
      .map(key => ({ key, label: dayLabel(Number(key), now), chats: sortChats(buckets.get(key)!, sortBy) }));
    const undated = buckets.get(UNDATED_KEY);
    return undated ? [...days, { key: UNDATED_KEY, label: "Earlier", chats: sortChats(undated, sortBy) }] : days;
  }

  if (groupBy === "project") {
    const titles = new Map(workspaces.map(workspace => [workspace.id, workspace.title]));
    const buckets = bucketBy(chats, chat => chat.workspaceId ?? NO_PROJECT_GROUP_KEY);
    const projects = [...buckets.keys()]
      .filter(key => key !== NO_PROJECT_GROUP_KEY)
      .map(key => ({ key, label: titles.get(key) ?? "Unknown project", chats: sortChats(buckets.get(key)!, sortBy) }));
    projects.sort((a, b) => (sortBy === "name" ? a.label.localeCompare(b.label) : mostRecent(b.chats) - mostRecent(a.chats)));
    const loose = buckets.get(NO_PROJECT_GROUP_KEY);
    return loose ? [...projects, { key: NO_PROJECT_GROUP_KEY, label: "No project", chats: sortChats(loose, sortBy) }] : projects;
  }

  if (groupBy === "agent") {
    const buckets = bucketBy(chats, chat => chat.harness);
    return [...buckets.entries()]
      .map(([key, group]) => ({ key, label: harnessLabel(key), chats: sortChats(group, sortBy) }))
      .sort((a, b) => b.chats.length - a.chats.length || a.label.localeCompare(b.label));
  }

  const buckets = bucketBy(chats, chat => statusBucket(chat.status));
  return STATUS_BUCKET_ORDER.filter(bucket => buckets.has(bucket)).map(bucket => ({
    key: bucket,
    label: STATUS_BUCKET_LABELS[bucket],
    chats: sortChats(buckets.get(bucket)!, sortBy),
  }));
}

function oneOf<T extends string>(value: unknown, allowed: T[], fallback: T): T {
  return typeof value === "string" && (allowed as string[]).includes(value) ? (value as T) : fallback;
}

/** Reads the persisted view, falling back per field so one bad value cannot cost
 * the others. */
export function readChatView(): ChatView {
  if (typeof localStorage === "undefined") return DEFAULT_CHAT_VIEW;
  let stored: unknown;
  try {
    stored = JSON.parse(localStorage.getItem(CHAT_VIEW_KEY) ?? "");
  } catch {
    return DEFAULT_CHAT_VIEW;
  }
  if (!stored || typeof stored !== "object") return DEFAULT_CHAT_VIEW;
  const raw = stored as Record<string, unknown>;
  return {
    status: oneOf(raw.status, STATUS_VALUES, DEFAULT_CHAT_VIEW.status),
    agent: typeof raw.agent === "string" && raw.agent ? raw.agent : DEFAULT_CHAT_VIEW.agent,
    groupBy: oneOf(raw.groupBy, GROUP_BY_VALUES, DEFAULT_CHAT_VIEW.groupBy),
    sortBy: oneOf(raw.sortBy, SORT_BY_VALUES, DEFAULT_CHAT_VIEW.sortBy),
  };
}

export function writeChatView(view: ChatView): void {
  if (typeof localStorage === "undefined") return;
  localStorage.setItem(CHAT_VIEW_KEY, JSON.stringify(view));
}
