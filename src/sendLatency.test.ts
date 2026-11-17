import { afterEach, describe, expect, it } from "vitest";
import {
  abandonFlight, beginNewChat, beginSend, bindNewChat, markFlight, markSubmitted,
  observeSendLatency, setLatencyFrameScheduler, setLatencyReporter, type LatencyStep,
} from "./sendLatency";
import type { AgentEvent } from "./types";

const event = (sessionId: string, kind: string, role: string | null = "assistant") =>
  ({ sessionId, kind, role, data: {}, providerMeta: {} }) as unknown as AgentEvent;

function capture() {
  const reports: { sessionId: string; flow: string; steps: LatencyStep[] }[] = [];
  setLatencyReporter((sessionId, flow, steps) => reports.push({ sessionId, flow, steps }));
  setLatencyFrameScheduler(callback => callback());
  return reports;
}

afterEach(() => setLatencyReporter(undefined));

describe("send latency", () => {
  it("times a new chat from the click through its first send to the first output", () => {
    const reports = capture();
    const flight = beginNewChat(1000);
    markFlight(flight, "resolve_chat_settings", 1010);
    markFlight(flight, "create_configured_chat", 1400);
    bindNewChat(flight, "chat-1");

    const send = beginSend("chat-1", 1500);
    expect(send).toBe(flight);
    markFlight(send, "prepare_turn", 1520);
    markFlight(send, "start_chat", 3000);
    markSubmitted(send, 3010);
    observeSendLatency(event("chat-1", "message.completed", "user"), 3020);
    observeSendLatency(event("chat-1", "turn.started"), 5000);
    observeSendLatency(event("chat-1", "message.delta"), 6000);

    expect(reports).toHaveLength(1);
    expect(reports[0].flow).toBe("new_chat");
    expect(reports[0].steps.map(step => [step.name, step.atMs])).toEqual([
      ["resolve_chat_settings", 10], ["create_configured_chat", 400], ["prepare_turn", 520],
      ["start_chat", 2000], ["submit_sent", 2010], ["first_frame", 2020],
      ["first_output_received", 5000], ["first_output_painted", expect.any(Number)],
    ]);
  });

  it("ignores frames before submit, the user's own echo, and other chats", () => {
    const reports = capture();
    const send = beginSend("chat-2", 0);
    observeSendLatency(event("chat-2", "message.delta"), 5);
    markSubmitted(send, 10);
    observeSendLatency(event("other", "message.delta"), 20);
    observeSendLatency(event("chat-2", "message.completed", "user"), 30);
    expect(reports).toHaveLength(0);
    observeSendLatency(event("chat-2", "tool.started"), 40);
    expect(reports[0].steps.find(step => step.name === "first_output_received")?.atMs).toBe(40);
  });

  it("reports nothing for an abandoned send, and a later send starts its own flight", () => {
    const reports = capture();
    const first = beginSend("chat-3", 0);
    abandonFlight(first);
    observeSendLatency(event("chat-3", "message.delta"), 10);
    expect(reports).toHaveLength(0);
    const second = beginSend("chat-3", 100);
    expect(second).not.toBe(first);
    expect(second.flow).toBe("send");
  });

  it("drops a send that waited past the abandon window", () => {
    const reports = capture();
    const send = beginSend("chat-4", 0);
    markSubmitted(send, 1);
    observeSendLatency(event("chat-4", "message.delta"), 11 * 60_000);
    expect(reports).toHaveLength(0);
  });
});
