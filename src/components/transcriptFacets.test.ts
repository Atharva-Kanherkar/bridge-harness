import { describe, expect, it } from "vitest";
import { asWireKind } from "../transcript/wire";
import type { AgentEvent } from "../types";
import {
  buildTranscriptRows,
  countFacets,
  filterTranscriptRows,
  problemReason,
  TRANSCRIPT_FACETS,
  turnNumbersAreAbsolute,
} from "./transcriptFacets";

// Contract: testing/feat-session-observability.md §C1–C5.

const event = (sequence: number, kind: string, overrides: Partial<AgentEvent> = {}): AgentEvent => ({
  id: sequence, sessionId: "s", sequence, protocolVersion: 1, kind: asWireKind(kind), itemId: null, role: null,
  status: null, title: null, text: null, data: {}, providerMeta: {}, createdAt: "2026-09-13T10:00:00Z", ...overrides,
});

describe("facets", () => {
  it("sorts the normalized vocabulary into one bucket each", () => {
    const rows = buildTranscriptRows([
      event(1, "user.message", { text: "go" }),
      event(2, "reasoning.completed", { text: "I should read the test first." }),
      event(3, "tool.started", { title: "cargo test" }),
      event(4, "command.completed", { title: "ls" }),
      event(5, "turn.completed"),
      event(6, "usage.updated"),
      event(7, "approval.requested"),
      event(8, "permission.requested"),
      event(9, "delegation.requested"),
      event(10, "worker.result", { status: "completed" }),
      event(11, "session.status"),
    ]);
    expect(rows.map(row => row.facet)).toEqual([
      "messages", "thinking", "tools", "tools", "turns", "usage",
      "approvals", "approvals", "delegation", "delegation", "other",
    ]);
  });

  it("counts every chip, including the ones that read zero", () => {
    const counts = countFacets(buildTranscriptRows([
      event(1, "user.message", { text: "go" }),
      event(2, "assistant.message", { text: "done" }),
    ]));
    expect(Object.keys(counts).sort()).toEqual([...TRANSCRIPT_FACETS].sort());
    expect(counts.all).toBe(2);
    expect(counts.messages).toBe(2);
    expect(counts.problems).toBe(0);
    expect(counts.thinking).toBe(0);
  });

  it("does not count an unbucketed event toward a facet, only toward all", () => {
    const counts = countFacets(buildTranscriptRows([event(1, "session.status")]));
    expect(counts.all).toBe(1);
    expect(TRANSCRIPT_FACETS.filter(facet => facet !== "all").every(facet => counts[facet] === 0)).toBe(true);
  });
});

describe("problems", () => {
  it("reads a failure the same way whichever harness produced it", () => {
    // The rules are over the normalized vocabulary, so this is one assertion
    // rather than one per adapter.
    expect(problemReason(event(1, "tool.completed", { status: "failed" }), "tool.completed"))
      .toBe("This tool call failed.");
    expect(problemReason(event(2, "command.completed", { status: "error" }), "command.completed"))
      .toBe("This tool call failed.");
    expect(problemReason(event(3, "error", { text: "429" }), "error"))
      .toBe("The provider reported an error.");
    expect(problemReason(event(4, "session.error"), "session.error"))
      .toBe("The provider reported an error.");
    expect(problemReason(event(5, "compaction.failed"), "compaction.failed"))
      .toBe("This step failed.");
    expect(problemReason(event(6, "session.resume_failed"), "session.resume_failed"))
      .toBe("This step failed.");
    expect(problemReason(event(7, "turn.completed", { status: "failed" }), "turn.completed"))
      .toBe("The turn ended in failure.");
  });

  it("calls a worker that did not complete a problem, and one that did not", () => {
    expect(problemReason(event(1, "worker.result", { status: "failed" }), "worker.result"))
      .toBe("A delegated worker ended failed.");
    expect(problemReason(event(2, "worker.result", { status: "needs_delegation" }), "worker.result"))
      .toBe("A delegated worker ended needs_delegation.");
    expect(problemReason(event(3, "worker.result", { status: "completed" }), "worker.result")).toBeNull();
  });

  it("leaves ordinary events alone", () => {
    for (const kind of ["assistant.message", "tool.completed", "turn.completed", "approval.resolved"]) {
      expect(problemReason(event(1, kind, { status: "completed" }), kind)).toBeNull();
    }
  });

  it("counts problems whichever facet is selected, because a hidden failure is the bug", () => {
    const rows = buildTranscriptRows([
      event(1, "assistant.message", { text: "ok" }),
      event(2, "tool.completed", { status: "failed", title: "cargo test" }),
    ]);
    expect(countFacets(rows).problems).toBe(1);
    expect(filterTranscriptRows(rows, "problems", "")).toHaveLength(1);
    expect(filterTranscriptRows(rows, "messages", "")).toHaveLength(1);
  });
});

