// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "./api";
import { mockListAttention, mockMarkAttentionSeen, pushMockAttention, resetMockAttention, resolveMockAttention } from "./attentionMock";
import type { AttentionItem } from "./types";
import { useAttentionFeed } from "./useAttentionFeed";

const HOLD_MS = 30;
const settle = (ms = 10) => act(async () => { await new Promise(resolve => setTimeout(resolve, ms)); });

function Probe({ onArrive }: { onArrive: (item: AttentionItem) => void }) {
  useAttentionFeed(onArrive, HOLD_MS);
  return null;
}

describe("useAttentionFeed", () => {
  let container: HTMLDivElement;
  let root: Root;
  let arrived: AttentionItem[];

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    resetMockAttention();
    arrived = [];
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.restoreAllMocks();
  });

  // A real read is asynchronous; make the mock one wait a tick too, so a
  // read sees every change made in the same act() block.
  function deferReads() {
    return vi.spyOn(bridgeApi, "listAttention").mockImplementation(async params => {
      await new Promise(resolve => setTimeout(resolve, 0));
      return mockListAttention(params);
    });
  }

  async function mount() {
    await act(async () => { root.render(<Probe onArrive={item => arrived.push(item)} />); });
    await settle();
  }

  it("starts after launch and delivers a new row once", async () => {
    pushMockAttention({ kind: "finished", sessionId: "before", title: "Turn completed" });
    await mount();
    expect(arrived).toEqual([]);

    await act(async () => { pushMockAttention({ kind: "finished", sessionId: "a", title: "Turn completed" }); });
    await settle();
    expect(arrived.map(item => item.sessionId)).toEqual(["a"]);

    // A hint with nothing new behind it (someone marked a chat seen).
    await act(async () => { mockMarkAttentionSeen("before"); });
    await settle();
    expect(arrived).toHaveLength(1);
  });

  it("keeps a row announced while the launch read is pending", async () => {
    let releaseLaunch: () => void = () => undefined;
    const launch = new Promise<void>(resolve => { releaseLaunch = resolve; });
    let first = true;
    vi.spyOn(bridgeApi, "listAttention").mockImplementation(async params => {
      if (first) {
        first = false;
        await launch;
      }
      return mockListAttention(params);
    });
    pushMockAttention({ kind: "finished", sessionId: "before", title: "Turn completed" });
    await act(async () => { root.render(<Probe onArrive={item => arrived.push(item)} />); });
    await settle(5);
    // Written after launch, announced while the launch read is still out.
    await act(async () => { pushMockAttention({ kind: "finished", sessionId: "after", title: "Turn completed" }); });
    await act(async () => { releaseLaunch(); });
    await settle(30);
    expect(arrived.map(item => item.sessionId)).toEqual(["after"]);
  });

  it("drops a request answered within the hold and delivers one still open", async () => {
    await mount();
    await act(async () => {
      const answered = pushMockAttention({ kind: "needsYou", sessionId: "auto", title: "Bridge needs you", approvalEventId: 3 });
      resolveMockAttention(answered.sequence);
    });
    await act(async () => { pushMockAttention({ kind: "needsYou", sessionId: "worker", title: "Bridge needs you", approvalEventId: 9 }); });
    await settle();
    expect(arrived, "held, not delivered yet").toEqual([]);
    await settle(HOLD_MS * 3);
    expect(arrived.map(item => [item.sessionId, item.approvalEventId])).toEqual([["worker", 9]]);
  });

  it("drops a request superseded within the hold", async () => {
    await mount();
    await act(async () => { pushMockAttention({ kind: "needsYou", sessionId: "s", title: "Bridge needs you", approvalEventId: 2 }); });
    await settle();
    await act(async () => { pushMockAttention({ kind: "finished", sessionId: "s", title: "Turn completed" }); });
    await settle(HOLD_MS * 3);
    expect(arrived.map(item => item.kind)).toEqual(["finished"]);
  });

  it("skips seen, resolved and session-less rows", async () => {
    await mount();
    deferReads();
    await act(async () => {
      pushMockAttention({ kind: "finished", sessionId: "seen", title: "Turn completed" });
      mockMarkAttentionSeen("seen");
      const resolved = pushMockAttention({ kind: "failed", sessionId: "resolved", title: "Turn failed" });
      resolveMockAttention(resolved.sequence);
      pushMockAttention({ kind: "finished", sessionId: null, title: "Turn completed" });
    });
    await settle(HOLD_MS * 3);
    expect(arrived).toEqual([]);
  });

  it("pages through a full page", async () => {
    await mount();
    const reads = deferReads();
    await act(async () => {
      for (let index = 0; index < 101; index += 1) pushMockAttention({ kind: "finished", sessionId: `s${index}`, title: "Turn completed" });
    });
    await settle(30);
    expect(arrived).toHaveLength(101);
    expect(reads.mock.calls.slice(0, 2).map(([params]) => params?.afterSequence)).toEqual([0, 100]);
  });

  it("skips kinds it does not know", async () => {
    await mount();
    await act(async () => {
      pushMockAttention({ kind: "other", sessionId: "newer", title: "Something new" });
      pushMockAttention({ kind: "laterKind" as AttentionItem["kind"], sessionId: "newest", title: "Later" });
    });
    await settle(HOLD_MS * 3);
    expect(arrived).toEqual([]);
  });

  it("cancels a pending hold on unmount", async () => {
    await mount();
    await act(async () => { pushMockAttention({ kind: "needsYou", sessionId: "s", title: "Bridge needs you" }); });
    await settle();
    await act(async () => root.unmount());
    root = createRoot(container);
    await settle(HOLD_MS * 3);
    expect(arrived).toEqual([]);
  });
});
