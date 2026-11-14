import { beforeEach, describe, expect, it } from "vitest";
import { mockListAttention, mockMarkAttentionSeen, mockMarkAttentionUnread, mockUnreadSummary, onMockAttentionChanged, pushMockAttention, resetMockAttention, resolveMockAttention } from "./attentionMock";

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

  it("marks a session unread again the way core does", () => {
    const top = () => null;
    expect(mockMarkAttentionUnread("a"), "no row, nothing to mark").toEqual({ updated: 0 });
    pushMockAttention({ kind: "failed", sessionId: "a", title: "Turn failed" });
    pushMockAttention({ kind: "finished", sessionId: "a", title: "Turn completed" });
    mockMarkAttentionSeen("a");
    expect(mockUnreadSummary(top).total).toBe(0);
    let calls = 0;
    const off = onMockAttentionChanged(() => { calls += 1; });
    expect(mockMarkAttentionUnread("a")).toEqual({ updated: 1 });
    expect(mockMarkAttentionUnread("a"), "already unread").toEqual({ updated: 0 });
    off();
    expect(calls).toBe(1);
    expect(mockUnreadSummary(top).byChat).toEqual({ a: "finished" });
    // An answered request waits on nobody.
    const request = pushMockAttention({ kind: "needsYou", sessionId: "b", title: "Bridge needs you" });
    resolveMockAttention(request.sequence);
    mockMarkAttentionSeen("b");
    expect(mockMarkAttentionUnread("b")).toEqual({ updated: 0 });
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

  describe("unread summary", () => {
    // chat <- worker <- nested, and a separate top-level chat.
    const parents: Record<string, string | null> = { chat: null, worker: "chat", nested: "worker", other: null };
    const parentOf = (id: string) => parents[id];

    it("rolls a worker's request up to its chat and prefers the stronger kind", () => {
      pushMockAttention({ kind: "finished", sessionId: "chat", title: "Turn completed" });
      pushMockAttention({ kind: "needsYou", sessionId: "nested", title: "Bridge needs you" });
      pushMockAttention({ kind: "failed", sessionId: "other", title: "Turn failed" });
      pushMockAttention({ kind: "finished", sessionId: "worker", title: "Turn completed" });
      expect(mockUnreadSummary(parentOf)).toEqual({
        total: 2,
        needsYou: 1,
        byChat: { chat: "needsYou", other: "failed" },
        bySession: { chat: "finished", nested: "needsYou", other: "failed", worker: "finished" },
      });
    });

    it("skips seen, answered and gone sessions", () => {
      pushMockAttention({ kind: "finished", sessionId: "chat", title: "Turn completed" });
      mockMarkAttentionSeen("chat");
      const request = pushMockAttention({ kind: "needsYou", sessionId: "worker", title: "Bridge needs you" });
      resolveMockAttention(request.sequence);
      pushMockAttention({ kind: "finished", sessionId: "archived", title: "Turn completed" });
      expect(mockUnreadSummary(parentOf)).toEqual({ total: 0, needsYou: 0, byChat: {}, bySession: {} });
    });
  });
});
