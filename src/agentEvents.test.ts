import { describe, expect, it } from "vitest";
import {
  appendAgentEventBatch,
  MAX_MERGED_EVENT_TEXT,
  MAX_TOTAL_EVENT_TEXT,
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
