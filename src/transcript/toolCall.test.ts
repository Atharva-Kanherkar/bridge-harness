import { describe, expect, it } from "vitest";
import { readToolCall } from "./toolCall";

describe("fallback tool action labels", () => {
  it("uses a trimmed title for the provider-neutral fallback", () => {
    const tool = readToolCall({ title: "  Resolve project context\n", text: "", status: "inProgress", surface: "activity", data: { type: "unknownAction" } });
    expect(tool).toMatchObject({ doing: "Running: Resolve project context", done: "Finished: Resolve project context", status: "running" });
    expect(tool.target).toBeUndefined();
  });

  it.each([undefined, "", "   "])("keeps an honest fallback when no action is named (%s)", (title) => {
    const tool = readToolCall({ title, text: "", surface: "activity", data: {} });
    expect(tool).toMatchObject({ doing: "Using a tool", done: "Used a tool" });
    expect(tool.target).toBeUndefined();
  });

  it.each([
    ["think", "Thinking", "Thought", "brain"],
    ["switch_mode", "Switching mode", "Switched mode", "navigation"],
  ])("classifies ACP %s", (kind, doing, done, glyph) => {
    expect(readToolCall({ title: "Plan", text: "", surface: "activity", data: { kind } }))
      .toMatchObject({ doing, done, glyph, target: "Plan" });
  });

  it("says a Claude tool is being prepared until its arguments have landed", () => {
    // content_block_start: Bash named, no input yet, nothing running.
    const preparing = readToolCall({ title: "Bash", text: "", status: "inProgress", surface: "activity", data: { type: "tool_use", name: "Bash", input: {}, phase: "preparing" } });
    expect(preparing).toMatchObject({ doing: "Preparing", target: "Bash", status: "running" });
    // The snapshot carries the command and the tool actually runs.
    const running = readToolCall({ title: "gh pr create", text: "", status: "inProgress", surface: "activity", data: { type: "tool_use", name: "Bash", input: { command: "gh pr create" }, phase: "running" } });
    expect(running).toMatchObject({ doing: "Running", target: "gh pr create" });
  });

  it("preserves tool names and recognized categories", () => {
    expect(readToolCall({ title: "Context", text: "", surface: "activity", data: { name: "lookup_context" } }))
      .toMatchObject({ doing: "Using lookup_context", done: "Used lookup_context" });
    expect(readToolCall({ title: "project", text: "", surface: "activity", data: { kind: "read" } }))
      .toMatchObject({ doing: "Reading", done: "Read", target: "project" });
  });
});
