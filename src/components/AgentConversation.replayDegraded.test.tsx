// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { MotionGlobalConfig } from "framer-motion";
import { expect, it } from "vitest";
import { AgentConversation } from "./AgentConversation";
import { asWireKind } from "../transcript/wire";
import type { AgentEvent } from "../types";

it("renders an unavailable replay entry between the surviving messages", () => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  MotionGlobalConfig.skipAnimations = true;
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  const event = (sequence: number, kind: string, overrides: Partial<AgentEvent>): AgentEvent => ({
    id: sequence, sessionId: "s", sequence, protocolVersion: 1, kind: asWireKind(kind),
    itemId: null, role: null, status: null, title: null, text: null, data: {},
    providerMeta: {}, createdAt: "now", ...overrides,
  });
  const events = [
    event(1, "message.completed", { itemId: "before", role: "user", status: "completed", text: "Before damaged entry" }),
    event(2, "entry.invalid", {
      status: "degraded", title: "Unavailable history entry",
      text: "This history entry could not be read: payload is not an object",
      data: { entryId: "damaged", originalKind: "assistant.message", sequence: 2, reason: "payload is not an object" },
    }),
    event(3, "message.completed", { itemId: "after", role: "assistant", status: "completed", text: "After damaged entry" }),
  ];
  try {
    act(() => root.render(<AgentConversation events={events} readOnly onResolve={() => {}} />));
    const rows = [...host.querySelector("[data-conversation-content]")!.children];
    expect(rows).toHaveLength(3);
    expect(rows[0].textContent).toContain("Before damaged entry");
    expect(rows[1].querySelector('[role="alert"]')?.textContent).toContain("Unavailable history entry");
    expect(rows[1].textContent).toContain("payload is not an object");
    expect(rows[1].getAttribute("data-entry-id")).toBe("damaged");
    expect(rows[2].textContent).toContain("After damaged entry");
    expect(host.textContent).not.toContain("Using a tool");
  } finally {
    act(() => root.unmount());
    host.remove();
    MotionGlobalConfig.skipAnimations = false;
  }
});
