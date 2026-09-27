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
import { readToolCall, type ToolCallDisplay, type ToolCallSource } from "./transcript/toolCall";
import { itemIdentity, type ConversationItem } from "./transcript/item";
import type { AgentEvent, SessionEntry } from "./types";

export { itemIdentity, itemSignature, sameItem, sameItems, subagentLabel, subagentSource, type ConversationItem, type ConversationItemType, type SubagentSource } from "./transcript/item";
export { alignTurns, groupItems, isToolItem, type Rendered } from "./transcript/grouping";
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

/**
 * The text two projections of one row would share, when they have no shared id.
 *
 * A row's `identity` is what tells the merge that a live frame and its persisted
 * twin are one thing, and it is only as good as the ids the two sides carry. A
 * tool call always names itself, and named prose does too, so their identities
 * agree and the dedupe below never has to guess.
 *
 * A thought usually does not. `liveKey` returns no key for an unnamed reasoning
 * frame (the reducer borrows the turn's), and the durable writer files the same
 * frame under the forest entry's own id, so the live row reads
 * `reasoning:<event id>` while its twin reads `entry:<entry id>`, which are two
 * numbering spaces that can never agree. Both rows then survive the merge, and
 * `coalesceThoughts` joins them into a single card whose body is the same
 * paragraph twice: one thought, printed twice, in the one component the
 * transcript has for thinking.
 *
 * So a row with no id to match on is matched on its text, the one thing the two
 * projections cannot word differently. Unnamed assistant prose already needed
 * this and already had it. A thought is the same problem with the same answer,
 * and the commoner case, since nearly every harness sends reasoning with no item
 * id. The match is on the whole trimmed body, so a row it drops is replaced by
 * one that says exactly the same thing, and the durable row is the one that
 * survives a reload anyway.
 */
function shadowRowText(item: ConversationItem): string | undefined {
  if (item.type === "reasoning") return item.text.trim() || undefined;
  if (item.type !== "message" || item.role === "user") return undefined;
  return item.text.trim() || undefined;
}

/**
 * How much of a streaming row has to be there before it counts as the opening of
 * the thought the forest already holds.
 *
 * A delta is never persisted, so a thought the forest has caught up with leaves
 * a live row holding only the opening of a body that is already stored whole. The
 * two cannot be compared for equality until the stream finishes, and a forest
 * poll is three seconds wide, which is long enough for the whole seam to be on
 * screen twice. Comparing prefixes fixes it, but "the newest stored thought
 * begins with what I have just streamed" only means something once there is
 * enough text to mean anything: the first few words of a thought are the part
 * two different thoughts are most likely to share.
 */
const STREAMED_PREFIX_FLOOR = 24;

export function mergeConversationProjections(durableItems: ConversationItem[], liveItems: ConversationItem[]): ConversationItem[] {
  const durableIds = new Set(durableItems.map(item => item.identity ?? itemIdentity(item)));
  const liveAnchors = new Map(liveItems.map(item => [item.identity ?? itemIdentity(item), item.sequence]));
  const liveShadows = new Map<string, number>();
  const durableTexts = new Set<string>();
  for (const live of liveItems) {
    if (live.status !== "streaming" && live.itemId) continue;
    const text = shadowRowText(live);
    // Preserve find() semantics: the first matching item supplies the anchor,
    // which need not be the smallest sequence in an unsorted input.
    if (text !== undefined && !liveShadows.has(text)) liveShadows.set(text, live.sequence);
  }
  const items = durableItems.map(item => {
    const identity = item.identity ?? itemIdentity(item);
    const text = shadowRowText(item);
    if (text !== undefined) durableTexts.add(text);
    const anchor = liveAnchors.get(identity) ?? (text === undefined ? undefined : liveShadows.get(text));
    return anchor !== undefined && anchor < item.sequence ? { ...item, sequence: anchor } : item;
  });
  // The thought being streamed is the newest one in the forest, so the newest
  // stored thought is the only one that can still be arriving. Comparing
  // against all of them would let an old thought swallow a new one that merely
  // opens the same way.
  let newestStoredThought: string | undefined;
  for (const item of durableItems) if (item.type === "reasoning") newestStoredThought = item.text.trim();
  for (const live of liveItems) {
    const identity = live.identity ?? itemIdentity(live);
    if (durableIds.has(identity)) continue;
    const text = shadowRowText(live);
    if ((live.status === "streaming" || !live.itemId) && text !== undefined && durableTexts.has(text)) continue;
    if (live.status === "streaming" && text !== undefined && text.length >= STREAMED_PREFIX_FLOOR
      && newestStoredThought?.startsWith(text)) continue;
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
export function toolCallDisplay(item: ConversationItem): ToolCallDisplay {
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
 * Collapse each worker's result onto the row that spawned it.
 *
 * Letting the result arrive as its own row further down left the user with two
 * rows for one worker: a stale "delegated" and a disconnected outcome. One
 * worker is one line in the transcript, from "delegated" through to "done";
 * the worker itself is watched in the dock's Agents pane.
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
    // The panel's text is the *objective* — the one line saying what this
    // worker was asked to do. A result's text is the worker's own prose, which
    // is a different fact and can run to paragraphs; letting it overwrite the
    // objective turned a finished card into a wall of summary with the ask
    // gone. The worker's own words stay in its chat in the Agents pane.
    // Fill only when the spawn carried no objective at all.
    if (item.text && !panel.text && !workerResultSummary(item.text)) panel.text = item.text;
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
