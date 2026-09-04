/**
 * The conversation projections, as the app has always called them.
 *
 * Both are now the same two steps — normalize, then reduce — over the typed
 * union in `src/transcript/`. This module is the seam that keeps `App.tsx` and
 * `AgentConversation.tsx` call sites unchanged, plus the item-level folds
 * (worker delegation panels, live/durable merge) that operate on reduced items
 * rather than on events.
 */

import { normalizeAgentEvent, normalizeSessionEntry } from "./transcript/codec";
import { reduceTranscript } from "./transcript/reducer";
import type { TranscriptEvent } from "./transcript/events";
import { readToolCall, type ToolCallSource } from "./transcript/toolCall";
import { itemIdentity, type ConversationItem } from "./transcript/item";
import type { AgentEvent, SessionEntry } from "./types";

export { itemIdentity, type ConversationItem, type ConversationItemType } from "./transcript/item";
export { compactionReasonLabel, reasoningDisplayText } from "./transcript/codec";
export { isInternalCompactionEnvelope, stripWorkerResultBlocks } from "./transcript/reducer";
export {
  classifyExploratoryCommand,
  hasUnquotedRedirect,
  parseCommandTokens,
  type ToolCallDisplay,
  type ToolGlyph,
  type ToolStatus,
  type ToolVerb,
} from "./transcript/toolCall";

/** Select one root-to-leaf path without relying on input array order. */
export function selectActiveBranch(entries: SessionEntry[], activeLeafId: string | null): SessionEntry[] {
  if (!activeLeafId) return [];
  const leaf = entries.find((entry) => entry.id === activeLeafId);
  if (!leaf) return [];
  const byId = new Map(
    entries
      .filter((entry) => entry.sessionId === leaf.sessionId)
      .map((entry) => [entry.id, entry] as const),
  );
  const branch: SessionEntry[] = [];
  const visited = new Set<string>();
  let current: SessionEntry | undefined = byId.get(activeLeafId);
  while (current && !visited.has(current.id)) {
    branch.push(current);
    visited.add(current.id);
    current = current.parentEntryId ? byId.get(current.parentEntryId) : undefined;
  }
  return branch.reverse();
}

/** The live event window, reduced. */
export function reduceConversation(events: AgentEvent[]): ConversationItem[] {
  return reduceTranscript(events.map(normalizeAgentEvent));
}

/** Immutable forest entries on the active branch, reduced the same way. */
export function projectSessionConversation(entries: SessionEntry[], activeLeafId: string | null): ConversationItem[] {
  const branch = selectActiveBranch(entries, activeLeafId);
  const events: TranscriptEvent[] = [];
  for (const entry of branch) {
    const event = normalizeSessionEntry(entry);
    if (event) events.push(event);
  }
  return reduceTranscript(events);
}

/**
 * The optimistic pending rows that have not yet come back as real user turns.
 *
 * Delivery is judged per row, in the row's **own** session: a pending message
 * for an aside must reconcile against the aside's slice of the live stream,
 * never against whichever session happens to be selected. The selected
 * session gets one extra source — its durable projection — because its forest
 * is the only one the app holds in memory; every other session's user turn
 * still arrives on the global live stream, which is enough.
 *
 * Returns the same array reference when nothing was delivered, so callers can
 * keep referential equality for render stability.
 */
export function undeliveredPending<T extends { sessionId: string; text: string }>(
  pending: readonly T[],
  liveEvents: AgentEvent[],
  selected: { sessionId?: string; durableUserTexts: ReadonlySet<string> },
): T[] {
  if (!pending.length) return pending as T[];
  const liveTexts = new Map<string, Set<string>>();
  const deliveredIn = (sessionId: string, text: string): boolean => {
    let texts = liveTexts.get(sessionId);
    if (!texts) {
      texts = new Set(
        reduceConversation(liveEvents.filter(event => event.sessionId === sessionId))
          .filter(item => item.type === "message" && item.role === "user")
          .map(item => item.text.trim()),
      );
      liveTexts.set(sessionId, texts);
    }
    if (texts.has(text)) return true;
    return sessionId === selected.sessionId && selected.durableUserTexts.has(text);
  };
  const next = pending.filter(item => !deliveredIn(item.sessionId, item.text.trim()));
  return next.length === pending.length ? (pending as T[]) : next;
}

function isUnidentifiedAssistantShadow(live: ConversationItem, durable: ConversationItem): boolean {
  if (live.type !== "message" || durable.type !== "message") return false;
  if ((live.role ?? "assistant") === "user" || (durable.role ?? "assistant") === "user") return false;
  const text = live.text.trim();
  if (!text || text !== durable.text.trim()) return false;
  return live.status === "streaming" || !live.itemId;
}