describe("turn grouping", () => {
  it("takes the session's own turn number off the boundary, not its position in the window", () => {
    // A pane showing the newest page of a long session used to count from what
    // it had loaded, so the same event read as turn 2 there and turn 118 in an
    // export. The boundary carries its ordinal; both readers use it.
    const window = buildTranscriptRows([
      event(9001, "turn.started", { data: { turnIndex: 118 } }),
      event(9002, "assistant.message", { text: "late in a long session" }),
      event(9003, "turn.started", { data: { turnIndex: 119 } }),
      event(9004, "assistant.message", { text: "and the next" }),
    ]);
    expect(window.map(row => row.turnIndex)).toEqual([118, 118, 119, 119]);
  });

  it("does not renumber loaded rows when an earlier page arrives", () => {
    const tail = [
      event(9001, "turn.started", { data: { turnIndex: 118 } }),
      event(9002, "assistant.message", { text: "late" }),
    ];
    const earlier = [
      event(8001, "turn.started", { data: { turnIndex: 117 } }),
      event(8002, "assistant.message", { text: "earlier" }),
    ];
    const before = buildTranscriptRows(tail).map(row => row.turnIndex);
    const after = buildTranscriptRows([...earlier, ...tail]).slice(2).map(row => row.turnIndex);
    expect(after).toEqual(before);
  });

  it("counts when a boundary predates the stamp, and says the count is relative", () => {
    const rows = buildTranscriptRows([
      event(1, "turn.started"),
      event(2, "assistant.message", { text: "one" }),
    ]);
    expect(rows.map(row => row.turnIndex)).toEqual([1, 1]);
    // No stamp and more events before this window: the number is this
    // window's, so the pane must not present it as the session's.
    expect(turnNumbersAreAbsolute(rows, false)).toBe(false);
    // Same rows, but this window starts at the session's first event.
    expect(turnNumbersAreAbsolute(rows, true)).toBe(true);
  });

  it("vouches for the numbers as soon as one stamped boundary is loaded", () => {
    const rows = buildTranscriptRows([event(9001, "turn.started", { data: { turnIndex: 118 } })]);
    expect(turnNumbersAreAbsolute(rows, false)).toBe(true);
  });

  it("numbers from the boundaries and leaves the preamble at zero", () => {
    const rows = buildTranscriptRows([
      event(1, "session.status"),
      event(2, "turn.started"),
      event(3, "assistant.message", { text: "one" }),
      event(4, "turn.completed"),
      event(5, "turn.started"),
      event(6, "assistant.message", { text: "two" }),
    ]);
    expect(rows.map(row => row.turnIndex)).toEqual([0, 1, 1, 1, 2, 2]);
  });
});

describe("detail line", () => {
  it("shows a settled thought's text rather than the word completed", () => {
    const [row] = buildTranscriptRows([
      event(1, "reasoning.completed", { status: "completed", text: "The lock is held by the reader." }),
    ]);
    expect(row.detail).toBe("The lock is held by the reader.");
  });

  it("prefers a tool's title, which is what names the call", () => {
    const [row] = buildTranscriptRows([
      event(1, "tool.started", { title: "cargo test", text: "running" }),
    ]);
    expect(row.detail).toBe("cargo test");
  });
});

describe("filtering", () => {
  const rows = buildTranscriptRows([
    event(1, "assistant.message", { text: "the migration is additive" }),
    event(2, "tool.completed", { status: "failed", title: "cargo test" }),
    event(3, "tool.completed", { status: "completed", title: "cargo build" }),
  ]);

  it("composes the query with the selected facet", () => {
    expect(filterTranscriptRows(rows, "tools", "cargo")).toHaveLength(2);
    expect(filterTranscriptRows(rows, "tools", "build")).toHaveLength(1);
    expect(filterTranscriptRows(rows, "problems", "cargo")).toHaveLength(1);
    expect(filterTranscriptRows(rows, "messages", "cargo")).toHaveLength(0);
  });

  it("matches on the kind as well as the text, because a kind is what a reader knows", () => {
    expect(filterTranscriptRows(rows, "all", "tool.completed")).toHaveLength(2);
  });

  it("treats an all-whitespace query as no query", () => {
    expect(filterTranscriptRows(rows, "all", "   ")).toHaveLength(3);
  });
});
