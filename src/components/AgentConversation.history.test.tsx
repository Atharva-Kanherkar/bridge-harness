// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { AgentConversation } from "./AgentConversation";
import type { Session, SessionEntry } from "../types";
import type { SessionEntryWindowSummary } from "../protocol/generated/protocol";

const session: Session = { id: "s", workspaceId: "w", harness: "codex", label: "Chat", status: "idle", startedAt: "now", endedAt: null, contextPercent: null, usagePercent: null, metricSource: "reported", model: "gpt-5.6-luna", restorationMode: "fresh", continuationFidelity: "native", kind: "direct" };

const entry: SessionEntry = { id: "e1", sessionId: "s", parentEntryId: null, sequence: 1, semanticSchemaVersion: 2, kind: "assistant.message", payload: { text: "history" }, providerEventId: null, contextVisibility: "eligible", tokenEstimate: null, createdAt: "now" };

const window_ = (returned: number, total: number): SessionEntryWindowSummary => ({ returned, total, trimmedPayloads: 0 });

async function render(node: React.ReactElement) {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const container = document.createElement("div");
  const root = createRoot(container);
  await act(async () => root.render(node));
  return { container, cleanup: () => act(async () => root.unmount()) };
}

describe("AgentConversation history loading", () => {
  it("shows a skeleton while an empty transcript's history is in flight", async () => {
    const { container, cleanup } = await render(
      <AgentConversation session={session} onResolve={() => undefined} />,
    );
    try {
      expect(container.querySelector("[aria-label=\"Loading conversation\"]")).not.toBeNull();
    } finally {
      await cleanup();
    }
  });

  it("drops the skeleton once history arrives", async () => {
    const { container, cleanup } = await render(
      <AgentConversation session={session} onResolve={() => undefined}
        forestEntries={[entry]} activeLeafId="e1" entryWindow={window_(1, 1)} />,
    );
    try {
      // A poll that refreshes an already-rendered transcript must not flicker
      // it back to a placeholder.
      expect(container.querySelector("[aria-label=\"Loading conversation\"]")).toBeNull();
      expect(container.textContent).toContain("history");
    } finally {
      await cleanup();
    }
  });

  it("says how much of a long chat is rendered rather than passing a tail off as the whole thing", async () => {
    const { container, cleanup } = await render(
      <AgentConversation session={session} onResolve={() => undefined}
        forestEntries={[entry]} activeLeafId="e1" entryWindow={window_(1500, 7344)} />,
    );
    try {
      expect(container.textContent).toContain("most recent 1,500 of 7,344 events");
      expect(container.textContent).toContain("5,844 earlier events are");
    } finally {
      await cleanup();
    }
  });

  it("stays silent when the whole conversation fits in the window", async () => {
    const { container, cleanup } = await render(
      <AgentConversation session={session} onResolve={() => undefined}
        forestEntries={[entry]} activeLeafId="e1" entryWindow={window_(1, 1)} />,
    );
    try {
      expect(container.textContent).not.toContain("most recent");
    } finally {
      await cleanup();
    }
  });
});
