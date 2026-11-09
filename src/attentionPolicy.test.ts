import { describe, expect, it } from "vitest";
import { decide, type Presence } from "./attentionPolicy";
import { DEFAULT_ATTENTION_SETTINGS, type AttentionSettings } from "./attentionSettings";
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
    expect(decide(row(), presence(false, "a"))).toEqual({ toast: true, banner: true, sound: false, durationMs: 5_000 });
  });

  it("toasts another chat in a focused window without a banner", () => {
    expect(decide(row(), presence(true, "b"))).toEqual({ toast: true, banner: false, sound: false, durationMs: 5_000 });
  });

  it.each([
    ["needsYou", 8_000],
    ["failed", 8_000],
    ["finished", 5_000],
    ["escalation", null],
  ] as const)("gives %s a toast of %s ms", (kind, durationMs) => {
    expect(decide(row({ kind }), presence(true))?.durationMs).toBe(durationMs);
  });

  describe("with settings", () => {
    const settings = (change: (next: AttentionSettings) => void) => {
      const next = structuredClone(DEFAULT_ATTENTION_SETTINGS);
      change(next);
      return next;
    };
    const noon = new Date(2026, 10, 9, 12, 0);
    const midnight = new Date(2026, 10, 9, 23, 30);

    it("plays a sound for a request by default, not for a finished turn", () => {
      expect(decide(row({ kind: "needsYou" }), presence(false), DEFAULT_ATTENTION_SETTINGS, noon)?.sound).toBe(true);
      expect(decide(row(), presence(false), DEFAULT_ATTENTION_SETTINGS, noon)?.sound).toBe(false);
    });

    it.each(["toast", "banner", "sound"] as const)("turns %s off for its kind only", channel => {
      const off = settings(next => { next.kinds.needsYou[channel] = false; });
      expect(decide(row({ kind: "needsYou" }), presence(false), off, noon)?.[channel]).toBe(false);
      const other = decide(row({ kind: "escalation" }), presence(false), off, noon);
      expect(other?.[channel]).toBe(true);
    });

    it("returns nothing when every channel is off", () => {
      const silent = settings(next => { next.kinds.finished = { toast: false, banner: false, sound: false }; });
      expect(decide(row(), presence(false), silent, noon)).toBeNull();
    });

    it("keeps the toast but drops banner and sound in quiet hours", () => {
      const quiet = settings(next => { next.quietHours = { enabled: true, start: "22:00", end: "08:00" }; });
      expect(decide(row({ kind: "needsYou" }), presence(false), quiet, midnight)).toEqual({ toast: true, banner: false, sound: false, durationMs: 8_000 });
      expect(decide(row({ kind: "needsYou" }), presence(false), quiet, noon)).toEqual({ toast: true, banner: true, sound: true, durationMs: 8_000 });
    });

    it("still says nothing about the chat on screen, whatever the settings", () => {
      expect(decide(row({ kind: "needsYou" }), presence(true, "a"), DEFAULT_ATTENTION_SETTINGS, noon)).toBeNull();
    });
  });

  it("posts no banner from a client that does not own them, and keeps its toast and sound", () => {
    const notOwner = { focused: false, onScreen: new Set<string>(), bannerOwner: false };
    expect(decide(row({ kind: "needsYou" }), notOwner)).toEqual({ toast: true, banner: false, sound: true, durationMs: 8_000 });
    expect(decide(row({ kind: "needsYou" }), { ...notOwner, bannerOwner: true })?.banner).toBe(true);
  });

  it("times a post by its severity and lets its switch turn it off", () => {
    expect(decide(row({ kind: "posted", severity: "actionable" }), presence(true))?.durationMs).toBe(8_000);
    expect(decide(row({ kind: "posted", severity: "informational" }), presence(true))?.durationMs).toBe(5_000);
    const off = structuredClone(DEFAULT_ATTENTION_SETTINGS);
    off.kinds.posted = { toast: false, banner: false, sound: false };
    expect(decide(row({ kind: "posted", severity: "actionable" }), presence(false), off)).toBeNull();
  });
});
