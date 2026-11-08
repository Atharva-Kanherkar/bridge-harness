import { describe, expect, it } from "vitest";
import { attentionCopy, attentionToastKey } from "./attentionCopy";
import type { AttentionItem, Session } from "./types";

function session(overrides: Partial<Session> = {}): Session {
  return {
    continuationFidelity: "native",
    harness: "claude",
    id: "s1",
    kind: "chat",
    label: "Label chat",
    metricSource: "provider",
    restorationMode: "fresh",
    status: "ready",
    title: "Auth tokens",
    ...overrides,
  };
}

function row(kind: AttentionItem["kind"]): Pick<AttentionItem, "kind" | "title" | "body"> {
  return { kind, title: "Core headline", body: "New chat · Claude core body" };
}

describe("attentionCopy", () => {
  it("describes a request with the chat's live name and harness", () => {
    expect(attentionCopy(row("needsYou"), session())).toEqual({
      headline: "Bridge needs you",
      detail: "Auth tokens · Claude is waiting for your input",
      tone: "needs-you",
    });
  });

  it("describes a finished turn", () => {
    expect(attentionCopy(row("finished"), session())).toEqual({
      headline: "Turn completed",
      detail: "Auth tokens · Claude finished its turn",
      tone: "completed",
    });
  });

  it("describes a failed turn", () => {
    expect(attentionCopy(row("failed"), session())).toEqual({
      headline: "Turn failed",
      detail: "Auth tokens · Claude ended this turn with an error",
      tone: "failed",
    });
  });

  it("falls back to the label and humanizes the harness", () => {
    expect(attentionCopy(row("needsYou"), session({ title: "", harness: "codex" })).detail).toBe("Label chat · Codex is waiting for your input");
  });

  it("uses the row's own copy when the chat is not known here", () => {
    expect(attentionCopy(row("failed"))).toEqual({ headline: "Core headline", detail: "New chat · Claude core body", tone: "failed" });
    expect(attentionCopy({ kind: "finished", title: "Turn completed", body: null }).detail).toBe("");
  });
});

describe("escalation copy", () => {
  it("keeps core's count even when the chat is known", () => {
    const item = { kind: "escalation" as const, title: "A result is waiting", body: "Ship it · Codex stopped before it read 2 updates" };
    expect(attentionCopy(item, session())).toEqual({ headline: "A result is waiting", detail: "Ship it · Codex stopped before it read 2 updates", tone: "needs-you" });
    expect(attentionCopy(item)).toEqual(attentionCopy(item, session()));
  });
});

describe("attentionToastKey", () => {
  it("keys by ledger sequence", () => {
    expect(attentionToastKey({ sequence: 42 })).toBe("attention:42");
  });
});
