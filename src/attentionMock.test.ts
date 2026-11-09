import { beforeEach, describe, expect, it } from "vitest";
import { mockListAttention, mockMarkAttentionSeen, onMockAttentionChanged, pushMockAttention, resetMockAttention, resolveMockAttention } from "./attentionMock";

describe("attention mock ledger", () => {
  beforeEach(() => resetMockAttention());

  it("supersedes a chat's older unseen rows and skips them on read", () => {
    pushMockAttention({ kind: "needsYou", sessionId: "a", title: "Bridge needs you", approvalEventId: 4 });
    pushMockAttention({ kind: "finished", sessionId: "b", title: "Turn completed" });
    pushMockAttention({ kind: "finished", sessionId: "a", title: "Turn completed" });
    const page = mockListAttention();
    expect(page.latestSequence).toBe(3);
    expect(page.items.map(item => [item.sequence, item.sessionId, item.kind])).toEqual([[2, "b", "finished"], [3, "a", "finished"]]);
    expect(mockListAttention({ afterSequence: 2 }).items.map(item => item.sequence)).toEqual([3]);
    expect(mockListAttention({ limit: 1 }).items).toHaveLength(1);
  });

  it("marks one chat seen, filters unseen, and keeps seen rows from being superseded", () => {
    pushMockAttention({ kind: "finished", sessionId: "a", title: "Turn completed" });
    pushMockAttention({ kind: "finished", sessionId: "b", title: "Turn completed" });
    expect(mockMarkAttentionSeen("a")).toEqual({ updated: 1 });
    expect(mockMarkAttentionSeen("a")).toEqual({ updated: 0 });
    expect(mockListAttention({ unseenOnly: true }).items.map(item => item.sessionId)).toEqual(["b"]);
    pushMockAttention({ kind: "failed", sessionId: "a", title: "Turn failed" });
    expect(mockListAttention().items.map(item => item.kind)).toEqual(["finished", "finished", "failed"]);
  });

  it("announces every change and stops after unsubscribe", () => {
    let calls = 0;
    const off = onMockAttentionChanged(() => { calls += 1; });
    const item = pushMockAttention({ kind: "needsYou", sessionId: "a", title: "Bridge needs you" });
    resolveMockAttention(item.sequence);
    resolveMockAttention(item.sequence);
    mockMarkAttentionSeen("a");
    off();
    pushMockAttention({ kind: "finished", sessionId: "a", title: "Turn completed" });
    expect(calls).toBe(3);
    expect(mockListAttention().items[0].resolvedAt).toBeTruthy();
  });
});
