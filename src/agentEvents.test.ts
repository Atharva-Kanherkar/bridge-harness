import { describe, expect, it } from "vitest";
import {
  appendAgentEventBatch,
  MAX_MERGED_EVENT_TEXT,
  MAX_PENDING_AGENT_EVENTS,
  MAX_TOTAL_EVENT_TEXT,
  queueAgentEvent,
} from "./agentEvents";
import type { AgentEvent } from "./types";

function event(id: number, kind = "message.delta", text = "x"): AgentEvent {
  return {
    id,
    sessionId: "session-1",
    sequence: id,
    protocolVersion: 1,
    kind,
    itemId: "message-1",
    role: "assistant",
    status: "streaming",
    title: null,
    text,
    data: {},
    providerMeta: {},
    createdAt: "now",
  };
}

function itemEvent(itemId: string, kind: string, text: string, id = 0): AgentEvent {
  return { ...event(id, kind, text), sequence: id, itemId };
}

describe("appendAgentEventBatch", () => {
  it("coalesces adjacent streaming deltas and ignores duplicate delivery", () => {
    const result = appendAgentEventBatch([], [event(1, "message.delta", "hel"), event(2, "message.delta", "lo"), event(2, "message.delta", "lo")]);
    expect(result).toHaveLength(1);
    expect(result[0].text).toBe("hello");
    expect(result[0].id).toBe(2);
  });

  it("keeps semantic boundaries and caps retained live events", () => {
    const completed = event(2, "message.completed", "hello");
    const result = appendAgentEventBatch([event(1)], [completed, event(3, "message.delta", "next")], 2);
    expect(result.map(item => item.id)).toEqual([2, 3]);
  });

  it("does not deduplicate unrelated transient id-zero events", () => {
    const result = appendAgentEventBatch([], [
      itemEvent("message-1", "message.delta", "hello"),
      itemEvent("tool-1", "tool.progress", "searching"),
      itemEvent("message-2", "reasoning.delta", "thinking"),
    ]);
    expect(result.map(item => item.itemId)).toEqual(["message-1", "tool-1", "message-2"]);
  });

  it("scopes durable ids to their session", () => {
    const first = event(7, "message.completed", "one");
    const second = { ...event(7, "message.completed", "two"), sessionId: "session-2" };
    const result = appendAgentEventBatch([], [first, second, first]);
    expect(result.map(item => `${item.sessionId}:${item.text}`)).toEqual([
      "session-1:one",
      "session-2:two",
    ]);
  });

  it("replaces repeated progress snapshots while additive deltas append", () => {
    const result = appendAgentEventBatch([], [
      itemEvent("tool-1", "tool.progress", "Searched 1 file"),
      itemEvent("message-1", "message.delta", "hel"),
      itemEvent("tool-1", "tool.progress", "Searched 20 files"),
      itemEvent("message-1", "message.delta", "lo"),
    ]);
    expect(result).toHaveLength(2);
    expect(result.find(item => item.itemId === "tool-1")?.text).toBe("Searched 20 files");
    expect(result.find(item => item.itemId === "message-1")?.text).toBe("hello");
  });

  it("bounds and coalesces the pre-render queue during a large tool burst", () => {
    let queue: AgentEvent[] = [];
    for (let index = 0; index < 10_000; index += 1) {
      queue = queueAgentEvent(queue, itemEvent(`tool-${index % 16}`, "tool.progress", `Scanned ${index} files`));
    }
    expect(queue).toHaveLength(16);
    expect(queue.length).toBeLessThanOrEqual(MAX_PENDING_AGENT_EVENTS);
    expect(queue.at(-1)?.text).toBe("Scanned 9999 files");
  });

  it("evicts the stalest snapshot instead of freshly merged progress", () => {
    let queue = Array.from({ length: MAX_PENDING_AGENT_EVENTS }, (_, index) =>
      itemEvent(`tool-${index}`, "tool.progress", `Initial ${index}`),
    );
    queue = queueAgentEvent(queue, itemEvent("tool-0", "tool.progress", "Still active"));
    queue = queueAgentEvent(queue, itemEvent("tool-new", "tool.progress", "New tool"));

    expect(queue).toHaveLength(MAX_PENDING_AGENT_EVENTS);
    expect(queue.find(item => item.itemId === "tool-0")?.text).toBe("Still active");
    expect(queue.some(item => item.itemId === "tool-1")).toBe(false);
    expect(queue.at(-1)?.itemId).toBe("tool-new");
  });

  it("caps a merged item's text at the byte budget, keeping the tail", () => {
    const half = "a".repeat(MAX_MERGED_EVENT_TEXT - 1);
    const result = appendAgentEventBatch(
      [event(1, "command.output_delta", half)],
      [event(2, "command.output_delta", "TAIL")],
    );
    expect(result).toHaveLength(1);
    expect(result[0].text).toHaveLength(MAX_MERGED_EVENT_TEXT);
    expect(result[0].text?.endsWith("TAIL")).toBe(true);
  });

  it("evicts the oldest events once total text exceeds the budget", () => {
    const big = (id: number) =>
      event(id, "message.completed", "b".repeat(MAX_MERGED_EVENT_TEXT));
    const events = Array.from({ length: 12 }, (_, index) => big(index + 1));
    const result = appendAgentEventBatch([], events);
    const total = result.reduce((sum, item) => sum + (item.text?.length ?? 0), 0);
    expect(total).toBeLessThanOrEqual(MAX_TOTAL_EVENT_TEXT);
    expect(result[result.length - 1].id).toBe(12);
    expect(result[0].id).toBeGreaterThan(1);
  });
});
