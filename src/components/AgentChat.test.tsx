// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import { asWireKind } from "../transcript/wire";
import type { AgentEvent, Session, SessionEntry, SessionForestSnapshot, WorkerRuntimeRecord } from "../types";
import { AgentChat } from "./AgentChat";

// Contract: testing/feat-agents-pane-auto-open.md, agent chat.

const worker = (overrides: Partial<Session> = {}): Session => ({
  id: "w1", workspaceId: "w", harness: "claude", label: "Research · strong", status: "working", startedAt: "2026-09-27T10:00:00Z",
  endedAt: null, contextPercent: null, usagePercent: null, metricSource: "reported", model: "claude-opus-5-5", restorationMode: "native",
  continuationFidelity: "native", kind: "worker", parentSessionId: "chat", ...overrides,
});
const runtime = (overrides: Partial<WorkerRuntimeRecord> = {}): WorkerRuntimeRecord => ({
  sessionId: "w1", parentSessionId: "chat", lifecycleState: "working", taskFamily: "research", compatibilityKey: "k",
  resultStatus: "pending", retryCount: 0, lastResult: null, ...overrides,
} as WorkerRuntimeRecord);
const entry = (id: string, sequence: number, kind: string, text: string, parentEntryId: string | null = null): SessionEntry => ({
  id, sessionId: "w1", parentEntryId, sequence, semanticSchemaVersion: 2, kind, payload: { text, itemId: id },
  providerEventId: null, contextVisibility: "eligible", tokenEstimate: null, createdAt: "2026-09-27T10:00:00Z",
});
const live = (id: number, kind: string, overrides: Partial<AgentEvent> = {}): AgentEvent => ({
  id, sessionId: "w1", sequence: id, protocolVersion: 1, kind: asWireKind(kind), itemId: null, role: null, status: null,
  title: null, text: null, data: {}, providerMeta: {}, createdAt: "2026-09-27T10:00:00Z", ...overrides,
});

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  const entries = [entry("u1", 1, "user.message", "Rank the gateway repos"), entry("a1", 2, "assistant.message", "Checking Bifrost first.", "u1")];
  vi.spyOn(bridgeApi, "sessionForestDigest").mockResolvedValue("digest");
  vi.spyOn(bridgeApi, "sessionForest").mockResolvedValue({ entries, head: { activeEntryId: "a1" } } as unknown as SessionForestSnapshot);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

async function mount(props: Partial<Parameters<typeof AgentChat>[0]> = {}) {
  await act(async () => {
    root.render(<AgentChat session={worker()} runtime={runtime()} liveEvents={[]} onSteer={async () => undefined} {...props} />);
  });
  await act(async () => { await Promise.resolve(); });
}

describe("AgentChat", () => {
  it("draws the worker's history and live frames as a chat, not a raw feed", async () => {
    const other = live(9, "message.delta", { sessionId: "someone-else", itemId: "x", role: "assistant", text: "not this worker" });
    await mount({ liveEvents: [live(3, "message.delta", { itemId: "a2", role: "assistant", text: "Now agentgateway." }), other] });
    const region = container.querySelector('[role="region"][aria-label="Agent Research · strong"]');
    expect(region).not.toBeNull();
    expect(container.querySelector('[role="dialog"]')).toBeNull();
    expect(container.textContent).toContain("Rank the gateway repos");
    expect(container.textContent).toContain("Checking Bifrost first.");
    expect(container.textContent).toContain("Now agentgateway.");
    expect(container.textContent).not.toContain("not this worker");
    // The old feed printed every frame as a mono line in an ordered list.
    expect(container.querySelector("ol.list-none li p.font-mono")).toBeNull();
  });

  it("offers the steer box while the worker is live", async () => {
    await mount();
    expect(container.querySelector('textarea[aria-label="Steer this worker…"]')).not.toBeNull();
  });

  it("shows the finished notice once the worker has reported", async () => {
    await mount({ session: worker({ status: "completed" }), runtime: runtime({ resultStatus: "reported", lifecycleState: "completed" }) });
    expect(container.querySelector('textarea[aria-label="Steer this worker…"]')).toBeNull();
    expect(container.textContent).toContain("This worker has finished");
  });

  it("does not offer steering while the worker is checkpointing", async () => {
    await mount({ runtime: runtime({ lifecycleState: "checkpointing" }) });
    expect(container.querySelector('textarea[aria-label="Steer this worker…"]')).toBeNull();
  });

  it("ends a reported worker's chat on its typed result, even when its last word was only the fence", async () => {
    // A compliant worker may answer with nothing but the fence, which the
    // transcript strips. The outcome must still be on screen.
    const fenceOnly = live(4, "message.completed", { itemId: "a3", role: "assistant", status: "completed", text: "```bridge-worker-result\n{\"schemaVersion\":1,\"status\":\"completed\"}\n```" });
    await mount({
      session: worker({ status: "completed" }),
      liveEvents: [fenceOnly],
      runtime: runtime({
        resultStatus: "reported", lifecycleState: "completed",
        lastResult: { status: "completed", summary: "Bifrost ranks first.", filesChanged: ["notes/ranking.md"], tests: [{ command: "bun test", status: "passed" }, { command: "cargo test", status: "failed" }] },
      }),
    });
    const card = container.querySelector('section[aria-label="Worker result"]');
    expect(card).not.toBeNull();
    expect(card!.textContent).toContain("completed");
    expect(card!.textContent).toContain("Bifrost ranks first.");
    expect(card!.textContent).toContain("1 file changed");
    expect(card!.textContent).toContain("notes/ranking.md");
    expect(card!.textContent).toContain("1 of 2 checks failing");
    expect(container.textContent).not.toContain("bridge-worker-result");
  });

  it("shows no result before the worker reports", async () => {
    await mount({ runtime: runtime({ lastResult: { status: "completed", summary: "the previous run" } }) });
    expect(container.querySelector('section[aria-label="Worker result"]')).toBeNull();
    expect(container.textContent).not.toContain("the previous run");
  });

  it("shows no composer at all when the host does not offer steering", async () => {
    await mount({ onSteer: undefined });
    expect(container.querySelector("textarea")).toBeNull();
    expect(container.textContent).not.toContain("This worker has finished");
  });
});
