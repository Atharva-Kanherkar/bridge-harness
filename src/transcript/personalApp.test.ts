import { describe, expect, it } from "vitest";
import type { ConversationItem } from "./item";
import { groupItems } from "./grouping";
import { isPersonalAppItem, personalAppReference } from "./personalApp";
const hash = `sha256:${"b".repeat(64)}`;
const answer = { draftId: "draft", contentHash: hash };
const row = (data: Record<string, unknown>, status = "completed"): ConversationItem => ({ key: JSON.stringify(data), type: "activity", eventId: 1, sequence: 1, turn: 1, text: "", status, data });

describe("personal app draft transcript", () => {
  it("recognizes exact tool identities and extracts only result references", () => {
    for (const data of [
      { name: "mcp__bridge__apps_draft", aggregatedOutput: JSON.stringify(answer) },
      { type: "mcpToolCall", server: "bridge", tool: "apps_draft", result: { structuredContent: answer } },
      { tool: "bridge_apps_draft", state: { output: JSON.stringify(answer) } },
    ]) { expect(personalAppReference(row(data))).toEqual(answer); }
    expect(isPersonalAppItem(row({ name: "mcp__other__apps_draft" }))).toBe(false);
    expect(isPersonalAppItem(row({ type: "mcpToolCall", server: "other", tool: "apps_draft" }))).toBe(false);
    expect(personalAppReference(row({ name: "mcp__bridge__apps_draft", input: { ...answer, view_html: "evil" } }))).toBeUndefined();
    expect(personalAppReference(row({ name: "mcp__bridge__apps_draft", aggregatedOutput: JSON.stringify({ ...answer, contentHash: "wrong" }) }))).toBeUndefined();
    expect(personalAppReference(row({ name: "mcp__bridge__apps_draft", result: { structuredContent: answer, isError: true } }))).toBeUndefined();
  });
  it("stands alone while drafting and after completion", () => {
    const before = row({ type: "commandExecution", command: "ls" });
    const after = row({ type: "commandExecution", command: "pwd" });
    for (const status of ["inProgress", "completed", "failed"]) {
      const draft = row({ name: "mcp__bridge__apps_draft", aggregatedOutput: JSON.stringify(answer) }, status);
      expect(groupItems([before, draft, after]).map(entry => entry.kind)).toEqual(["group", "item", "group"]);
    }
  });
});
