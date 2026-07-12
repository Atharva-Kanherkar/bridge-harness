import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { AgentConversation } from "./AgentConversation";
import type { AgentEvent, Session } from "../types";

const session: Session = { id: "s", workspaceId: "w", harness: "codex", label: "Orchestrator", status: "working", startedAt: "now", endedAt: null, contextPercent: null, usagePercent: null, metricSource: "reported", model: "gpt-5.6-luna" };
const event = (id: number, kind: string, overrides: Partial<AgentEvent> = {}): AgentEvent => ({ id, sessionId: "s", sequence: id, protocolVersion: 1, kind, itemId: null, role: null, status: null, title: null, text: null, data: {}, providerMeta: {}, createdAt: "now", ...overrides });

describe("AgentConversation", () => {
  it("renders normalized primitives as GUI cards without a terminal surface", () => {
    const html = renderToStaticMarkup(<AgentConversation session={session} onResolve={() => undefined} events={[
      event(1, "message.completed", { itemId: "m", role: "assistant", text: "Structured response", status: "completed" }),
      event(2, "plan.updated", { title: "Plan", data: { plan: [{ step: "Render GUI", status: "completed" }] } }),
      event(3, "approval.requested", { title: "Approve command", status: "pending", data: { command: "bun test" } })
    ]}/>);
    expect(html).toContain("STRUCTURED ADAPTER");
    expect(html).toContain("Structured response");
    expect(html).toContain("Approve command");
    expect(html).not.toContain("terminal-host");
    expect(html).not.toContain("xterm");
  });
});
