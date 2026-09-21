/**
 * What the transcript draws: one reduced conversation item.
 *
 * The shape the cards have always read. It lives here rather than in
 * `src/conversation.ts` so the reducer can produce it without importing the
 * module that wraps the reducer.
 */

import type { ToolCallDisplay } from "./toolCall";

export type ConversationItemType =
  | "message" | "reasoning" | "activity" | "plan" | "approval" | "permission"
  | "question" | "error" | "diff" | "artifact" | "delegation" | "checkpoint"
  | "compaction" | "context-compacted" | "branch-summary" | "model-change"
  | "raw";

export interface ConversationItem {
  key: string;
  type: ConversationItemType;
  eventId: number;
  role?: string;
  status?: string;
  title?: string;
  text: string;
  data: Record<string, unknown>;
  sequence: number;
  /**
   * Which turn produced this row: 1 for the first, 0 for anything the reducer
   * saw before a boundary. Stamped at creation, so a tool call belongs to the
   * turn it started in whatever arrives afterwards.
   *
   * Derived from what *both* projections can see. A turn-marker frame is
   * transient — it carries sequence zero and the durable writer refuses to
   * store it — so a replay would count a different number of turns than the
   * live window if the index came from those alone. See `reduceTranscript`.
   */
  turn: number;
  entryId?: string;
  /**
   * When the item was first seen — the timestamp of the event that created it,
   * i.e. a tool call's start. Held so a run's trailer can report a wall-clock
   * span (union of call windows) rather than a sum of overlapping durations.
   */
  createdAt?: string;
  /**
   * Provider item id from the live event (`AgentEvent.itemId`) or the durable
   * payload.
   */
  itemId?: string;
  /**
   * The identity this item shares with its counterpart on the other
   * projection (live vs durable), when it has one. `eventId` is not it: on
   * the live side it is the session-event table's own autoincrement id, and
   * on the durable side it is the forest entry's sequence — two unrelated
   * numbering spaces that happen to collide by coincidence, not by design.
   * Prefer this field over `eventId` for any merge that needs to recognize a
   * streamed item and its persisted twin as the same logical thing. See
   * `itemIdentity`.
   */
  identity?: string;
  /**
   * The runtime that produced the frame this row was created from, when the
   * backend stamped one. Held per row, not per session: a chat switched from
   * Codex to OpenCode still contains the Codex rows it was switched away from,
   * and they must not be re-attributed to the harness now selected.
   */
  harness?: string;
  /**
   * The tool call this row describes, read once at ingestion rather than
   * re-derived on every render. Present on rows that came from a tool
   * lifecycle; `toolCallDisplay` falls back to deriving one for items built by
   * hand.
   */
  tool?: ToolCallDisplay;
}

/**
 * A cheap stand-in for "this row still says the same thing".
 *
 * Reference equality would be the natural test and is worthless here: the
 * reducer is a fold that rebuilds every item on every run, so a live turn hands
 * the transcript a hundred brand-new objects twenty times a second even when
 * ninety-nine of them are unchanged. What actually moves is one of these
 * fields — a status, a length of streamed text, the event id every frame
 * stamps onto the row it lands on — so a memo keyed on them re-renders exactly
 * the rows a frame touched.
 */
export function itemSignature(item: ConversationItem): string {
  return [
    item.identity ?? item.key,
    item.type,
    item.status ?? "",
    item.harness ?? "",
    item.text.length,
    item.title ?? "",
    // Every frame the reducer folds into a row restamps this, which is what
    // makes it a change detector for payloads the fields above cannot see —
    // a plan's steps, a diffstat, an exit code.
    item.eventId,
    item.sequence,
    item.turn,
    item.tool?.status ?? "",
  ].join("\u0000");
}

/**
 * The subagent session a row came from, when the backend stamped one.
 *
 * OpenCode's `task` tool runs a real child session. Its rows land in the
 * parent transcript with `data.subagent`, and a reader needs to see that a
 * tool call or a paragraph is the subagent's work, not the parent's.
 */
export interface SubagentSource {
  sessionId: string;
  agent?: string;
  title?: string;
}

export function subagentSource(item: Pick<ConversationItem, "data">): SubagentSource | undefined {
  const raw = item.data.subagent;
  if (!raw || typeof raw !== "object") return undefined;
  const record = raw as Record<string, unknown>;
  if (typeof record.sessionId !== "string" || record.sessionId === "") return undefined;
  return {
    sessionId: record.sessionId,
    agent: typeof record.agent === "string" && record.agent !== "" ? record.agent : undefined,
    title: typeof record.title === "string" && record.title !== "" ? record.title : undefined,
  };
}

/** What to call the subagent in a row label: its agent name, else its task title, else the generic word. */
export function subagentLabel(item: Pick<ConversationItem, "data">): string | undefined {
  const source = subagentSource(item);
  if (!source) return undefined;
  return source.agent ?? source.title ?? "subagent";
}

/** Whether two rows are the same row, saying the same thing. */
export function sameItem(left: ConversationItem, right: ConversationItem): boolean {
  return left === right || itemSignature(left) === itemSignature(right);
}

/** The same, for the list a group draws. */
export function sameItems(left: readonly ConversationItem[], right: readonly ConversationItem[]): boolean {
  if (left === right) return true;
  if (left.length !== right.length) return false;
  return left.every((item, index) => sameItem(item, right[index]));
}

/**
 * Domain id keys, tried in priority order, that identify one logical item —
 * a tool call, an approval, a permission, a question — the same way whether
 * it is read off the live event stream or off a persisted forest entry. Both
 * sides carry these inside their own `data`/`payload` bag (the backend
 * writes the same JSON body to both the session-event row and, once it goes
 * durable, the forest entry's payload); the autoincrement ids each store
 * assigns around that body do not agree, and were never meant to.
 */
const IDENTITY_KEYS = ["itemId", "approvalId", "requestId", "questionId"] as const;

/**
 * The identity a conversation item shares with its counterpart on the other
 * projection, falling back to something still unique — but not
 * cross-projection-stable — when the item carries none of the known domain
 * ids. The reducer populates `item.identity` with this before returning, so a
 * caller merging live and durable lists never has to reach for `eventId`.
 */
export function itemIdentity(item: Pick<ConversationItem, "data" | "entryId" | "eventId" | "type" | "itemId">): string {
  if (typeof item.itemId === "string" && item.itemId) return item.itemId;
  for (const key of IDENTITY_KEYS) {
    const value = item.data[key];
    if (typeof value === "string" && value) return value;
  }
  // A durable-only card (checkpoint, compaction, branch summary) has no live
  // twin to line up with, so the entry id is unique enough. A live-only item
  // with none of the above falls back to its own event id.
  return item.entryId ? `entry:${item.entryId}` : `${item.type}:${item.eventId}`;
}
