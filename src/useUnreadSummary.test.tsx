// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "./api";
import { pushMockAttention, resetMockAttention } from "./attentionMock";
import type { UnreadSummary } from "./types";
import { NOTHING_UNREAD, useUnreadSummary } from "./useUnreadSummary";

const settle = (ms = 10) => act(async () => { await new Promise(resolve => setTimeout(resolve, ms)); });
const summaryOf = (byChat: UnreadSummary["byChat"]): UnreadSummary => ({ total: Object.keys(byChat).length, needsYou: 0, byChat, bySession: byChat });

function Probe({ sessionsKey, seen }: { sessionsKey: string; seen: UnreadSummary[] }) {
  seen.push(useUnreadSummary(sessionsKey));
  return null;
}

describe("useUnreadSummary", () => {
  let container: HTMLDivElement;
  let root: Root;
  let seen: UnreadSummary[];

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    resetMockAttention();
    seen = [];
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.restoreAllMocks();
  });

  const render = (sessionsKey: string) => act(async () => { root.render(<Probe sessionsKey={sessionsKey} seen={seen} />); });
  const latest = () => seen.at(-1);

  it("reads on mount and again on every hint", async () => {
    const read = vi.spyOn(bridgeApi, "unreadSummary");
    // A mock worker: its request marks the orchestrator above it.
    pushMockAttention({ kind: "needsYou", sessionId: "session-1w", title: "Bridge needs you" });
    await render("session-1");
    await settle();
    expect(read).toHaveBeenCalledTimes(1);
    expect(latest()?.byChat).toEqual({ "session-1": "needsYou" });

    await act(async () => { bridgeApi.markAttentionSeen("session-1w"); });
    await settle();
    expect(read).toHaveBeenCalledTimes(2);
    expect(latest()).toEqual(NOTHING_UNREAD);
  });

  it("rereads when the sessions change", async () => {
    const read = vi.spyOn(bridgeApi, "unreadSummary").mockResolvedValue(summaryOf({ a: "finished" }));
    await render("a,b");
    await settle();
    read.mockResolvedValue(NOTHING_UNREAD);
    await render("b");
    await settle();
    expect(read).toHaveBeenCalledTimes(2);
    expect(latest()).toEqual(NOTHING_UNREAD);
  });

  it("drops a response that lands after a newer read was sent", async () => {
    const answers: ((value: UnreadSummary) => void)[] = [];
    vi.spyOn(bridgeApi, "unreadSummary").mockImplementation(() => new Promise(resolve => { answers.push(resolve); }));
    await render("a");
    await settle();
    await act(async () => { pushMockAttention({ kind: "finished", sessionId: "a", title: "Turn completed" }); });
    expect(answers).toHaveLength(2);
    await act(async () => { answers[1](summaryOf({ a: "finished" })); });
    await act(async () => { answers[0](NOTHING_UNREAD); });
    expect(latest()?.byChat).toEqual({ a: "finished" });
  });

  it("keeps the last summary when a read fails", async () => {
    const read = vi.spyOn(bridgeApi, "unreadSummary").mockResolvedValue(summaryOf({ a: "failed" }));
    await render("a");
    await settle();
    read.mockRejectedValue(new Error("daemon restarting"));
    await act(async () => { pushMockAttention({ kind: "finished", sessionId: "a", title: "Turn completed" }); });
    await settle();
    expect(read).toHaveBeenCalledTimes(2);
    expect(latest()?.byChat).toEqual({ a: "failed" });
  });
});
