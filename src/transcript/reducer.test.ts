import { describe, expect, it, vi } from "vitest";
import { normalizeAgentEvent, normalizeSessionEntry } from "./codec";
import { reduceTranscript } from "./reducer";
import { acpOtherStream, durableEntriesFrom } from "./golden";
import { asWireKind } from "./wire";
import type { AgentEvent, SessionEntry } from "../types";

const live = (id: number, kind: string, overrides: Partial<AgentEvent> = {}): AgentEvent => ({
  id, sessionId: "s", sequence: id, protocolVersion: 1, kind: asWireKind(kind),
  itemId: null, role: null, status: null, title: null, text: null, data: {},
  providerMeta: {}, createdAt: "now", ...overrides,
});

const reduce = (events: AgentEvent[]) => reduceTranscript(events.map(normalizeAgentEvent));

const entry = (id: string, parentEntryId: string | null, kind: string, payload: Record<string, unknown>, sequence: number): SessionEntry => ({
  id, sessionId: "s", parentEntryId, sequence, semanticSchemaVersion: 2, kind, payload,
  providerEventId: null, contextVisibility: "eligible", tokenEstimate: null, createdAt: "now",
});

describe("reduceTranscript", () => {
  it("folds a tool lifecycle into one row", () => {
    const items = reduce([
      live(1, "command.started", { itemId: "c", title: "bun test", status: "inProgress", data: { type: "commandExecution", command: "bun test" } }),
      live(2, "command.output_delta", { itemId: "c", sequence: 0, text: "pass 1\n", status: "inProgress" }),
      live(3, "command.output_delta", { itemId: "c", sequence: 0, text: "pass 2\n", status: "inProgress" }),
      live(4, "command.completed", { itemId: "c", status: "completed", data: { type: "commandExecution", exitCode: 0 } }),
    ]);
    expect(items).toHaveLength(1);
    expect(items[0]).toMatchObject({ type: "activity", status: "completed", text: "pass 1\npass 2\n" });
    expect(items[0].tool).toMatchObject({ verb: "run", command: "bun test", exitCode: 0 });
  });

  it("keeps a subagent stamp on tool, message and reasoning rows", () => {
    const subagent = { sessionId: "ses_child", agent: "general", title: "Look up the facts" };
    const items = reduce([
      live(1, "turn.started", { status: "working" }),
      live(2, "tool.started", { itemId: "task", title: "Look up the facts", status: "inProgress", data: { tool: "task" } }),
      live(3, "reasoning.completed", { itemId: "r1", status: "completed", text: "Reading the file.", data: { subagent } }),
      live(4, "command.completed", { itemId: "c1", title: "cat facts.txt", status: "completed", data: { command: "cat facts.txt", subagent } }),
      live(5, "message.completed", { itemId: "m1", role: "assistant", status: "completed", text: "The answer is 42.", data: { subagent } }),
      live(6, "message.completed", { itemId: "m2", role: "assistant", status: "completed", text: "It is 42.", data: {} }),
    ]);
    const stamped = items.filter(item => item.data.subagent !== undefined).map(item => item.type);
    expect(stamped).toEqual(["reasoning", "activity", "message"]);
    expect(items.find(item => item.itemId === "task")?.data.subagent).toBeUndefined();
    expect(items.find(item => item.itemId === "m2")?.data.subagent).toBeUndefined();
    expect(items.find(item => item.itemId === "c1")?.data.subagent).toEqual(subagent);
  });

  it("folds a replayed lifecycle by item id, across two forest entries", () => {
    const items = reduceTranscript([
      normalizeSessionEntry(entry("e1", null, "command.started", { itemId: "c", status: "inProgress", data: { command: "bun test" } }, 1))!,
      normalizeSessionEntry(entry("e2", "e1", "command.completed", { itemId: "c", status: "completed", data: { command: "bun test", exitCode: 0 } }, 2))!,
    ]);
    expect(items).toHaveLength(1);
    expect(items[0]).toMatchObject({ key: "entry:e1", status: "completed" });
  });

  it("folds an approval resolution onto its request and keeps the request's own id", () => {
    const items = reduce([
      live(7, "approval.requested", { title: "Approve command", status: "pending" }),
      live(8, "approval.resolved", { data: { requestEventId: 7, decision: "accept" } }),
    ]);
    expect(items).toEqual([expect.objectContaining({ type: "approval", eventId: 7, status: "accept" })]);
  });

  it("drops an answer whose request is not on this branch", () => {
    // A bare "resolved" row says nothing on its own, and a live window that
    // opened after the request would otherwise grow one.
    expect(reduce([live(8, "approval.resolved", { data: { requestEventId: 7, decision: "accept" } })])).toEqual([]);
  });

  it("does not let an approval join the tool call it is about", () => {
    // Claude sends the permission request under the tool's own item id. Sharing
    // the row would make the command answer for the approval and give both the
    // same identity in the live/durable merge.
    const items = reduce([
      live(1, "command.started", { itemId: "tool-1", title: "bun test", status: "inProgress" }),
      live(2, "permission.requested", { itemId: "tool-1", title: "Run this command?", status: "pending" }),
      live(3, "command.completed", { itemId: "tool-1", status: "completed" }),
    ]);
    expect(items.map(item => item.type)).toEqual(["activity", "permission"]);
    expect(items[0].itemId).toBe("tool-1");
    expect(items[1].itemId).toBeUndefined();
  });

  it("settles a thinking.started row of default status when the turn ends", () => {
    const items = reduce([
      live(1, "reasoning.started", { itemId: "r1", text: "Thinking deeply..." }),
      live(2, "turn.completed", { status: "completed" }),
    ]);
    expect(items).toEqual([expect.objectContaining({ type: "reasoning", status: "completed", text: "Thinking deeply..." })]);
  });

  it("settles streaming thinking when the turn ends", () => {
    const items = reduce([
      live(1, "reasoning.delta", { sequence: 0, text: "Thinking deeply..." }),
      live(2, "turn.completed", { status: "completed" }),
    ]);
    expect(items).toEqual([expect.objectContaining({ type: "reasoning", status: "completed", text: "Thinking deeply..." })]);
  });

  it("keeps separate thinking cards across turns when the provider names none", () => {
    const items = reduce([
      live(1, "turn.started"), live(2, "reasoning.delta", { sequence: 0, text: "Turn 1" }), live(3, "turn.completed"),
      live(4, "turn.started"), live(5, "reasoning.delta", { sequence: 0, text: "Turn 2" }), live(6, "turn.completed"),
    ]);
    expect(items.map(item => item.text)).toEqual(["Turn 1", "Turn 2"]);
  });

  it("orders sequence-0 frames where they streamed, not above the turn", () => {
    const items = reduce([
      live(0, "message.delta", { sequence: 0, itemId: "m1", role: "assistant", text: "Let me check." }),
      live(41, "tool.started", { itemId: "t1", title: "sqlite3", status: "inProgress" }),
      live(42, "tool.completed", { itemId: "t1", status: "completed" }),
      live(0, "message.delta", { sequence: 0, itemId: "m2", role: "assistant", text: "Found it." }),
    ]);
    expect(items.map(item => item.type)).toEqual(["message", "activity", "message"]);
  });

  it("surfaces an unknown event as a raw row, never as activity", () => {
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    const items = reduce([live(1, "telepathy.received", { data: { thought: "hi" } })]);
    expect(items).toEqual([expect.objectContaining({
      type: "raw",
      title: "Unrecognized event: telepathy.received",
      data: expect.objectContaining({ wireKind: "telepathy.received", collapsed: true, inspectable: true }),
    })]);
    vi.restoreAllMocks();
  });

  /* ── Turn stamping ─────────────────────────────────────────────────────
     The index has to come from something the forest also stores. A turn
     marker is transient — sequence 0, never persisted — so counting those
     alone would give a reload a different set of turns than the live window
     the reader just watched. */

  it("stamps every row with the turn its user message opened", () => {
    const items = reduce([
      live(1, "message.completed", { itemId: "u1", role: "user", text: "first", status: "completed" }),
      live(2, "command.started", { itemId: "c1", title: "ls", status: "inProgress" }),
      live(3, "message.completed", { itemId: "a1", role: "assistant", text: "done", status: "completed" }),
      live(4, "message.completed", { itemId: "u2", role: "user", text: "second", status: "completed" }),
      live(5, "command.started", { itemId: "c2", title: "pwd", status: "inProgress" }),
    ]);
    expect(items.map(item => [item.itemId, item.turn])).toEqual([
      ["u1", 1], ["c1", 1], ["a1", 1], ["u2", 2], ["c2", 2],
    ]);
  });

  it("does not count the same boundary twice when a turn marker follows the user", () => {
    const items = reduce([
      live(1, "message.completed", { itemId: "u1", role: "user", text: "go", status: "completed" }),
      live(0, "turn.started", { sequence: 0 }),
      live(2, "command.started", { itemId: "c1", title: "ls", status: "inProgress" }),
    ]);
    expect(items.map(item => item.turn)).toEqual([1, 1]);
  });

  it("lets a turn marker open a turn the user did not ask for", () => {
    // A continuation the model started on its own still separates its work
    // from the turn before it.
    const items = reduce([
      live(1, "message.completed", { itemId: "u1", role: "user", text: "go", status: "completed" }),
      live(0, "turn.started", { sequence: 0 }),
      live(2, "command.started", { itemId: "c1", title: "ls", status: "inProgress" }),
      live(0, "turn.started", { sequence: 0 }),
      live(3, "command.started", { itemId: "c2", title: "pwd", status: "inProgress" }),
    ]);
    expect(items.map(item => item.turn)).toEqual([1, 1, 2]);
  });

  it("counts one turn once when the marker opens it before the user's message", () => {
    // Codex announces the turn and then echoes the prompt inside it. Counting
    // both frames numbered one real turn twice, so a run split in half.
    const items = reduce([
      live(0, "turn.started", { sequence: 0 }),
      live(1, "message.completed", { itemId: "u1", role: "user", text: "go", status: "completed" }),
      live(2, "command.started", { itemId: "c1", title: "ls", status: "inProgress" }),
      live(0, "turn.completed", { sequence: 0 }),
      live(0, "turn.started", { sequence: 0 }),
      live(3, "message.completed", { itemId: "u2", role: "user", text: "again", status: "completed" }),
      live(4, "command.started", { itemId: "c2", title: "pwd", status: "inProgress" }),
    ]);
    expect(items.map(item => item.turn)).toEqual([1, 1, 2, 2]);
  });

  it("counts one turn once when the user's message opens it before the marker", () => {
    const items = reduce([
      live(1, "message.completed", { itemId: "u1", role: "user", text: "go", status: "completed" }),
      live(0, "turn.started", { sequence: 0 }),
      live(2, "command.started", { itemId: "c1", title: "ls", status: "inProgress" }),
      live(0, "turn.completed", { sequence: 0 }),
      live(3, "message.completed", { itemId: "u2", role: "user", text: "again", status: "completed" }),
      live(0, "turn.started", { sequence: 0 }),
      live(4, "command.started", { itemId: "c2", title: "pwd", status: "inProgress" }),
    ]);
    expect(items.map(item => item.turn)).toEqual([1, 1, 2, 2]);
  });

  it("counts markers alone when no user message accompanies them", () => {
    const items = reduce([
      live(0, "turn.started", { sequence: 0 }),
      live(1, "command.started", { itemId: "c1", title: "ls", status: "inProgress" }),
      live(0, "turn.completed", { sequence: 0 }),
      live(0, "turn.started", { sequence: 0 }),
      live(2, "command.started", { itemId: "c2", title: "pwd", status: "inProgress" }),
    ]);
    expect(items.map(item => item.turn)).toEqual([1, 2]);
  });

  // The fourth shape — a durable branch of user messages with no markers at
  // all — is the case below: the forest never stores a turn marker.
  it("agrees with the durable projection about which turn a row belongs to", () => {
    const persisted = [
      entry("e1", null, "user.message", { itemId: "u1", role: "user", text: "go", status: "completed" }, 1),
      entry("e2", "e1", "command.started", { itemId: "c1", status: "inProgress", data: { command: "ls" } }, 2),
      entry("e3", "e2", "user.message", { itemId: "u2", role: "user", text: "again", status: "completed" }, 3),
      entry("e4", "e3", "command.started", { itemId: "c2", status: "inProgress", data: { command: "pwd" } }, 4),
    ];
    const items = reduceTranscript(persisted.map(value => normalizeSessionEntry(value)!));
    expect(items.map(item => item.turn)).toEqual([1, 1, 2, 2]);
  });

  it("is a pure function of its input", () => {
    const events = [
      live(1, "message.completed", { itemId: "m", role: "assistant", text: "hi", status: "completed" }),
      live(2, "command.started", { itemId: "c", title: "bun test", status: "inProgress" }),
    ].map(normalizeAgentEvent);
    const before = JSON.stringify(events);
    expect(reduceTranscript(events)).toEqual(reduceTranscript(events));
    expect(JSON.stringify(events)).toBe(before);
  });

  it("keeps the harness's own boundary out of Bridge's maintenance window", () => {
    // A Bridge checkpoint request opens a window in which the model's prose is
    // protocol traffic rather than conversation. A native compaction is not
    // part of that exchange, so it must neither open the window nor close one
    // a checkpoint request opened, which would let the checkpoint's own reply
    // land in the transcript as a message.
    const items = reduce([
      live(1, "compaction.requested", { data: { reason: "phase_boundary" } }),
      live(2, "context.compacted", { status: "completed", data: { harness: "claude", preTokens: 180_000, postTokens: 20_000 } }),
      live(3, "message.completed", { role: "assistant", text: '{"schemaVersion":1,"summary":"internal"}' }),
    ]);
    expect(items.map(item => item.type)).toEqual(["compaction", "context-compacted"]);
    expect(items.find(item => item.type === "context-compacted")).toMatchObject({
      title: "Context compacted",
      text: "Claude summarised its context, 180k tokens down to 20k tokens.",
    });
  });

  it("draws a native boundary on its own row, never folded into a Bridge one", () => {
    const items = reduce([
      live(1, "context.compacted", { status: "completed", data: { harness: "codex" } }),
      live(2, "compaction.requested", { data: { reason: "manual" } }),
      live(3, "compaction", { data: { summary: "Saved the plan", reason: "manual" } }),
    ]);
    // A request and the boundary it commits are already two rows. What this
    // pins is that neither of them absorbs the native one.
    expect(items.map(item => item.type)).toEqual(["context-compacted", "compaction", "compaction"]);
    expect(items[1]).toMatchObject({ title: "Compaction requested" });
    expect(items[2]).toMatchObject({ title: "Checkpoint saved" });
  });
});


describe("ACP title-less updates", () => {
  it("retains the start title through progress and completion, live and replayed", () => {
    const frames = acpOtherStream();
    for (const count of [1, 2, 3]) {
      const stream = frames.slice(0, count);
      const liveRows = reduce(stream);
      const replayRows = reduceTranscript(durableEntriesFrom("cursor", stream).map(entry => normalizeSessionEntry(entry)!));
      for (const rows of [liveRows, replayRows]) {
        expect(rows).toHaveLength(1);
        expect(rows[0]).toMatchObject({ title: "Resolve project context", tool: {
          doing: "Running: Resolve project context", done: "Finished: Resolve project context",
          status: count === 3 ? "completed" : "running",
        } });
        expect(rows[0].tool?.target).toBeUndefined();
      }
      expect(replayRows[0].tool).toEqual(liveRows[0].tool);
    }
  });
});
