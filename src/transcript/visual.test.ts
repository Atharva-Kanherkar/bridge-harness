import { describe, expect, it } from "vitest";
import { groupItems } from "./grouping";
import type { ConversationItem } from "./item";
import { isVisualItem, REFUSAL_PREFIX, visualCallInput, visualCallRefusal, visualCallState } from "./visual";

let sequence = 0;
const item = (data: Record<string, unknown>, overrides: Partial<ConversationItem> = {}): ConversationItem => {
  sequence += 1;
  return { key: `k${sequence}`, identity: `i${sequence}`, type: "activity", eventId: sequence, sequence, turn: 1, text: "", data, ...overrides };
};

const spec = { version: 1, title: "Spend", blocks: [] };

// The three shapes as each harness actually delivers them (captured live).
const claude = (overrides: Partial<ConversationItem> = {}) =>
  item({ type: "tool_use", id: "toolu_1", name: "mcp__bridge__visualize", input: spec }, { status: "completed", ...overrides });
const codex = (overrides: Partial<ConversationItem> = {}) =>
  item({ type: "mcpToolCall", id: "item_3", server: "bridge", tool: "visualize", arguments: spec, status: "completed" }, { status: "completed", ...overrides });
const opencode = (overrides: Partial<ConversationItem> = {}) =>
  item({ type: "tool", tool: "bridge_visualize", callID: "call_1", state: { status: "completed", input: spec, output: "Shown to the user inline: \"Spend\" (bar, 3 rows)." } }, { status: "completed", ...overrides });

describe("visual calls in the transcript", () => {
  it("claude_mcp__bridge__visualize_is_a_visual_call", () => {
    expect(isVisualItem(claude())).toBe(true);
    expect(visualCallInput(claude())).toEqual(spec);
  });

  it("codex_mcpToolCall_bridge_visualize_is_a_visual_call", () => {
    expect(isVisualItem(codex())).toBe(true);
    expect(visualCallInput(codex())).toEqual(spec);
  });

  it("opencode_bridge_visualize_is_a_visual_call", () => {
    expect(isVisualItem(opencode())).toBe(true);
    expect(visualCallInput(opencode())).toEqual(spec);
  });

  it("another_servers_visualize_is_not", () => {
    expect(isVisualItem(item({ name: "mcp__other__visualize", input: spec }))).toBe(false);
    expect(isVisualItem(item({ type: "mcpToolCall", server: "other", tool: "visualize" }))).toBe(false);
    expect(isVisualItem(item({ tool: "other_visualize" }))).toBe(false);
    expect(isVisualItem(item({ name: "visualize" }))).toBe(false);
  });

  it("the_spec_comes_from_input_arguments_or_state_input", () => {
    expect(visualCallInput(item({ type: "mcpToolCall", server: "bridge", tool: "visualize", arguments: JSON.stringify(spec) }))).toEqual(spec);
    // Claude streams the start of a call before its arguments.
    expect(visualCallInput(item({ name: "mcp__bridge__visualize", input: {} }, { status: "inProgress" }))).toBeUndefined();
  });

  it("a_visual_call_stands_alone_and_closes_the_run", () => {
    const before = item({ type: "commandExecution", command: "ls" });
    const visual = claude();
    const after = item({ type: "commandExecution", command: "pwd" });
    const rendered = groupItems([before, visual, after]);
    expect(rendered.map(entry => entry.kind)).toEqual(["group", "item", "group"]);
    expect(rendered[1].kind === "item" && rendered[1].item).toBe(visual);
  });

  it("a_failed_visual_call_reads_as_a_quiet_error_not_a_frame", () => {
    const refusal = `${REFUSAL_PREFIX} Fix every item below and call visualize once more:\n- version: must be 1`;
    // Claude marks the row failed; Codex and OpenCode may report it completed
    // with the refusal in the result, which is why the text is the backstop.
    expect(visualCallState(claude({ status: "failed", data: { name: "mcp__bridge__visualize", input: spec, aggregatedOutput: refusal } }))).toBe("refused");
    expect(visualCallState(codex({ data: { type: "mcpToolCall", server: "bridge", tool: "visualize", arguments: spec, result: { content: [{ type: "text", text: refusal }] } } }))).toBe("refused");
    expect(visualCallState(opencode({ data: { tool: "bridge_visualize", state: { status: "completed", input: spec, output: refusal } } }))).toBe("refused");
    expect(visualCallRefusal(claude({ status: "failed", data: { name: "mcp__bridge__visualize", aggregatedOutput: refusal } }))).toContain("- version: must be 1");
  });

  it("drawn and drawing states follow the item", () => {
    expect(visualCallState(claude())).toBe("drawn");
    expect(visualCallState(opencode())).toBe("drawn");
    expect(visualCallState(claude({ status: "inProgress" }))).toBe("drawing");
  });
});
