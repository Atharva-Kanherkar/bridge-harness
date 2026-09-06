import { afterEach, describe, expect, it, vi } from "vitest";
import { normalizeAgentEvent, normalizeSessionEntry } from "./codec";
import { asWireKind } from "./wire";
import type { AgentEvent, SessionEntry } from "../types";

const live = (kind: string, overrides: Partial<AgentEvent> = {}): AgentEvent => ({
  id: 1, sessionId: "s", sequence: 1, protocolVersion: 1, kind: asWireKind(kind),
  itemId: null, role: null, status: null, title: null, text: null, data: {},
  providerMeta: {}, createdAt: "now", ...overrides,
});

const durable = (kind: string, payload: Record<string, unknown> = {}, overrides: Partial<SessionEntry> = {}): SessionEntry => ({
  id: "e1", sessionId: "s", parentEntryId: null, sequence: 1, semanticSchemaVersion: 2,
  kind, payload, providerEventId: null, contextVisibility: "eligible",
  tokenEstimate: null, createdAt: "now", ...overrides,
});

/**
 * The vocabulary the Rust normalizers and the durable writer own, and the
 * variant each kind has to land on. Table-driven because the point of the
 * codec is that this table is the whole of it: if a kind is missing here it is
 * missing from the codec, and the transcript will say so out loud.
 */
const VOCABULARY: [kind: string, type: string][] = [
  ["message.delta", "message.delta"],
  ["message.completed", "message.completed"],
  ["reasoning.delta", "thinking.delta"],
  ["reasoning.started", "thinking.started"],
  ["reasoning.completed", "thinking.completed"],
  ["reasoning", "thinking.completed"],
  ["tool.started", "tool.started"],
  ["tool.progress", "tool.progress"],
  ["tool.completed", "tool.completed"],
  ["command.started", "tool.started"],
  ["command.output_delta", "tool.progress"],
  ["command.completed", "tool.completed"],
  ["command.failed", "tool.completed"],
  ["file_change.started", "tool.started"],
  ["file_change.completed", "tool.completed"],
  ["diff.delta", "tool.progress"],
  ["diff.updated", "tool.completed"],
  ["item.started", "tool.started"],
  ["item.completed", "tool.completed"],
  ["plan.updated", "plan.updated"],
  ["approval.requested", "approval.requested"],
  ["approval.resolved", "approval.resolved"],
  ["approval.settled", "notice"],
  ["permission.requested", "interaction.requested"],
  ["permission.resolving", "interaction.settled"],
  ["permission.resolved", "interaction.settled"],
  ["question.requested", "interaction.requested"],
  ["question.settled", "interaction.settled"],
  ["question.resolved", "interaction.settled"],
  ["delegation.spawned", "delegation.updated"],
  ["delegation.resumed", "delegation.updated"],
  ["delegation.blocked", "delegation.updated"],
  ["delegation.rejected", "delegation.updated"],
  ["delegation.steered", "delegation.updated"],
  ["delegation.result", "delegation.updated"],
  ["worker.result", "delegation.updated"],
  ["artifact.created", "artifact.ready"],
  ["checkpoint", "checkpoint"],
  ["compaction", "compaction"],
  ["compaction.requested", "compaction"],
  ["compaction.failed", "compaction"],
  ["branch.summary", "branch.summary"],
  ["handoff.brief", "notice"],
  ["error", "error"],
  ["runtime.failed", "error"],
  ["turn.started", "turn.started"],
  ["turn.completed", "turn.completed"],
  ["turn.failed", "turn.completed"],
  ["usage.updated", "usage"],
  ["session.started", "session.lifecycle"],
  ["session.idle", "session.lifecycle"],
  ["session.model_changed", "model.change"],
  ["provider.unknown", "raw"],
  ["workspace.stale_base", "notice"],
  ["model.rerouted", "notice"],
  ["mode.updated", "notice"],
  ["commands.updated", "notice"],
  ["config.updated", "notice"],
  ["todo.updated", "notice"],
  ["extension.handled", "notice"],
  ["effort.changed", "notice"],
];

afterEach(() => vi.restoreAllMocks());