export function mergeConversationProjections(durableItems: ConversationItem[], liveItems: ConversationItem[]): ConversationItem[] {
  const durableIds = new Set(durableItems.map(item => item.identity ?? itemIdentity(item)));
  const liveAnchors = new Map(liveItems.map(item => [item.identity ?? itemIdentity(item), item.sequence]));
  const items = durableItems.map(item => {
    const identity = item.identity ?? itemIdentity(item);
    let anchor = liveAnchors.get(identity);
    if (anchor === undefined) {
      const twin = liveItems.find(live => isUnidentifiedAssistantShadow(live, item));
      if (twin) anchor = twin.sequence;
    }
    return anchor !== undefined && anchor < item.sequence ? { ...item, sequence: anchor } : item;
  });
  for (const live of liveItems) {
    const identity = live.identity ?? itemIdentity(live);
    if (durableIds.has(identity)) continue;
    if (durableItems.some(durable => isUnidentifiedAssistantShadow(live, durable))) continue;
    items.push(live);
  }
  items.sort((a, b) => a.sequence - b.sequence);
  return items;
}

/** A worker-result payload stamped onto assistant prose, if that is all the text is. */
export function workerResultSummary(text: string): string | undefined {
  const prefix = "[worker result]";
  if (!text.startsWith(prefix)) return undefined;
  const rest = text.slice(prefix.length).trim();
  return rest || undefined;
}

/**
 * Read one conversation item as the tool call it describes.
 *
 * The reduced item already carries the facet the codec read at ingestion; this
 * derives one only for items assembled by hand (tests, previews) so no caller
 * has to know which is which.
 */
export function toolCallDisplay(item: ConversationItem) {
  if (item.tool) return item.tool;
  const source: ToolCallSource = {
    title: item.title,
    text: item.text,
    status: item.status,
    surface: item.type === "diff" ? "diff" : "activity",
    data: item.data,
  };
  return readToolCall(source);
}

/* ── Worker delegation items ─────────────────────────────────────────────
   Four different provider events land as `delegation` items and the renderer
   used to sniff them apart with inline `"key" in data` checks. Naming the
   facets once means the fold below and the card that draws them can never
   disagree about what a row is. */

export type DelegationFacet = "spawn" | "result" | "blocked" | "rejected" | "steered";

export function delegationFacet(item: ConversationItem): DelegationFacet {
  if ("childBlocked" in item.data) return "blocked";
  if ("willRetry" in item.data) return "rejected";
  if ("steeredBy" in item.data) return "steered";
  if ("delivered" in item.data) return "result";
  return "spawn";
}

export function delegationChildSessionId(item: ConversationItem): string | undefined {
  return typeof item.data.childSessionId === "string" ? item.data.childSessionId : undefined;
}

/**
 * Collapse each worker's result onto the panel that spawned it.
 *
 * The spawn row is a live panel while the worker runs, so letting the result
 * arrive as its own row further down left the user with two cards for one
 * worker: a stale live one and a disconnected outcome. One worker is one place
 * in the transcript, from "delegated" through to "done".
 *
 * Applied to the merged durable+live list rather than inside either projection,
 * because a spawn read from the forest and a result still only in the live
 * stream is the normal case mid-run.
 */
export function foldWorkerDelegations(items: ConversationItem[]): ConversationItem[] {
  const panelByChild = new Map<string, ConversationItem>();
  const folded: ConversationItem[] = [];
  for (const item of items) {
    if (item.type !== "delegation") { folded.push(item); continue; }
    const childSessionId = delegationChildSessionId(item);
    const facet = delegationFacet(item);
    if (!childSessionId) { folded.push(item); continue; }
    if (facet === "spawn") {
      // Copied because the merge below mutates the row that is already in the
      // output list, and the caller's item must not change underneath it.
      const panel = { ...item, data: { ...item.data } };
      panelByChild.set(childSessionId, panel);
      folded.push(panel);
      continue;
    }
    const panel = facet === "result" ? panelByChild.get(childSessionId) : undefined;
    // An orphan result — durable history truncated, or a branch switched away
    // from the spawn — still has to render. Folding must never lose a row.
    if (!panel) { folded.push(item); continue; }
    panel.data = { ...panel.data, ...item.data };
    panel.status = item.status ?? panel.status;
    panel.title = item.title ?? panel.title;
    // A `[worker result] …` stamp is routing metadata for the panel, not a
    // replacement for the human objective the spawn already showed.
    if (item.text && !workerResultSummary(item.text)) panel.text = item.text;
    // The panel keeps its own key and eventId: the key is what React reconciles
    // on, and the eventId is what the durable/live dedupe upstream matches.
  }
  return folded;
}

/**
 * Image attachments persisted on a user turn, as renderable data URIs.
 *
 * The backend stamps `data.attachments = [{mediaType, dataUri}]` onto the
 * user's message event so the conversation can re-render what was sent after
 * a reload. Malformed payloads return [] — a bad attachment must never be
 * able to break the transcript row it rides on.
 */
export function attachmentUris(data: Record<string, unknown>): string[] {
  const attachments = data.attachments;
  if (!Array.isArray(attachments)) return [];
  return attachments.flatMap((attachment) => {
    const dataUri = (attachment as { dataUri?: unknown } | null)?.dataUri;
    return typeof dataUri === "string" && dataUri.startsWith("data:image/") ? [dataUri] : [];
  });
}
