// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { MotionGlobalConfig } from "framer-motion";
import { expect, it } from "vitest";
import { AgentConversation } from "./AgentConversation";
import { asWireKind } from "../transcript/wire";
import { MOCK_OUTGOING_ID, resetMockOutgoing } from "../outgoingMock";
import type { AgentEvent } from "../types";

// An archived chat renders from its events alone, with no `session` prop:
// the draft card must still find its chat and load the stored draft.
it("loads a drafted message in an archived transcript that has no session prop", async () => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  MotionGlobalConfig.skipAnimations = true;
  resetMockOutgoing();
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  const event = (sequence: number, kind: string, overrides: Partial<AgentEvent>): AgentEvent => ({
    id: sequence, sessionId: "session-1", sequence, protocolVersion: 1, kind: asWireKind(kind),
    itemId: null, role: null, status: null, title: null, text: null, data: {},
    providerMeta: {}, createdAt: "now", ...overrides,
  });
  const events = [
    event(1, "message.completed", { itemId: "ask", role: "user", status: "completed", text: "Tell #dev the deploy is done" }),
    event(2, "tool.completed", {
      itemId: "toolu_1", status: "completed", title: "mcp__bridge__outgoing_draft",
      data: { type: "tool_use", name: "mcp__bridge__outgoing_draft", input: {}, aggregatedOutput: JSON.stringify({ id: MOCK_OUTGOING_ID, state: "draft" }) },
    }),
  ];
  try {
    await act(async () => root.render(<AgentConversation events={events} readOnly onResolve={() => {}} />));
    await act(async () => { await Promise.resolve(); await Promise.resolve(); });
    expect(host.querySelector("[data-outgoing-state=drafting]")).toBeNull();
    expect(host.querySelector("[data-outgoing-preview]")?.textContent).toContain("Deploy to staging is done");
    expect([...host.querySelectorAll("button")].some(button => button.textContent?.trim() === "Edit")).toBe(false);
  } finally {
    act(() => root.unmount());
    host.remove();
  }
});
