import { describe, expect, it } from "vitest";
import { diffAttentionEvents } from "./attentionEvents";
import type { Session, SessionStatus } from "./types";

function session(id: string, status: SessionStatus, overrides: Partial<Session> = {}): Session {
  return {
    continuationFidelity: "native",
    harness: "claude",
    id,
    kind: "chat",
    label: `Chat ${id}`,
    metricSource: "provider",
    restorationMode: "fresh",
    status,
    ...overrides,
  };
}

describe("diffAttentionEvents", () => {
  it("fires nothing on the first snapshot", () => {
    expect(diffAttentionEvents(undefined, [session("a", "waiting")])).toEqual([]);
  });

  it("fires nothing when no status changed", () => {
    const sessions = [session("a", "working")];
    expect(diffAttentionEvents(sessions, sessions)).toEqual([]);
  });

  it("reports needs-you when a session starts waiting", () => {
    const events = diffAttentionEvents([session("a", "working")], [session("a", "waiting")]);
    expect(events).toEqual([{ kind: "needs-you", session: session("a", "waiting") }]);
  });

  it("reports turn-completed when an active session finishes", () => {
    const events = diffAttentionEvents([session("a", "working")], [session("a", "ready")]);
    expect(events).toEqual([{ kind: "turn-completed", session: session("a", "ready") }]);
  });

  it("reports turn-completed when an active turn fails", () => {
    const events = diffAttentionEvents([session("a", "working")], [session("a", "failed")]);
    expect(events).toEqual([{ kind: "turn-completed", session: session("a", "failed") }]);
  });

  it("reports nothing when the human replies and the session resumes", () => {
    expect(diffAttentionEvents([session("a", "waiting")], [session("a", "working")])).toEqual([]);
  });

  it("reports nothing for a bucket-internal status change", () => {
    expect(diffAttentionEvents([session("a", "starting")], [session("a", "working")])).toEqual([]);
  });

  it("reports nothing for a session created since the last snapshot", () => {
    expect(diffAttentionEvents([], [session("a", "waiting")])).toEqual([]);
  });

  it("reports one event per changed session, in next order", () => {
    const previous = [session("a", "working"), session("b", "working")];
    const next = [session("a", "waiting"), session("b", "ready")];
    expect(diffAttentionEvents(previous, next)).toEqual([
      { kind: "needs-you", session: session("a", "waiting") },
      { kind: "turn-completed", session: session("b", "ready") },
    ]);
  });
});
