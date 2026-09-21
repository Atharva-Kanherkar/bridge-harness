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

  it("preserves tool names and recognized categories", () => {
    expect(readToolCall({ title: "Context", text: "", surface: "activity", data: { name: "lookup_context" } }))
      .toMatchObject({ doing: "Using lookup_context", done: "Used lookup_context" });
    expect(readToolCall({ title: "project", text: "", surface: "activity", data: { kind: "read" } }))
      .toMatchObject({ doing: "Reading", done: "Read", target: "project" });
  });
});

describe("harness subagent facet (issue #667)", () => {
  it("reads agent type, description and prompt off a Task call", () => {
    const tool = readToolCall({
      title: "Task", text: "", status: "inProgress", surface: "activity",
      data: { name: "Task", input: { description: "Explore auth", prompt: "Map the login flow", subagent_type: "Explore" } },
    });
    expect(tool).toMatchObject({ verb: "tool", glyph: "fork", doing: "Delegating", target: "Explore auth" });
    expect(tool.subagent).toEqual({ agentType: "Explore", description: "Explore auth", prompt: "Map the login flow" });
  });

  it("matches task case-insensitively via tool alias and nested state input", () => {
    const tool = readToolCall({
      title: "task", text: "", surface: "activity",
      data: { tool: "task", state: { input: { description: "Research", prompt: "Dig in" } } },
    });
    expect(tool.subagent).toMatchObject({ description: "Research", prompt: "Dig in" });
  });

  it("recognizes a collab-agent item type", () => {
    const tool = readToolCall({
      title: "Explore", text: "done", surface: "activity",
      data: { type: "collabAgentToolCall", description: "Explore repo", prompt: "Summarize" },
    });
    expect(tool.subagent).toMatchObject({ description: "Explore repo", prompt: "Summarize" });
  });

  it("leaves generic tools without a subagent facet", () => {
    for (const data of [
      { name: "Bash", input: { command: "bun test" } },
      { name: "Read", input: { file_path: "src/lib.rs" } },
      { name: "mcp__github__search", input: { query: "x" } },
    ]) {
      expect(readToolCall({ title: "t", text: "", surface: "activity", data }).subagent).toBeUndefined();
    }
  });

  it("does not claim ACP other-kind rows without a task payload", () => {
    expect(readToolCall({ title: "Plan", text: "", surface: "activity", data: { kind: "other" } }).subagent).toBeUndefined();
  });

  it("does not claim dynamic tool calls without both type and prompt fields", () => {
    expect(readToolCall({ title: "d", text: "", surface: "activity", data: { type: "dynamicToolCall", prompt: "only prompt" } }).subagent).toBeUndefined();
  });
});
