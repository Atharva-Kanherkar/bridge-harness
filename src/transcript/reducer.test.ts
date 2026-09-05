import { describe, expect, it, vi } from "vitest";
import { normalizeAgentEvent, normalizeSessionEntry } from "./codec";
import { reduceTranscript } from "./reducer";
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

  it("is a pure function of its input", () => {
    const events = [
      live(1, "message.completed", { itemId: "m", role: "assistant", text: "hi", status: "completed" }),
      live(2, "command.started", { itemId: "c", title: "bun test", status: "inProgress" }),
    ].map(normalizeAgentEvent);
    const before = JSON.stringify(events);
    expect(reduceTranscript(events)).toEqual(reduceTranscript(events));
    expect(JSON.stringify(events)).toBe(before);
  });
});
