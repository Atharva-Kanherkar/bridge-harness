import { describe, expect, it } from "vitest";
import { normalizeAgentEvent } from "./codec";
import { projectSessionConversation } from "../conversation";
import type { ConversationItem } from "./item";
import { durableEntries, HARNESSES, harnessStream, reduceHarness, type GoldenHarness } from "./golden";

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
 * Not identical, and honestly so: Claude streams no tool output, OpenCode
 * re-sends a whole running part instead of a delta, and ACP has no
 * "thought completed" frame at all. What has to be identical is the row
 * structure below, which is what a reader actually sees.
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
    "thinking.delta",
    "tool.started", "tool.progress", "tool.completed",
    "thinking.delta",
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
    if (harness === "cursor") {
      // Documented divergence 11: ACP (acp_events.rs) emits a thought only as
      // `reasoning.delta`, never a `reasoning.completed`, so the live window's
      // two thought cards are built from deltas alone. The durable writer
      // never persists a kind ending in `.delta` (store.rs), so a Cursor
      // turn replayed from the forest has no thought cards at all.
      expect(durableRows).toEqual(liveRows.filter(row => row.type !== "reasoning"));
      return;
    }
    expect(durableRows).toEqual(liveRows);
  });
});
