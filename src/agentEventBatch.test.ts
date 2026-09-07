// @vitest-environment jsdom
import { beforeAll, describe, expect, it, vi } from "vitest";
import { asWireKind } from "./transcript/wire";
import type { AgentEvent } from "./types";

/**
 * The seam where the shell's batching stops.
 *
 * `src-tauri/src/agent_batch.rs` coalesces a flush window's worth of agent
 * frames into one IPC message, because a hundred-step turn is four hundred of
 * them and one message each woke the webview four hundred times while it was
 * trying to draw. Everything above `src/api.ts` still sees one frame at a
 * time; this is the assertion that it does.
 */

const listeners = vi.hoisted(() => new Map<string, (event: { payload: unknown }) => void>());

vi.mock("@tauri-apps/api/event", () => ({
  listen: async (name: string, handler: (event: { payload: unknown }) => void) => {
    listeners.set(name, handler);
    return () => listeners.delete(name);
  },
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: async () => ({}) }));

beforeAll(() => {
  (window as typeof window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
});

const frame = (id: number): AgentEvent => ({
  id, sessionId: "s", sequence: id, protocolVersion: 1, kind: asWireKind("message.delta"),
  itemId: null, role: "assistant", status: "streaming", title: null, text: `chunk ${id}`,
  data: {}, providerMeta: {}, createdAt: "now",
});

describe("the batched agent stream", () => {
  it("delivers one frame at a time, in order, from one message", async () => {
    const { bridgeApi } = await import("./api");
    const seen: number[] = [];
    const unlisten = await bridgeApi.onAgentEvent(event => seen.push(event.id));

    const deliver = listeners.get("agent-event-batch");
    expect(deliver, "subscribed to the batched stream").toBeDefined();
    deliver!({ payload: [frame(1), frame(2), frame(3)] });
    deliver!({ payload: [frame(4)] });

    expect(seen).toEqual([1, 2, 3, 4]);
    unlisten();
    expect(listeners.has("agent-event-batch")).toBe(false);
  });
});
