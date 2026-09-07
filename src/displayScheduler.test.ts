import { describe, expect, it } from "vitest";
import { createDisplayScheduler, DISPLAY_BUDGET_MS, NATIVE_BATCH_BUDGET_MS, type DisplayClock } from "./displayScheduler";
import type { AgentEvent } from "./types";
import { asWireKind } from "./transcript/wire";

const event = (kind: string, text = "", sessionId = "s"): AgentEvent => ({
  id: 0, sequence: 0, protocolVersion: 1, kind: asWireKind(kind), text,
  sessionId, itemId: "m", role: null, title: null, status: null, data: {}, providerMeta: {}, createdAt: "now",
});
function setup() {
  let id = 0;
  const frames = new Map<number, () => void>();
  const timers = new Map<number, () => void>();
  const delays: number[] = [];
  const batches: AgentEvent[][] = [];
  const clock: DisplayClock = {
    frame: cb => { frames.set(++id, cb); return id; },
    cancelFrame: id => { frames.delete(id); },
    timeout: (cb, ms) => { delays.push(ms); timers.set(++id, cb); return id; },
    cancelTimeout: id => { timers.delete(id); },
  };
  const scheduler = createDisplayScheduler(batch => batches.push(batch), clock);
  return { scheduler, batches, frames, timers, delays };
}
describe("display scheduling", () => {
  it("delivers first content immediately then coalesces until the next frame", () => {
    const { scheduler, batches, frames, timers, delays } = setup();
    scheduler.push(event("message.delta", "a"));
    expect(batches).toHaveLength(1);
    scheduler.push(event("message.delta", "b"));
    scheduler.push(event("message.delta", "c"));
    expect(batches).toHaveLength(1);
    expect(frames.size).toBe(1);
    expect(delays).toEqual([DISPLAY_BUDGET_MS - NATIVE_BATCH_BUDGET_MS]);
    [...frames.values()][0]();
    expect(batches[1][0].text).toBe("bc");
    expect(timers.size).toBe(0);
  });
  it.each(["reasoning.completed", "message.completed", "tool.started", "approval.requested", "permission.requested", "turn.completed", "session.stopped", "error"])("flushes pending deltas before %s", kind => {
    const { scheduler, batches, frames } = setup();
    scheduler.push(event("reasoning.delta", "first"));
    scheduler.push(event("reasoning.delta", "second"));
    scheduler.push(event(kind));
    expect(batches[1].map(e => e.kind)).toEqual(["reasoning.delta", kind]);
    expect(frames.size).toBe(0);
  });
  it("drains in background windows without rAF and cancels on disposal", () => {
    const { scheduler, batches, frames, timers } = setup();
    scheduler.push(event("message.delta", "a"));
    scheduler.push(event("message.delta", "b"));
    [...timers.values()][0]();
    expect(batches).toHaveLength(2);
    expect(frames.size).toBe(0);
    scheduler.push(event("message.delta", "c"));
    const stale = [...timers.values()][0];
    scheduler.dispose();
    stale();
    scheduler.push(event("message.delta", "late"));
    expect(batches).toHaveLength(2);
    expect(scheduler.pendingCount()).toBe(0);
  });
  it("preserves order across session switches and resets first content per turn", () => {
    const { scheduler, batches } = setup();
    scheduler.push(event("message.delta", "a"));
    scheduler.push(event("message.delta", "b"));
    scheduler.push(event("message.delta", "c", "other"));
    expect(batches[1].map(e => e.text)).toEqual(["b", "c"]);
    scheduler.push(event("turn.started", "", "other"));
    scheduler.push(event("message.delta", "new", "other"));
    expect(batches.at(-1)?.[0].text).toBe("new");
  });
  it("coalesces sustained concurrent workers after each first frame", () => {
    const { scheduler, batches, frames } = setup();
    for (const session of ["a", "b", "c"]) scheduler.push(event("message.delta", "first", session));
    expect(batches).toHaveLength(3);
    for (let i = 0; i < 10; i++) for (const session of ["a", "b", "c"]) scheduler.push(event("message.delta", "next", session));
    expect(batches).toHaveLength(3);
    [...frames.values()][0]();
    expect(batches[3]).toHaveLength(3);
  });
  it("bounds queued bursts without losing distinct frames", () => {
    const { scheduler, batches } = setup();
    for (let i = 0; i < 5000; i++) {
      scheduler.push({ ...event("message.delta", String(i)), itemId: `m${i}` });
      expect(scheduler.pendingCount()).toBeLessThan(256);
    }
    scheduler.flush();
    expect(batches.flat()).toHaveLength(5000);
  });
});
