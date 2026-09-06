import { describe, expect, it } from "vitest";
import { groupItems } from "./grouping";
import type { ConversationItem, ConversationItemType } from "./item";

/**
 * The folding rules, as data.
 *
 * One turn's tool work is one group. A thought inside the run travels with it;
 * a thought outside it stands alone; anything a reader has to read or answer
 * closes the run and keeps its place.
 */

let sequence = 0;
const item = (type: ConversationItemType, overrides: Partial<ConversationItem> = {}): ConversationItem => {
  sequence += 1;
  return {
    key: `k${sequence}`, identity: `i${sequence}`, type, eventId: sequence, sequence,
    turn: 1, text: "", data: {}, ...overrides,
  };
};

const shape = (items: ConversationItem[]) =>
  groupItems(items).map(entry => entry.kind === "item" ? entry.item.type : entry.kind);

describe("groupItems", () => {
  it("folds a run of tool calls into one group however many thoughts interrupt it", () => {
    const items = [];
    for (let step = 0; step < 40; step += 1) {
      items.push(item("reasoning", { text: `thinking ${step}` }));
      items.push(item("activity"));
    }
    const rendered = groupItems(items);
    // The first thought came before any call, so it stands alone. Every other
    // thought fell inside the run and travelled with it.
    expect(shape(items)).toEqual(["reasoning", "group"]);
    const group = rendered[1];
    expect(group.kind === "group" && group.items).toHaveLength(79);
  });

  it("keeps a thought after the last call at the top level", () => {
    expect(shape([
      item("activity"),
      item("reasoning", { text: "done, writing it up" }),
      item("message", { role: "assistant", text: "here is what I did" }),
    ])).toEqual(["group", "reasoning", "message"]);
  });

  it("does not let a plan update split a run", () => {
    expect(shape([
      item("activity"),
      item("plan", { title: "Next step" }),
      item("activity"),
    ])).toEqual(["group"]);
  });

  it("lets prose close a run, because narrative order is the point", () => {
    expect(shape([
      item("activity"),
      item("message", { role: "assistant", text: "found it" }),
      item("activity"),
    ])).toEqual(["group", "message", "group"]);
  });

  it("lets a decision the reader has to make stand on its own", () => {
    expect(shape([
      item("activity"),
      item("permission", { status: "pending" }),
      item("activity"),
    ])).toEqual(["group", "permission", "group"]);
  });

  it("never merges work from two turns", () => {
    expect(shape([
      item("activity", { turn: 1 }),
      item("reasoning", { turn: 2, text: "new instructions" }),
      item("activity", { turn: 2 }),
    ])).toEqual(["group", "reasoning", "group"]);
  });

  it("keeps the group's key on the identity of the call that opened it", () => {
    const first = item("activity", { key: "live-key", identity: "cmd-1" });
    const [group] = groupItems([first, item("activity")]);
    expect(group.kind === "group" && group.key).toBe("group:cmd-1");
  });

  it("coalesces consecutive top-level thoughts into one row", () => {
    const rendered = groupItems([
      item("reasoning", { text: "first" }),
      item("reasoning", { text: "second" }),
    ]);
    expect(rendered).toHaveLength(1);
    expect(rendered[0].kind === "item" && rendered[0].item.text).toBe("first\nsecond");
  });

  it("does not write through the items it was given", () => {
    const first = item("reasoning", { text: "first" });
    groupItems([first, item("reasoning", { text: "second" })]);
    expect(first.text).toBe("first");
  });

  it("collects raw provider events into one inspector at the end", () => {
    expect(shape([
      item("raw"),
      item("activity"),
      item("raw"),
    ])).toEqual(["group", "raw-group"]);
  });

  it("keeps a model change out of the run it interrupts", () => {
    expect(shape([
      item("activity"),
      item("model-change"),
      item("activity"),
    ])).toEqual(["group", "model-change", "group"]);
  });

  it("keeps a resumed model change out of the run just the same", () => {
    // The milestone is a normalized item type, so grouping finds it however
    // the switch went — natively resumed or freshly started.
    expect(shape([
      item("activity"),
      item("model-change", { data: { modelChanged: true, freshProviderSession: false } }),
      item("activity"),
    ])).toEqual(["group", "model-change", "group"]);
  });
});
