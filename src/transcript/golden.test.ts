import { describe, expect, it } from "vitest";
import { normalizeAgentEvent } from "./codec";
import { mergeConversationProjections, projectSessionConversation, reduceConversation } from "../conversation";
import { alignTurns, groupItems } from "./grouping";
import type { ConversationItem } from "./item";
import { durableEntries, durableEntriesFrom, HARNESSES, harnessStream, reduceHarness, type GoldenHarness } from "./golden";

/**
 * One logical turn, four harnesses, one transcript.
 *
 * The fixtures in `fixtures/` are the same turn — user message, thought,
 * command with output, second thought, file read, patch, reply, done — in the
 * shape each Rust adapter actually publishes it. Everything asserted here is a
 * claim about what a reader sees; everything that legitimately differs is
 * asserted as the documented difference rather than skipped.
 */

/** What the transcript is supposed to be, whichever agent produced it. */
const EXPECTED_ROWS = [
  { type: "message", role: "user", status: "completed" },
  { type: "reasoning", status: "completed" },
  { type: "activity", status: "completed", verb: "run" },
  { type: "reasoning", status: "completed" },
  { type: "activity", status: "completed", verb: "read" },
  { type: "diff", status: "completed", verb: "edit" },
  { type: "message", role: "assistant", status: "completed" },
] as const;

/**
 * The normalized event types each harness emits for that turn.
 *
 * Not identical, and honestly so: Claude streams no tool output and OpenCode
 * re-sends a whole running part instead of a delta. Every one of them does end
 * a thought, though — ACP has no terminal reasoning frame on the wire, so
 * `acp_events.rs` closes the thought run itself at the first update that is not
 * a thought chunk. What has to be identical is the row structure below, which
 * is what a reader actually sees.
 */
const EXPECTED_EVENT_TYPES: Record<GoldenHarness, string[]> = {
  claude: [
    "message.completed",
    "thinking.delta", "thinking.completed",
    "tool.started", "tool.completed",
    "thinking.delta", "thinking.completed",
    "tool.started", "tool.completed",
    "tool.started", "tool.completed",
    "message.delta", "message.completed",
    "turn.completed",
  ],
  codex: [
    "message.completed",
    "thinking.delta", "thinking.completed",
    "tool.started", "tool.progress", "tool.completed",
    "thinking.delta", "thinking.completed",
    "tool.started", "tool.completed",
    "tool.started", "tool.completed",
    "message.delta", "message.completed",
    "turn.completed",
  ],
  cursor: [
    "message.completed",
    "thinking.delta", "thinking.completed",
    "tool.started", "tool.progress", "tool.completed",
    "thinking.delta", "thinking.completed",
    "tool.started", "tool.completed",
    "tool.started", "tool.completed",
    "message.delta", "message.completed",
    "turn.completed",
  ],
  opencode: [
    "message.completed",
    "thinking.delta", "thinking.completed",
    "tool.started", "tool.started", "tool.completed",
    "thinking.delta", "thinking.completed",
    "tool.started", "tool.completed",
    "tool.started", "tool.completed",
    "message.delta", "message.completed",
    "turn.completed",
  ],
};

/** Only two of the four protocols have a field for an exit code. */
const REPORTS_EXIT_CODE: Record<GoldenHarness, boolean> = {
  claude: false, codex: true, cursor: false, opencode: true,
};

describe("golden streams", () => {
  it.each(HARNESSES)("normalizes every %s frame into the union, never into unknown", harness => {
    const events = harnessStream(harness).map(normalizeAgentEvent);
    expect(events.filter(event => event.type === "unknown")).toEqual([]);
    expect(events.map(event => event.type)).toEqual(EXPECTED_EVENT_TYPES[harness]);
  });

  it.each(HARNESSES)("puts %s's three calls on the same three surfaces", harness => {
    // By item id, not by frame: OpenCode re-sends a whole running part, so one
    // call can open twice. Which call is a diff must not depend on that.
    const surfaces = new Map<string, string>();
    for (const event of harnessStream(harness).map(normalizeAgentEvent)) {
      if (event.type !== "tool.started") continue;
      const id = event.envelope.itemId ?? event.envelope.key ?? "";
      if (!surfaces.has(id)) surfaces.set(id, event.surface);
    }
    expect([...surfaces.values()]).toEqual(["activity", "activity", "diff"]);
  });

  it.each(HARNESSES)("reduces the %s turn to the same rows", harness => {
    const rows = reduceHarness(harness).map(item => ({
      type: item.type,
      ...(item.role ? { role: item.role } : {}),
      status: item.status,
      ...(item.tool && item.type !== "message" && item.type !== "reasoning" ? { verb: item.tool.verb } : {}),
    }));
    expect(rows).toEqual(EXPECTED_ROWS.map(row => ({ ...row })));
  });

  it.each(HARNESSES)("says the same thing about %s's tool calls", harness => {
    const items = reduceHarness(harness);
    const [command, read, edit] = items.filter(item => item.type === "activity" || item.type === "diff");

    // The command: what ran, what it printed.
    expect(command.tool?.command).toBe("bun test");
    expect(command.tool?.output).toContain("1 failing");

    // The read: which file, and its contents underneath.
    expect(read.tool?.target).toBe("lib.rs");
    expect(read.tool?.path).toBe("src/lib.rs");
    expect(read.tool?.output).toContain("fn a() {}");

    // The patch: a diff card carrying a real unified diff.
    expect(edit.tool?.target).toBe("lib.rs");
    expect(edit.tool?.path).toBe("src/lib.rs");
    expect(edit.tool?.patch).toContain("+fn a() { 1 }");
    expect(edit.tool?.patch).toContain("-fn a() {}");
  });

  it.each(HARNESSES)("reports %s's exit code only where the protocol has one", harness => {
    const [command] = reduceHarness(harness).filter(item => item.type === "activity");
    expect(command.tool?.exitCode).toBe(REPORTS_EXIT_CODE[harness] ? 1 : undefined);
  });

  it("reads the same prose out of all four", () => {
    for (const harness of HARNESSES) {
      const messages = reduceHarness(harness).filter(item => item.type === "message");
      expect(messages.map(item => item.text)).toEqual([
        "Run the tests and fix the failing case.",
        "Fixed the assertion in src/lib.rs.",
      ]);
      const thoughts = reduceHarness(harness).filter(item => item.type === "reasoning");
      expect(thoughts.map(item => item.text)).toEqual([
        "Start with the suite.",
        "Read the file it points at.",
      ]);
    }
  });

  /** What a reader sees, independent of which projection produced the row. */
  function projectRow(item: ConversationItem) {
    return {
      type: item.type,
      ...(item.role ? { role: item.role } : {}),
      status: item.status,
      // The turn is part of what a reader sees now that a run folds into one
      // group per turn. Live counts turn markers the forest never stored, so
      // this is the assertion that the two still arrive at the same index.
      turn: item.turn,
      verb: item.tool?.verb,
      target: item.tool?.target,
      path: item.tool?.path,
      hasPatch: !!item.tool?.patch,
    };
  }

  it.each(HARNESSES)("agrees between the live and durable projections of the %s turn", harness => {
    // The live window and the forest projection share one reducer; this is
    // the assertion that they actually agree on one real turn, not just that
    // reducing the same events twice is deterministic.
    const liveRows = reduceHarness(harness).map(projectRow);
    const entries = durableEntries(harness);
    const lastEntryId = entries.at(-1)?.id ?? null;
    const durableRows = projectSessionConversation(entries, lastEntryId).map(projectRow);
    // No exception for any of the four. Cursor used to need one: ACP streams a
    // thought as deltas and the forest refuses to persist a `.delta`, so a
    // replayed Cursor turn had no thoughts in it at all. The adapter closes the
    // run itself now, and the completion is what the forest keeps.
    expect(durableRows).toEqual(liveRows);
  });
});

