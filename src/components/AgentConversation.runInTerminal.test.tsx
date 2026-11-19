// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { MotionGlobalConfig } from "framer-motion";
import { expect, it, vi } from "vitest";
import { AgentConversation } from "./AgentConversation";
import type { RunInTerminal } from "./RunInTerminal";
import { asWireKind } from "../transcript/wire";
import { MOCK_OUTGOING_ID, resetMockOutgoing } from "../outgoingMock";
import type { AgentEvent } from "../types";

// Contract: testing/feat-run-in-terminal.md §Functional Behavior. Run is a
// command the agent offers the person, so only the agent's own reply has it.
it("offers Run in the agent's reply but not in a thought or a draft for someone else", async () => {
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
    event(1, "message.completed", { itemId: "ask", role: "user", status: "completed", text: "How do I run the tests?" }),
    event(2, "reasoning.completed", { itemId: "thought", status: "completed", text: "They could run:\n\n```bash\nbun run test:thought\n```" }),
    event(3, "message.completed", { itemId: "reply", role: "assistant", status: "completed", text: "Run the suite:\n\n```bash\nbun run test\n```" }),
    event(4, "tool.completed", {
      itemId: "toolu_1", status: "completed", title: "mcp__bridge__outgoing_draft",
      data: { type: "tool_use", name: "mcp__bridge__outgoing_draft", input: {}, aggregatedOutput: JSON.stringify({ id: MOCK_OUTGOING_ID, state: "draft" }) },
    }),
  ];
  const run = vi.fn<RunInTerminal>().mockResolvedValue({ shell: "shell 1" });
  try {
    await act(async () => root.render(<AgentConversation events={events} readOnly onResolve={() => {}} onRunInTerminal={run} />));
    for (let turn = 0; turn < 20 && !host.querySelector("[data-outgoing-preview]"); turn += 1) {
      await act(async () => { await new Promise(resolve => setTimeout(resolve, 10)); });
    }
    host.querySelector<HTMLElement>("[data-thinking] summary")?.click();
    await act(async () => { await Promise.resolve(); });

    const draft = host.querySelector("[data-outgoing-preview]");
    expect(draft?.textContent).toContain("bun run deploy --env staging");
    expect(draft?.querySelector(".code-block")).not.toBeNull();
    expect(draft?.querySelector("button[data-run-state]")).toBeNull();

    const runs = [...host.querySelectorAll<HTMLButtonElement>("button[data-run-state]")];
    expect(runs).toHaveLength(1);
    expect(runs[0].closest(".code-block")?.textContent).toContain("bun run test");
    expect(runs[0].closest(".code-block")?.textContent).not.toContain("test:thought");

    await act(async () => { runs[0].click(); });
    expect(run).toHaveBeenCalledWith("bun run test");
  } finally {
    act(() => root.unmount());
    host.remove();
  }
});
