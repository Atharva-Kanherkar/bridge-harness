import { describe, expect, it } from "vitest";
import { foldWorkerDelegations, mergeConversationProjections, projectSessionConversation, reduceConversation } from "../conversation";
import { floodStream } from "./fixtures/codexFlood";
import { durableEntriesFrom } from "./golden";
import { alignTurns, groupItems } from "./grouping";
import { asWireKind } from "./wire";
import type { AgentEvent } from "../types";

/**
 * The seam between the two projections.
 *
 * The transcript reduces the forest and the live window separately and merges
 * the rows. Each reduction counts turns from its own start, so mid-turn the
 * durable half of a run is stamped with one index and the live half with
 * another — and the grouping walk, which closes a group whenever the turn
 * changes, cuts the run in two at exactly that point. Every forest poll moves
 * the cut, so a settled run comes apart while the reader is watching it.
 */

const SESSION = "flood";

const frame = (id: number, kind: string, overrides: Partial<AgentEvent>): AgentEvent => ({
  id, sessionId: SESSION, sequence: id, protocolVersion: 1, kind: asWireKind(kind),
  itemId: null, role: null, status: null, title: null, text: null, data: {},
  providerMeta: { adapter: SESSION }, createdAt: "2026-09-05T09:59:00Z", ...overrides,
});

/** A short exchange, then the flood: two turns, so the two counters disagree. */
function twoTurnStream(steps: number): AgentEvent[] {
  const opening = [
    frame(1, "message.completed", { itemId: "u0", role: "user", status: "completed", text: "warm up" }),
    frame(2, "message.completed", { itemId: "a0", role: "assistant", status: "completed", text: "ready" }),
  ];
  const second = floodStream(steps).map(event => event.sequence > 0
    ? { ...event, id: event.id + opening.length, sequence: event.sequence + opening.length }
    : event);
  return [...opening, ...second];
}

/**
 * What the pane holds mid-turn: the forest has the first half of the stream,
 * the live window opened with the second turn and still has all of it.
 */
function merged(steps: number) {
  const stream = twoTurnStream(steps);
  const entries = durableEntriesFrom(SESSION, stream.slice(0, Math.floor(stream.length / 2)));
  const durableItems = projectSessionConversation(entries, entries[entries.length - 1].id);
  const liveItems = reduceConversation(stream.slice(2));
  const items = mergeConversationProjections(durableItems, liveItems);
  return foldWorkerDelegations(items.filter(item => item.type !== "raw"));
}

const groups = (rows: ReturnType<typeof groupItems>) => rows.filter(row => row.kind === "group");

describe("the durable and live seam", () => {
  it("splits one run in two while the forest is still catching up", () => {
    // The defect itself, so the fix below is measured against something real:
    // the durable rows of the second turn say turn 2, the live rows of the
    // same turn say turn 1, and the walk closes the group between them.
    const rows = groupItems(merged(20));
    expect(groups(rows).length).toBeGreaterThan(1);
  });

  it("draws the merged turn as one run once the turns are aligned", () => {
    const rows = groupItems(alignTurns(merged(20)));
    expect(groups(rows)).toHaveLength(1);
    // Their first message and its reply, then the second turn: their message,
    // the thought it opened with, the run, the closing thought, the reply.
    expect(rows.map(row => row.kind === "item" ? row.item.type : row.kind)).toEqual([
      "message", "message", "message", "reasoning", "group", "reasoning", "message",
    ]);
  });

  it("keeps the top-level rows flat as the run grows", () => {
    const shape = (steps: number) => groupItems(alignTurns(merged(steps))).map(row => row.kind);
    expect(shape(10)).toEqual(shape(40));
  });

  it("leaves a row whose turn is already right exactly as it was", () => {
    const items = merged(10);
    const aligned = alignTurns(items);
    const unchanged = aligned.filter((item, index) => item === items[index]);
    // A re-stamp that rebuilt every row would invalidate every memo signature
    // in the transcript on every poll.
    expect(unchanged.length).toBeGreaterThan(0);
    expect(aligned.every((item, index) => item.key === items[index].key)).toBe(true);
  });
});