/**
 * The same four turns, with the reasoning left unnamed, swept two ways.
 *
 * Every fixture happens to name its reasoning (`reasoning-msg-1`, `thought-2`,
 * …), so all four reconcile the live row and its stored twin on identity alone
 * and none of them can see a doubled thought. But a provider item id on a
 * reasoning frame is a courtesy, not a guarantee: several harnesses send
 * reasoning with none at all, and then the live row is keyed from the live event
 * id and the stored row from the forest entry, two numbering spaces that can
 * never agree, so the merge kept both and coalescing printed the same paragraph
 * twice inside one Thinking card.
 *
 * So the names come off, and **both** axes are swept. The live window is a tail
 * of the stream, not the whole turn, and the forest is a prefix that grows on
 * its own 3 s poll, so every pair of cuts is a state the reader can be in.
 * Sweeping only the forest, with the live window holding the entire turn, hides
 * the defect completely: the live window then keeps only the turn's last thought
 * while the forest holds each one separately, so no two rows ever say the same
 * thing. The doubled card needs the live window to still be holding the thought
 * the forest has just stored, which is exactly what a live tail is.
 *
 * The claim is the one this layer owns: **the forest never makes the reader read
 * a thought twice.** The doubled body is one card holding `thought\nthought`, so
 * the assertion is that no card ever holds the same line twice, at any pair of
 * cuts.
 *
 * What a card *should* hold is deliberately not asserted here. An unnamed
 * provider gets one thought per turn: `liveKey` hands an unnamed reasoning
 * frame the turn's key on purpose. So a live window that has seen two thoughts
 * keeps only the second while the forest still holds the first, and coalescing
 * puts both in one card in reverse order. That is a real artifact of the unnamed
 * path, but it is a different defect from a doubled thought, it is the codec's
 * own documented decision, and fixing it would change what unnamed harnesses show
 * rather than stop them showing it twice.
 */
describe("golden streams with unnamed reasoning", () => {
  const unnamed = (harness: GoldenHarness) =>
    harnessStream(harness).map(event => String(event.kind).startsWith("reasoning") ? { ...event, itemId: null } : event);

  it.each(HARNESSES)("never shows a %s thought twice, at any live and durable cut", harness => {
    const frames = unnamed(harness);
    // Guard the guard: the stream has to differ from the named fixture, or this
    // proves nothing about the path that needs covering.
    expect(frames.some(event => String(event.kind).startsWith("reasoning") && event.itemId === null)).toBe(true);
    const entries = durableEntriesFrom(harness, frames);
    let thoughts = 0;
    for (let liveCut = 1; liveCut <= frames.length; liveCut += 1) {
      const live = reduceConversation(frames.slice(0, liveCut));
      for (let cut = 0; cut <= entries.length; cut += 1) {
        const forest = entries.slice(0, cut);
        const merged = alignTurns(mergeConversationProjections(projectSessionConversation(forest, forest.at(-1)?.id ?? null), live));
        for (const row of groupItems(merged)) {
          if (row.kind !== "item" || row.item.type !== "reasoning") continue;
          thoughts += 1;
          const lines = row.item.text.split("\n");
          expect(new Set(lines).size, `live ${liveCut}, forest ${cut}: ${JSON.stringify(row.item.text)}`).toBe(lines.length);
        }
      }
    }
    // A sweep that asserted nothing because it rendered nothing is not a pass.
    expect(thoughts).toBeGreaterThan(0);
  });
});
