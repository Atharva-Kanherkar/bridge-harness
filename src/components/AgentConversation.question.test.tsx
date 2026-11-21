// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { AgentConversation } from "./AgentConversation";
import { asWireKind } from "../transcript/wire";
import type { AgentEvent, Session, SessionEntry } from "../types";

const session: Session = { id: "s", workspaceId: "w", harness: "claude", label: "Chat", status: "waiting", startedAt: "now", endedAt: null, contextPercent: null, usagePercent: null, metricSource: "reported", model: "claude-opus-5-5", restorationMode: "fresh", continuationFidelity: "native", kind: "direct" };

// The shapes Claude's AskUserQuestion is stored with: the permission callback
// becomes the question, and the assistant's tool_use is reported as a tool call
// under the same id.
const question = { tool_use_id: "toolu_q", tool_name: "AskUserQuestion", requestId: "root-permission-1", requestMethod: "claude.askUserQuestion", interactionKind: "question", questions: [{ id: "0", header: "Junk chats", question: "What should Bridge do with them?", multiSelect: false, allowOther: true, options: [{ label: "Remove them", description: "" }, { label: "Leave them", description: "" }] }] };
const toolUse = { type: "tool_use", id: "toolu_q", name: "AskUserQuestion", phase: "running" };

const live = (id: number, kind: string, overrides: Partial<AgentEvent>): AgentEvent => ({
  id, sessionId: "s", sequence: id, protocolVersion: 1, kind: asWireKind(kind), itemId: null, role: null,
  status: null, title: null, text: null, data: {}, providerMeta: { adapter: "claude" }, createdAt: "now", ...overrides,
});

const stored = (id: string, parentEntryId: string | null, kind: string, sequence: number, payload: Record<string, unknown>): SessionEntry => ({
  id, sessionId: "s", parentEntryId, sequence, semanticSchemaVersion: 2, kind, payload,
  providerEventId: null, contextVisibility: "eligible", tokenEstimate: null, createdAt: "now",
});

const events: AgentEvent[] = [
  live(1, "user.message", { role: "user", status: "completed", text: "Clean up the imports" }),
  live(2, "question.requested", { itemId: "toolu_q", title: "Junk chats", status: "pending", text: "What should Bridge do with them?", data: question }),
  live(3, "tool.started", { itemId: "toolu_q", title: "AskUserQuestion", status: "inProgress", data: toolUse }),
];

const forest: SessionEntry[] = [
  stored("e1", null, "user.message", 1, { role: "user", status: "completed", text: "Clean up the imports" }),
  stored("e2", "e1", "question.requested", 2, { itemId: "toolu_q", title: "Junk chats", status: "pending", text: "What should Bridge do with them?", data: question }),
  stored("e3", "e2", "tool.started", 3, { itemId: "toolu_q", title: "AskUserQuestion", status: "inProgress", data: toolUse }),
];

async function mount(forestEntries: SessionEntry[] | undefined) {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const container = document.createElement("div");
  const root = createRoot(container);
  await act(async () => root.render(
    <AgentConversation session={session} events={events} forestEntries={forestEntries} activeLeafId={forestEntries ? "e3" : undefined} onResolve={async () => {}} onAnswerQuestion={async () => {}} />,
  ));
  return { container, cleanup: () => act(async () => root.unmount()) };
}

describe("AgentConversation question card", () => {
  it.each([
    ["live only", undefined],
    ["once the forest has caught up", forest],
  ] as const)("draws Claude's question once, with no tool row for it, %s", async (_label, forestEntries) => {
    const { container, cleanup } = await mount(forestEntries);
    expect(container.querySelectorAll('section[aria-label="Question from the agent"]')).toHaveLength(1);
    expect(container.textContent).not.toContain("AskUserQuestion");
    await cleanup();
  });
});
