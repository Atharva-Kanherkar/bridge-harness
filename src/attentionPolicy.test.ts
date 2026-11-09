import { describe, expect, it } from "vitest";
import { decide, type Presence } from "./attentionPolicy";
import type { AttentionItem } from "./types";

const row = (overrides: Partial<AttentionItem> = {}): AttentionItem => ({
  sequence: 1,
  id: "row-1",
  kind: "finished",
  severity: "informational",
  sessionId: "a",
  approvalEventId: null,
  title: "Turn completed",
  body: null,
  createdAt: "2026-11-09T00:00:00Z",
  resolvedAt: null,
  seenAt: null,
  ...overrides,
});

const presence = (focused: boolean, ...onScreen: string[]): Presence => ({ focused, onScreen: new Set(onScreen) });

describe("decide", () => {
  it.each([
    ["no session", row({ sessionId: null })],
    ["seen", row({ seenAt: "2026-11-09T00:00:01Z" })],
    ["resolved", row({ kind: "needsYou", resolvedAt: "2026-11-09T00:00:01Z" })],
    ["a kind from a newer daemon", row({ kind: "other" })],
    ["a kind this build never heard of", row({ kind: "brandNew" as AttentionItem["kind"] })],
  ])("drops a row with %s", (_case, item) => {
    expect(decide(item, presence(false))).toBeNull();
  });

  it("says nothing about the chat on screen in a focused window", () => {
    for (const kind of ["finished", "failed", "needsYou", "escalation"] as const) {
      expect(decide(row({ kind }), presence(true, "a")), kind).toBeNull();
    }
  });

  it("toasts and banners the chat on screen while Bridge is in the background", () => {
    expect(decide(row(), presence(false, "a"))).toEqual({ toast: true, banner: true, durationMs: 5_000 });
  });

  it("toasts another chat in a focused window without a banner", () => {
    expect(decide(row(), presence(true, "b"))).toEqual({ toast: true, banner: false, durationMs: 5_000 });
  });

  it.each([
    ["needsYou", 8_000],
    ["failed", 8_000],
    ["finished", 5_000],
    ["escalation", null],
  ] as const)("gives %s a toast of %s ms", (kind, durationMs) => {
    expect(decide(row({ kind }), presence(true))?.durationMs).toBe(durationMs);
  });
});
