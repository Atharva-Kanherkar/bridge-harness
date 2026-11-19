import { describe, expect, it } from "vitest";
import type { ConversationItem } from "./item";
import { groupItems } from "./grouping";
import { isPersonalAppItem, personalAppReference } from "./personalApp";
import { normalizeAgentEvent } from "./codec";
import { reduceTranscript } from "./reducer";
import { asWireKind } from "./wire";
import type { AgentEvent } from "../types";
const hash = `sha256:${"b".repeat(64)}`;
const answer = { draftId: "draft", contentHash: hash };
const reference = { kind: "draft", ...answer };
const row = (data: Record<string, unknown>, status = "completed"): ConversationItem => ({ key: JSON.stringify(data), type: "activity", eventId: 1, sequence: 1, turn: 1, text: "", status, data });

describe("personal app draft transcript", () => {
  it("recognizes exact tool identities and extracts only result references", () => {
    for (const data of [
      { name: "mcp__bridge__apps_draft", aggregatedOutput: JSON.stringify(answer) },
      { type: "mcpToolCall", server: "bridge", tool: "apps_draft", result: { structuredContent: answer } },
      { tool: "bridge_apps_draft", state: { output: JSON.stringify(answer) } },
    ]) { expect(personalAppReference(row(data))).toEqual(reference); }
    const acp = (title: string, update: Record<string, unknown>): ConversationItem => ({ ...row({ update }), title });
    for (const update of [
      { sessionUpdate: "tool_call_update", status: "completed", rawOutput: answer },
      { sessionUpdate: "tool_call_update", status: "completed", content: [{ type: "content", content: { type: "text", text: JSON.stringify(answer) } }] },
      { sessionUpdate: "tool_call_update", rawOutput: { content: [{ type: "text", text: JSON.stringify(answer) }] } },
    ]) { expect(personalAppReference(acp("bridge: apps_draft", update))).toEqual(reference); }
    expect(isPersonalAppItem(acp("bridge: apps_draft", {}))).toBe(true);
    expect(isPersonalAppItem(acp("railway: apps_draft", {}))).toBe(false);
    expect(isPersonalAppItem(acp("Run apps_draft", {}))).toBe(false);
    expect(personalAppReference(acp("bridge: apps_draft", { rawInput: answer }))).toBeUndefined();
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
  it("reads a Cursor draft through the real start-to-completion reducer flow", () => {
    const live = (id: number, kind: string, overrides: Partial<AgentEvent>): AgentEvent => ({
      id, sessionId: "s", sequence: id, protocolVersion: 1, kind: asWireKind(kind),
      itemId: "call-1", role: null, status: null, title: null, text: null, data: {}, providerMeta: {}, createdAt: "now", ...overrides,
    });
    const start = live(1, "tool.started", { title: "bridge: apps_draft", status: "inProgress", data: { kind: "other", update: {
      sessionUpdate: "tool_call", toolCallId: "call-1", title: "bridge: apps_draft", kind: "other", status: "in_progress",
      rawInput: { draftId: "spoofed", contentHash: `sha256:${"c".repeat(64)}` },
    } } });
    for (const completion of [
      { sessionUpdate: "tool_call_update", toolCallId: "call-1", status: "completed", rawOutput: answer },
      { sessionUpdate: "tool_call_update", toolCallId: "call-1", status: "completed", content: [{ type: "content", content: { type: "text", text: JSON.stringify(answer) } }] },
      { sessionUpdate: "tool_call_update", toolCallId: "call-1", status: "completed", rawOutput: { content: [{ type: "text", text: JSON.stringify(answer) }] } },
    ]) {
      const done = live(2, "tool.completed", { status: "completed", data: completion });
      const [item] = reduceTranscript([start, done].map(normalizeAgentEvent));
      expect(isPersonalAppItem(item)).toBe(true);
      expect(personalAppReference(item)).toEqual(reference);
    }
    const [unfinished] = reduceTranscript([start].map(normalizeAgentEvent));
    expect(personalAppReference(unfinished)).toBeUndefined();
  });
  it("recognizes apps_open in every agent's shape and reads only the open's identity", () => {
    const opened = { openId: "open-1", contentHash: hash, appId: "slack-dm" };
    const expected = { kind: "open", openId: "open-1", contentHash: hash };
    for (const data of [
      { name: "mcp__bridge__apps_open", aggregatedOutput: JSON.stringify(opened) },
      { type: "mcpToolCall", server: "bridge", tool: "apps_open", result: { structuredContent: opened } },
      { tool: "bridge_apps_open", state: { output: JSON.stringify(opened) } },
    ]) { expect(personalAppReference(row(data))).toEqual(expected); }
    const acp: ConversationItem = { ...row({ update: { sessionUpdate: "tool_call" }, rawOutput: opened }), title: "bridge: apps_open" };
    expect(personalAppReference(acp)).toEqual(expected);
    expect(personalAppReference(row({ name: "mcp__bridge__apps_open", aggregatedOutput: JSON.stringify(answer) }))).toBeUndefined();
    expect(personalAppReference(row({ name: "mcp__bridge__apps_draft", aggregatedOutput: JSON.stringify(opened) }))).toBeUndefined();
    expect(personalAppReference(row({ name: "mcp__bridge__apps_open", input: opened }))).toBeUndefined();
    expect(isPersonalAppItem(row({ name: "mcp__other__apps_open" }))).toBe(false);
  });
});
