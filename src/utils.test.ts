import { describe, expect, it } from "vitest";
import { canTransition, formatElapsed, safeSlug } from "./utils";

describe("safeSlug", () => {
  it("normalizes unsafe branch text", () => expect(safeSlug(" Add OAuth / callbacks! ")).toBe("add-oauth-callbacks"));
  it("never returns an empty slug", () => expect(safeSlug("!!!")).toBe("task"));
});

describe("session transitions", () => {
  it("accepts a supervised waiting cycle", () => { expect(canTransition("working", "waiting")).toBe(true); expect(canTransition("waiting", "working")).toBe(true); });
  it("rejects impossible idle-to-ready state", () => expect(canTransition("idle", "ready")).toBe(false));
});

describe("formatElapsed", () => {
  it("formats supervised runtime without fake precision", () => expect(formatElapsed("2026-07-12T10:00:00Z", Date.parse("2026-07-12T12:05:00Z"))).toBe("2h 05m"));
  it("labels sessions that have not started", () => expect(formatElapsed(null)).toBe("—"));
});
