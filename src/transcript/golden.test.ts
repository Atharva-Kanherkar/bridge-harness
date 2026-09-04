import { describe, expect, it } from "vitest";
import { normalizeAgentEvent } from "./codec";
import { reduceTranscript } from "./reducer";
import { HARNESSES, harnessStream, reduceHarness, type GoldenHarness } from "./golden";

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

  it("keeps live and durable reductions of the same stream in step", () => {
    // Reducing a normalized stream twice must be a pure function of its input:
    // the reducer holds no state between calls, which is what lets the live
    // window and the forest projection share it.
    for (const harness of HARNESSES) {
      const events = harnessStream(harness).map(normalizeAgentEvent);
      expect(reduceTranscript(events)).toEqual(reduceTranscript(events));
    }
  });
});
