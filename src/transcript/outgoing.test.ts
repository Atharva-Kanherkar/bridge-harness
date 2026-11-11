import { describe, expect, it } from "vitest";
import { groupItems } from "./grouping";
import type { ConversationItem } from "./item";
import { isOutgoingItem, outgoingCallState, outgoingDraftId, outgoingRefusal } from "./outgoing";

let sequence = 0;
const item = (data: Record<string, unknown>, overrides: Partial<ConversationItem> = {}): ConversationItem => {
  sequence += 1;
  return { key: `k${sequence}`, identity: `i${sequence}`, type: "activity", eventId: sequence, sequence, turn: 1, text: "", data, ...overrides };
};

const args = { to: { slack: { channel: "#dev" } }, body: "Deploy is done", reason: "tell the team" };
const answer = { id: "o-1", state: "draft", destination: "Slack · #dev", rendered: "Deploy is done", lint: [], note: "..." };

// The three shapes as each harness delivers a bridge tool call (captured live
// from notes_search, which rides the same route).
const claude = (output: string, overrides: Partial<ConversationItem> = {}) =>
  item({ type: "tool_use", id: "toolu_1", name: "mcp__bridge__outgoing_draft", input: args, tool_use_id: "toolu_1", content: output, aggregatedOutput: output }, { status: "completed", ...overrides });
const codex = (result: Record<string, unknown>, overrides: Partial<ConversationItem> = {}) =>
  item({ type: "mcpToolCall", id: "exec-1", server: "bridge", tool: "outgoing_draft", arguments: args, status: "completed", result }, { status: "completed", ...overrides });
const opencode = (state: Record<string, unknown>, overrides: Partial<ConversationItem> = {}) =>
  item({ type: "tool", tool: "bridge_outgoing_draft", callID: "call_1", state }, { status: "completed", ...overrides });

describe("outgoing draft calls in the transcript", () => {
  it("detects_all_three_harness_shapes", () => {
    expect(isOutgoingItem(claude(JSON.stringify(answer)))).toBe(true);
    expect(isOutgoingItem(codex({}))).toBe(true);
    expect(isOutgoingItem(opencode({ status: "running" }))).toBe(true);
  });

  it("ignores_another_servers_outgoing_draft", () => {
    expect(isOutgoingItem(item({ name: "mcp__other__outgoing_draft", input: args }))).toBe(false);
    expect(isOutgoingItem(item({ type: "mcpToolCall", server: "other", tool: "outgoing_draft" }))).toBe(false);
    expect(isOutgoingItem(item({ type: "tool", tool: "outgoing_draft" }))).toBe(false);
  });

  it("extracts_the_id_from_each_harness", () => {
    expect(outgoingDraftId(claude(JSON.stringify(answer)))).toBe("o-1");
    expect(outgoingDraftId(codex({ content: [{ type: "text", text: JSON.stringify(answer, null, 2) }] }))).toBe("o-1");
    expect(outgoingDraftId(codex({ structuredContent: answer, content: [] }))).toBe("o-1");
    expect(outgoingDraftId(opencode({ status: "completed", input: args, output: JSON.stringify(answer) }))).toBe("o-1");
  });

  it("is_drafting_until_the_call_answers", () => {
    const pending = item({ type: "tool_use", id: "toolu_2", name: "mcp__bridge__outgoing_draft", input: {} }, { status: "inProgress" });
    expect(outgoingCallState(pending)).toBe("drafting");
    expect(outgoingCallState(claude(JSON.stringify(answer)))).toBe("drafted");
  });

  it("a_settled_call_without_a_draft_is_refused_with_its_reason", () => {
    const refused = claude("to must be exactly one of ...");
    expect(outgoingCallState(refused)).toBe("refused");
    expect(outgoingRefusal(refused)).toBe("to must be exactly one of ...");
    expect(outgoingCallState(codex({ isError: true, content: [{ type: "text", text: "Only a top-level chat can draft messages." }] }))).toBe("refused");
  });

  it("a_draft_stands_on_its_own_between_tool_runs", () => {
    const before = item({ type: "commandExecution", command: "ls" });
    const draft = claude(JSON.stringify(answer));
    const after = item({ type: "commandExecution", command: "pwd" });
    const rendered = groupItems([before, draft, after]);
    expect(rendered.map(entry => entry.kind)).toEqual(["group", "item", "group"]);
    expect(rendered[1].kind === "item" && rendered[1].item).toBe(draft);
  });
});
