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
  ["context.compacted", "context.compacted"],
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

  /**
   * Which runtime a row came from is the row's fact, not the session's. Both
   * doors read it off `providerMeta` — live from the event, durably from the
   * copy `store::session_event_in_transaction` writes into the payload.
   */
  it("carries the originating adapter onto the envelope from both doors", () => {
    expect(normalizeAgentEvent(live("error", { text: "boom", providerMeta: { adapter: "codex" } })).envelope.adapter)
      .toBe("codex");
    expect(normalizeSessionEntry(durable("error", { text: "boom", providerMeta: { adapter: "codex" } }))?.envelope.adapter)
      .toBe("codex");
    // An entry stored before the stamp existed simply has none; nothing guesses.
    expect(normalizeSessionEntry(durable("error", { text: "boom" }))?.envelope.adapter).toBeUndefined();
  });

  it("reads a live error's text from the same places its replay does", () => {
    // The two doors used to disagree: live read only `text`, replay fell back
    // to `data.error.message`. One failure then said two different things —
    // and the merge, which matches errors on their words, kept both rows.
    const nested = { data: { error: { message: "You've hit your usage limit." } } };
    expect(normalizeAgentEvent(live("error", nested))).toMatchObject({ type: "error", text: "You've hit your usage limit." });
    expect(normalizeSessionEntry(durable("error", nested))).toMatchObject({ type: "error", text: "You've hit your usage limit." });
  });

  it("keeps a replayed raw provider frame inspectable", () => {
    expect(normalizeSessionEntry(durable("provider.unknown", { title: "frame" }, { contextVisibility: "worker_raw" })))
      .toMatchObject({ type: "raw", inspectable: true, title: "frame" });
  });

  it("draws the compacted card from the harness's own boundary, live and replayed", () => {
    // The card exists to say the provider's context actually shrank, so it is
    // drawn from the provider's own report and nowhere else.
    const facts = { harness: "claude", trigger: "auto", preTokens: 184_000, postTokens: 22_500 };
    const liveEvent = normalizeAgentEvent(live("context.compacted", { status: "completed", data: facts }));
    expect(liveEvent).toMatchObject({
      type: "context.compacted",
      harness: "claude",
      trigger: "auto",
      preTokens: 184_000,
      postTokens: 22_500,
      title: "Context compacted",
      text: "Claude summarised its context, 184k tokens down to 23k tokens.",
    });

    // The durable twin reads the same, from the stored `data` wrapper.
    const replayed = normalizeSessionEntry(durable("context.compacted", { status: "completed", title: "Context compacted", data: facts }));
    expect(replayed).toMatchObject({
      type: "context.compacted",
      preTokens: 184_000,
      postTokens: 22_500,
      title: "Context compacted",
      text: "Claude summarised its context, 184k tokens down to 23k tokens.",
    });
  });

  it("names a token figure only when the provider sent one", () => {
    // Codex and OpenCode report a boundary with no numbers at all. A zero
    // rendered as a figure would read as "the context shrank to nothing".
    for (const harness of ["codex", "opencode"]) {
      const event = normalizeAgentEvent(live("context.compacted", { data: { harness } }));
      expect(event).toMatchObject({ type: "context.compacted", preTokens: undefined, postTokens: undefined });
      expect((event as { text: string }).text).not.toMatch(/\d/);
    }
    expect(normalizeAgentEvent(live("context.compacted", { data: { harness: "codex" } })))
      .toMatchObject({ text: "Codex summarised its own context." });

    // A boundary that summarized everything reports no post figure.
    expect(normalizeAgentEvent(live("context.compacted", { data: { harness: "claude", preTokens: 90_000 } })))
      .toMatchObject({ text: "Claude summarised its context at 90k tokens." });
  });

  it("does not let a Bridge checkpoint claim the context shrank", () => {
    // Committing a Bridge checkpoint writes forest entries and moves the
    // session head. It never touches the adapter, so on a hot session the
    // provider's context is exactly as full as it was.
    expect(normalizeSessionEntry(durable("compaction", { summary: "Kept the API stable", reason: "phase_boundary" })))
      .toMatchObject({ type: "compaction", phase: "completed", title: "Checkpoint saved", text: "Kept the API stable" });
    expect(normalizeSessionEntry(durable("compaction.requested", { reason: "manual" })))
      .toMatchObject({ type: "compaction", phase: "requested", title: "Compaction requested" });
    expect(normalizeSessionEntry(durable("compaction.failed", { reason: "timed out", message: "The summary did not arrive" })))
      .toMatchObject({ type: "compaction", phase: "failed", title: "Compaction failed" });
  });

  it("fails closed on an unsupported future semantic event schema", () => {
    expect(() => normalizeSessionEntry(durable("assistant.message", { text: "x" }, { semanticSchemaVersion: 3 })))
      .toThrow("Unsupported semantic event schema version 3");
  });
});
