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
  | "compaction" | "branch-summary" | "raw";

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
   * The tool call this row describes, read once at ingestion rather than
   * re-derived on every render. Present on rows that came from a tool
   * lifecycle; `toolCallDisplay` falls back to deriving one for items built by
   * hand.
   */
  tool?: ToolCallDisplay;
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