describe("normalizeAgentEvent", () => {
  it.each(VOCABULARY)("maps %s to %s", (kind, type) => {
    expect(normalizeAgentEvent(live(kind)).type).toBe(type);
  });

  it("reports an unfamiliar kind instead of calling it activity", () => {
    const reported = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const event = normalizeAgentEvent(live("telepathy.received", { data: { thought: "hi" } }));
    expect(event.type).toBe("unknown");
    expect(event).toMatchObject({ wireKind: "telepathy.received", raw: { thought: "hi" } });
    expect(reported).toHaveBeenCalledWith(expect.stringContaining("telepathy.received"));
  });

  it("reports an unmapped kind once, not once per flush", () => {
    // reduceConversation re-normalizes the whole live window on every 50ms
    // flush; a kind nothing here has a name for must not log once per flush.
    const reported = vi.spyOn(console, "error").mockImplementation(() => undefined);
    normalizeAgentEvent(live("phantom.sighted"));
    normalizeAgentEvent(live("phantom.sighted"));
    expect(reported).toHaveBeenCalledTimes(1);
  });

  it("keeps a live raw provider frame out of the transcript", () => {
    // It has no stable identity to reconcile against the durable twin the
    // forest holds a moment later, so drawing it would double every raw row.
    expect(normalizeAgentEvent(live("provider.unknown"))).toMatchObject({ type: "raw", inspectable: false });
  });

  it("reads Codex summary-only reasoning as the thought text", () => {
    const event = normalizeAgentEvent(live("reasoning.completed", { data: { summary: "the rest of the thought" } }));
    expect(event).toMatchObject({ type: "thinking.completed", text: "", summary: "the rest of the thought" });
  });

  it("reads an exit code wherever a provider puts it", () => {
    const exitOf = (data: Record<string, unknown>) => {
      const event = normalizeAgentEvent(live("command.completed", { itemId: "c", data }));
      return event.type === "tool.completed" ? event.tool.exitCode : undefined;
    };
    expect(exitOf({ exitCode: 0 })).toBe(0);
    expect(exitOf({ exit_code: 2 })).toBe(2);
    expect(exitOf({ state: { metadata: { exit: 127 } } })).toBe(127);
    expect(exitOf({})).toBeUndefined();
    expect(exitOf({ exitCode: "boom" })).toBeUndefined();
  });

  it("reads a patch wherever a provider hangs it", () => {
    const patchOf = (data: Record<string, unknown>, text?: string) => {
      const event = normalizeAgentEvent(live("file_change.completed", { itemId: "f", data, text: text ?? null }));
      return event.type === "tool.completed" ? event.tool.patch : undefined;
    };
    const patch = "@@ -1,2 +1,2 @@\n-a\n+b";
    expect(patchOf({ patch })).toBe(patch);
    expect(patchOf({ state: { diff: patch } })).toBe(patch);
    expect(patchOf({ changes: [{ path: "a.rs", diff: patch }, { path: "b.rs", diff: "@@ -9 +9 @@\n+c" }] }))
      .toBe(`${patch}\n@@ -9 +9 @@\n+c`);
    expect(patchOf({}, patch)).toBe(patch);
    // ACP describes an edit as before/after text, not as a patch.
    expect(patchOf({ kind: "edit", update: { content: [{ type: "diff", path: "a.rs", oldText: "a", newText: "b" }] } }))
      .toContain("+b");
  });

  it("never claims a patch for a read, whose output is only ever output", () => {
    const patch = "@@ -1,2 +1,2 @@\n-a\n+b";
    const event = normalizeAgentEvent(live("tool.completed", { itemId: "t", text: patch, data: { name: "Read", input: { file_path: "a.rs" } } }));
    expect(event.type === "tool.completed" && event.tool.patch).toBeUndefined();
    expect(event.type === "tool.completed" && event.tool.output).toBe(patch);
  });
});

describe("normalizeSessionEntry", () => {
  it("re-encodes a durable tool entry into the same variant as its live twin", () => {
    const liveEvent = normalizeAgentEvent(live("command.started", { id: 9, itemId: "tool-1", status: "inProgress" }));
    const replayed = normalizeSessionEntry(durable("command.started", { itemId: "tool-1", status: "inProgress" }));
    expect(replayed?.type).toBe(liveEvent.type);
    expect(replayed?.envelope.itemId).toBe(liveEvent.envelope.itemId);
    expect(replayed?.envelope.origin).toBe("durable");
  });

  it("turns a stored user turn into a message, which the live channel never sees", () => {
    expect(normalizeSessionEntry(durable("user.message", { text: "hi" })))
      .toMatchObject({ type: "message.completed", role: "user", text: "hi" });
  });

  it("flattens the stored wrapper so a replayed row reads like a live one", () => {
    const event = normalizeSessionEntry(durable("command.completed", { itemId: "c", status: "completed", data: { command: "bun test", exitCode: 1 } }));
    expect(event?.envelope.providerData).toMatchObject({ command: "bun test", exitCode: 1 });
  });

  it("drops session lifecycle plumbing rather than replaying it as a tool row", () => {
    expect(normalizeSessionEntry(durable("session.started", { status: "ready" }))).toBeNull();
    expect(normalizeSessionEntry(durable("session.status", { status: "working" }))).toBeNull();
    expect(normalizeSessionEntry(durable("session.model_changed", { title: "Chat model changed" })))
      .toMatchObject({ type: "model.change" });
  });

  it("keeps a replayed raw provider frame inspectable", () => {
    expect(normalizeSessionEntry(durable("provider.unknown", { title: "frame" }, { contextVisibility: "worker_raw" })))
      .toMatchObject({ type: "raw", inspectable: true, title: "frame" });
  });

  it("fails closed on an unsupported future semantic event schema", () => {
    expect(() => normalizeSessionEntry(durable("assistant.message", { text: "x" }, { semanticSchemaVersion: 3 })))
      .toThrow("Unsupported semantic event schema version 3");
  });
});
