import { describe, expect, it } from "vitest";
import { readSpawns } from "./agentSpawns";

// Contract: testing/feat-agents-pane-auto-open.md, spawn detection.

describe("readSpawns", () => {
  it("seeds a chat's first reading without reporting what was already running", () => {
    const { next, spawned } = readSpawns(undefined, "chat", ["w1", "w2"]);
    expect(spawned).toEqual([]);
    expect([...next.seen]).toEqual(["w1", "w2"]);
  });

  it("reports a live agent it has not seen, newest last", () => {
    const first = readSpawns(undefined, "chat", ["w1"]).next;
    expect(readSpawns(first, "chat", ["w1", "w2", "w3"]).spawned).toEqual(["w2", "w3"]);
  });

  it("reseeds on a chat switch instead of reporting the other chat's agents", () => {
    const first = readSpawns(undefined, "chat", ["w1"]).next;
    const { next, spawned } = readSpawns(first, "other", ["o1"]);
    expect(spawned).toEqual([]);
    expect(next.chatId).toBe("other");
  });

  it("does not report an agent again after it finishes and returns", () => {
    let watch = readSpawns(undefined, "chat", []).next;
    watch = readSpawns(watch, "chat", ["w1"]).next;
    watch = readSpawns(watch, "chat", []).next;
    expect(readSpawns(watch, "chat", ["w1"]).spawned).toEqual([]);
  });
});
