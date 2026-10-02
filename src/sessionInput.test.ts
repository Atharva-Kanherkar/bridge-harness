import { describe, expect, it } from "vitest";
import type { BridgeEvent } from "./types";
import { activeTurnAction, queuedFollowUps } from "./sessionInput";

let nextId = 100;
const event = (kind: string, entityId: string, body: string): BridgeEvent => ({
  id: nextId--, source: "session", kind, entityId, body, createdAt: "now",
});

describe("queuedFollowUps", () => {
  it("retires a follow-up when its own delivery row arrives", () => {
    // Newest-first, the way the feed actually arrives.
    const events = [
      event("session.input.delivered", "chat", "q-1"),
      event("session.input.queued", "chat", "q-2"),
      event("session.input.queued", "chat", "q-1"),
    ];
    expect(queuedFollowUps("chat", events)).toEqual(["q-2"]);
  });

  it("keeps sessions apart", () => {
    const events = [
      event("session.input.queued", "other", "q-9"),
      event("session.input.queued", "chat", "q-1"),
    ];
    expect(queuedFollowUps("chat", events)).toEqual(["q-1"]);
    expect(queuedFollowUps("other", events)).toEqual(["q-9"]);
  });

  it("retires follow-ups dropped with a cleared session or abandoned by a restart", () => {
    const events = [
      event("session.input.abandoned", "chat", "q-2"),
      event("session.input.discarded", "chat", "q-1"),
      event("session.input.queued", "chat", "q-2"),
      event("session.input.queued", "chat", "q-1"),
    ];
    expect(queuedFollowUps("chat", events)).toEqual([]);
  });

  it("is empty for a session that never queued anything", () => {
    expect(queuedFollowUps("chat", [])).toEqual([]);
    expect(queuedFollowUps("chat", [event("session.started", "chat", "working")])).toEqual([]);
  });
});


describe("activeTurnAction", () => {
  it.each([
    ["steer", ["steering"], "steer"],
    ["steer", [], "queue"],
    ["steer", undefined, "queue"],
    ["queue", ["steering"], "queue"],
    ["queue", [], "queue"],
    ["queue", undefined, "queue"],
  ] as const)("%s with %j resolves to %s", (preference, capabilities, expected) => {
    expect(activeTurnAction(capabilities ? [...capabilities] : undefined, preference)).toBe(expected);
  });
});
