import { describe, expect, it } from "vitest";
import { canTransition, safeSlug } from "./utils";

describe("safeSlug", () => {
  it("normalizes unsafe branch text", () => expect(safeSlug(" Add OAuth / callbacks! ")).toBe("add-oauth-callbacks"));
  it("never returns an empty slug", () => expect(safeSlug("!!!")).toBe("task"));
});

describe("session transitions", () => {
  it("accepts a supervised waiting cycle", () => { expect(canTransition("working", "waiting")).toBe(true); expect(canTransition("waiting", "working")).toBe(true); });
  it("rejects impossible idle-to-ready state", () => expect(canTransition("idle", "ready")).toBe(false));
});
