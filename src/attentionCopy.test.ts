import { describe, expect, it } from "vitest";
import { attentionCopy, attentionToastKey } from "./attentionCopy";
import type { AttentionEvent } from "./attentionEvents";
import type { Session, SessionStatus } from "./types";

function session(status: SessionStatus, overrides: Partial<Session> = {}): Session {
  return {
    continuationFidelity: "native",
    harness: "claude",
    id: "s1",
    kind: "chat",
    label: "Label chat",
    metricSource: "provider",
    restorationMode: "fresh",
    status,
    title: "Auth tokens",
    ...overrides,
  };
}

function event(kind: AttentionEvent["kind"], status: SessionStatus, overrides: Partial<Session> = {}): AttentionEvent {
  return { kind, session: session(status, overrides) };
}

describe("attentionCopy", () => {
  it("describes a needs-you event with chat name and harness", () => {
    expect(attentionCopy(event("needs-you", "waiting"))).toEqual({
      headline: "Bridge needs you",
      detail: "Auth tokens · Claude is waiting for your input",
      tone: "needs-you",
    });
  });

  it("describes a completed turn", () => {
    expect(attentionCopy(event("turn-completed", "ready"))).toEqual({
      headline: "Turn completed",
      detail: "Auth tokens · Claude finished its turn",
      tone: "completed",
    });
  });

  it("uses Turn failed when the session status is failed", () => {
    expect(attentionCopy(event("turn-completed", "failed"))).toEqual({
      headline: "Turn failed",
      detail: "Auth tokens · Claude ended this turn with an error",
      tone: "failed",
    });
  });

  it("falls back to label when title is empty and humanizes harness", () => {
    const copy = attentionCopy(event("needs-you", "waiting", { title: "", harness: "codex" }));
    expect(copy.detail).toBe("Label chat · Codex is waiting for your input");
  });
});

describe("attentionToastKey", () => {
  it("keys by session, kind, and status", () => {
    expect(attentionToastKey(event("needs-you", "waiting"))).toBe("s1:needs-you:waiting");
  });
});
