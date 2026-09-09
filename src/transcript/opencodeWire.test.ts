import { expect, it } from "vitest";
import fixture from "./fixtures/opencode-wire.json";
import { reduceConversation } from "../conversation";
import { asWireKind, readWireKind } from "./wire";
import type { AgentEvent } from "../types";
it("captured OpenCode thinking settles before answer text and reloads as one thought", () => {
  const events = fixture.expected.map((event, i) => ({ ...event, kind: asWireKind(event.kind), protocolVersion: 1, title: null, providerMeta: {}, id: i + 1, sequence: i + 1, sessionId: "fixture", data: {}, createdAt: "2026-09-10T00:00:00Z" })) as AgentEvent[];
  const firstAnswer = events.findIndex(event => readWireKind(event.kind) === "message.delta");
  const atAnswer = reduceConversation(events.slice(0, firstAnswer + 1));
  expect(atAnswer.filter(item => item.type === "reasoning").map(item => [item.text, item.status])).toEqual([["Checking the facts.", "completed"]]);
  const full = reduceConversation(events);
  expect(full.filter(item => item.type === "message").map(item => item.text)).toEqual(["Here is the answer."]);
  const durable = reduceConversation(events.filter(event => !readWireKind(event.kind).endsWith(".delta") && readWireKind(event.kind) !== "reasoning.started"));
  expect(durable.filter(item => item.type === "reasoning").map(item => item.text)).toEqual(["Checking the facts."]);
  expect(events.filter(event => readWireKind(event.kind) === "turn.started")).toHaveLength(1);
});
