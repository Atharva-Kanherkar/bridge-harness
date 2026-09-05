/**
 * Which reduced items share a row, and which stand alone.
 *
 * The transcript used to fold *consecutive* tool calls, which reads well for
 * an agent that emits its tool calls back to back and falls apart for one that
 * opens a fresh thought between every pair of them: a hundred-step turn became
 * a hundred one-call groups with a hundred thoughts wedged between them, and
 * the reader lost the turn entirely.
 *
 * So the run, not the adjacency, is the unit. Within one turn every tool call
 * belongs to the same group, thoughts that fall inside the run travel with it,
 * and anything a reader has to read or answer — prose, an approval, an error —
 * closes the group and stands on its own, because rearranging narrative order
 * would destroy the one thing a transcript is for.
 *
 * Pure and React-free, so the rules are testable as data.
 */

import type { ConversationItem, ConversationItemType } from "./item";

/** One top-level entry of the transcript. */
export type Rendered =
  | { kind: "item"; item: ConversationItem }
  /** A turn's tool work, with the thoughts that fell inside it, in order. */
  | { kind: "group"; key: string; items: ConversationItem[] }
  | { kind: "raw-group"; key: string; items: ConversationItem[] };

/** The rows a group is made of. */
const TOOL_TYPES = new Set<ConversationItemType>(["activity", "diff", "artifact"]);

/**
 * The rows that travel with a run rather than interrupting it: a thought, and
 * a plan update. Both are the model narrating work in progress; neither is
 * something the reader has to act on, and letting either close a group is what
 * shattered a long turn into confetti.
 */
const HELD_TYPES = new Set<ConversationItemType>(["reasoning", "plan"]);

export function isToolItem(item: ConversationItem): boolean {
  return TOOL_TYPES.has(item.type);
}

/** Stable across re-renders, across the live-to-durable merge, and across the
 *  moment the run finishes: the identity of the call that opened it. */
function groupKey(first: ConversationItem): string {
  return `group:${first.identity ?? first.key}`;
}

/**
 * Consecutive thoughts, as one row.
 *
 * A provider that opens a new reasoning item per paragraph would otherwise
 * stack a card per paragraph at the top level. Rebuilt rather than mutated:
 * these items are the memoized output of the reducer and writing through them
 * would edit a projection two other callers are holding.
 */
function coalesceThoughts(items: ConversationItem[]): ConversationItem[] {
  const out: ConversationItem[] = [];
  for (const item of items) {
    const prior = out[out.length - 1];
    if (item.type !== "reasoning" || prior?.type !== "reasoning") {
      out.push(item);
      continue;
    }
    out[out.length - 1] = {
      ...prior,
      text: prior.text ? `${prior.text}\n${item.text}` : item.text,
      status: item.status === "streaming" || prior.status === "streaming" ? "streaming" : "completed",
      data: { ...prior.data, ...item.data },
    };
  }
  return out;
}

/**
 * One turn numbering across a merged list.
 *
 * The transcript draws two projections at once: durable rows off the forest
 * and live rows off the event window, merged row by row. Each was reduced on
 * its own and each counted turns from its own start — the forest from the
 * first message of the session, the live window from wherever it opened — so
 * mid-turn the same real turn carries two different indices, and the walk
 * below, which closes a group whenever the turn changes, splits the run at the
 * seam. The seam then moves on every forest poll, as more of the turn goes
 * durable, which is what made a settled run come apart while the reader
 * watched.
 *
 * So the merged list is re-stamped from the one boundary both projections can
 * see: a user message opens a turn, and belongs to the turn it opens. Rows
 * ahead of the first user message keep turn 0, the index the reducer gives
 * them. Turn markers are deliberately not consulted — they are live-only, and
 * that asymmetry is the whole defect.
 *
 * The reducer keeps its own stamp because it is the right answer for a single
 * projection: it can see a marker-only boundary that no user message
 * separates, and the live-versus-durable parity assertion in the golden test
 * is written against it. This is the merge's answer, and it only has to hold
 * for a list the merge produced.
 *
 * Rows whose turn does not change are returned as they were, so a re-stamp
 * never invalidates a memo signature it did not need to.
 */
export function alignTurns(items: ConversationItem[]): ConversationItem[] {
  let turn = 0;
  return items.map((item) => {
    if (item.type === "message" && item.role === "user") turn += 1;
    return item.turn === turn ? item : { ...item, turn };
  });
}

/**
 * Fold a turn's tool work into one group each, leaving everything else where
 * the reducer put it.
 *
 * The walk holds two things: the open group, and the rows *held* since its
 * last tool call. A further tool call means those held rows fell inside the
 * run, so they join the group's timeline; anything else means the run is over,
 * so they flush as top-level rows behind it.
 */
export function groupItems(items: ConversationItem[]): Rendered[] {
  const out: Rendered[] = [];
  const rawItems: ConversationItem[] = [];
  let group: ConversationItem[] | null = null;
  let held: ConversationItem[] = [];
  let turn: number | undefined;

  const flushHeld = () => {
    for (const item of coalesceThoughts(held)) out.push({ kind: "item", item });
    held = [];
  };
  const closeGroup = () => {
    if (group) out.push({ kind: "group", key: groupKey(group[0]), items: group });
    group = null;
    // After the group, never before it: these arrived once its last call had.
    flushHeld();
  };

  for (const item of items) {
    if (item.type === "raw") {
      rawItems.push(item);
      continue;
    }
    // A turn is the outer bound of a run. Work from two turns is two runs
    // however few rows separate them.
    if (turn !== undefined && item.turn !== turn) closeGroup();
    turn = item.turn;

    // A model change and a stale base are milestones, not activity: a group of
    // one labeled "Used tools" is how a reload made either read as a glitch.
    if (item.data.staleBase === true || item.data.freshProviderSession === true) {
      closeGroup();
      out.push({ kind: "item", item });
      continue;
    }

    if (isToolItem(item)) {
      if (group) {
        group.push(...held, item);
        held = [];
      } else {
        // Nothing held here belongs to the run: it came before the first call.
        flushHeld();
        group = [item];
      }
      continue;
    }

    if (HELD_TYPES.has(item.type)) {
      held.push(item);
      continue;
    }

    closeGroup();
    out.push({ kind: "item", item });
  }

  closeGroup();

  if (rawItems.length) {
    out.push({ kind: "raw-group", key: "raw-provider-events", items: rawItems });
  }

  return out;
}
