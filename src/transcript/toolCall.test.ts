import { describe, expect, it } from "vitest";
import { readToolCall } from "./toolCall";

describe("fallback tool action labels", () => {
  it.each(["other", "think", "switch_mode"])("uses the explicit title for ACP %s", (kind) => {
    const tool = readToolCall({ title: "Resolve project context", text: "", status: "inProgress", surface: "activity", data: { kind } });
    expect(tool).toMatchObject({ doing: "Resolve project context", done: "Resolve project context", status: "running" });
    expect(tool.target).toBeUndefined();
  });

  it.each([undefined, "", "   "])("keeps an honest fallback when no action is named (%s)", (title) => {
    const tool = readToolCall({ title, text: "", surface: "activity", data: {} });
    expect(tool).toMatchObject({ doing: "Using a tool", done: "Used a tool" });
    expect(tool.target).toBeUndefined();
  });

  it("preserves tool names and recognized categories", () => {
    expect(readToolCall({ title: "Context", text: "", surface: "activity", data: { name: "lookup_context" } }))
      .toMatchObject({ doing: "Using lookup_context", done: "Used lookup_context" });
    expect(readToolCall({ title: "project", text: "", surface: "activity", data: { kind: "read" } }))
      .toMatchObject({ doing: "Reading", done: "Read", target: "project" });
  });
});
