import { describe, expect, it } from "vitest";
import { appendAgentEventBatch } from "./agentEvents";
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
});
