import { expect, it, vi } from "vitest";
import { createSessionStops } from "./sessionStop";
const session = (id: string, status: "working" | "idle" | "stopped" = "working") => ({ id, status, activeTurnId: null });
it("pending Stop remains owned by the original chat after switching", async () => {
  const interrupt = vi.fn().mockResolvedValue(undefined);
  const stops = createSessionStops(interrupt, vi.fn(), vi.fn());
  stops.request(session("a", "idle"));
  expect(stops.has("a")).toBe(true); expect(stops.has("b")).toBe(false);
  stops.reconcile([session("a", "idle"), session("b")], new Set(["a"]));
  expect(interrupt).not.toHaveBeenCalled();
  stops.reconcile([session("a"), session("b")], new Set());
  expect(interrupt).toHaveBeenCalledTimes(1); expect(interrupt).toHaveBeenCalledWith("a");
  await Promise.resolve();
  stops.reconcile([session("a", "idle"), session("b")], new Set());
  expect(stops.has("a")).toBe(false);
});
it("concurrent stops settle independently and failures remain visible", async () => {
  let finish!: () => void;
  const failed = vi.fn();
  const interrupt = vi.fn((id: string) => id === "a" ? new Promise<void>(resolve => { finish = resolve; }) : Promise.reject(new Error("offline")));
  const stops = createSessionStops(interrupt, vi.fn(), failed);
  stops.request(session("a")); stops.request(session("b")); stops.request(session("a"));
  await Promise.resolve();
  expect(interrupt).toHaveBeenCalledTimes(2);
  expect(stops.has("a")).toBe(true); expect(stops.has("b")).toBe(false);
  expect(failed).toHaveBeenCalledWith("b", expect.any(Error));
  finish(); await Promise.resolve();
  stops.reconcile([session("a", "idle")], new Set());
  expect(stops.has("a")).toBe(false);
});

it("clears Stop on the committed stopped state before process disposal replies", () => {
  const stops = createSessionStops(() => new Promise<void>(() => {}), vi.fn(), vi.fn());
  stops.request(session("a"));
  stops.reconcile([{ id: "a", status: "stopped", activeTurnId: null }, session("b")], new Set());
  expect(stops.has("a")).toBe(false);
  expect(stops.has("b")).toBe(false);
});

it("invalidates preparation immediately without launching a turn to stop it", () => {
  const interrupt = vi.fn();
  const stops = createSessionStops(interrupt, vi.fn(), vi.fn());
  const withdrawn = vi.fn();
  const first = stops.beginSend("a", withdrawn);
  const other = stops.beginSend("b", withdrawn);
  stops.request(session("a", "idle"));
  expect(withdrawn).toHaveBeenCalledOnce();
  expect(first.withdrawn).toBe(true);
  expect(first.cancelled).toBe(true);
  expect(other.cancelled).toBe(false);
  expect(stops.has("a")).toBe(false);
  expect(interrupt).not.toHaveBeenCalled();
  const fresh = stops.beginSend("a");
  first.finish();
  expect(fresh.cancelled).toBe(false);
});

it("stops a cancelled startup as soon as it returns and holds later sends until settlement", async () => {
  const interrupt = vi.fn().mockResolvedValue(undefined);
  const stops = createSessionStops(interrupt, vi.fn(), vi.fn());
  const starting = stops.beginSend("a");
  starting.starting();
  stops.request(session("a", "idle"));
  const next = stops.beginSend("a");
  let released = false;
  void next.ready!.then(() => { released = true; });
  expect(starting.cancelled).toBe(true);
  expect(interrupt).not.toHaveBeenCalled();
  starting.stopIfCancelled(); starting.stopIfCancelled();
  expect(interrupt).toHaveBeenCalledOnce();
  await Promise.resolve();
  expect(released).toBe(false);
  starting.finish();
  stops.reconcile([{ id: "a", status: "stopped", activeTurnId: null }], new Set(["a"]));
  await next.ready;
  expect(released).toBe(true);
  expect(next.cancelled).toBe(false);
});

it("disposal invalidates late work and releases queued sends", async () => {
  const stops = createSessionStops(() => new Promise<void>(() => {}), vi.fn(), vi.fn());
  const active = stops.beginSend("a"); active.submitted();
  stops.request(session("a"));
  const next = stops.beginSend("a");
  stops.dispose();
  await next.ready;
  expect(active.cancelled).toBe(true);
  expect(next.cancelled).toBe(true);
});

it("keeps a submitted message's bubble: only unsent words are withdrawn", () => {
  const stops = createSessionStops(vi.fn().mockResolvedValue(undefined), vi.fn(), vi.fn());
  const sentWithdrawn = vi.fn();
  const startingWithdrawn = vi.fn();
  const sent = stops.beginSend("a", sentWithdrawn); sent.submitted();
  const starting = stops.beginSend("a", startingWithdrawn); starting.starting();
  stops.request(session("a"));
  expect(sentWithdrawn).not.toHaveBeenCalled();
  expect(sent.cancelled).toBe(true);
  expect(sent.withdrawn).toBe(false);
  expect(startingWithdrawn).toHaveBeenCalledOnce();
  expect(starting.withdrawn).toBe(true);
});

it("keeps a Stop pressed during startup of a previously stopped chat until that startup is stopped", async () => {
  const interrupt = vi.fn().mockResolvedValue(undefined);
  const stops = createSessionStops(interrupt, vi.fn(), vi.fn());
  const starting = stops.beginSend("a"); starting.starting();
  stops.request(session("a", "stopped"));
  const next = stops.beginSend("a");
  let released = false;
  void next.ready!.then(() => { released = true; });
  // The previous turn's `stopped` is not news about this request.
  stops.reconcile([session("a", "stopped")], new Set(["a"]));
  expect(stops.has("a")).toBe(true);
  starting.stopIfCancelled();
  expect(interrupt).toHaveBeenCalledOnce();
  starting.finish();
  stops.reconcile([session("a", "stopped")], new Set());
  await Promise.resolve();
  expect(released, "a later send waits for this startup to be stopped").toBe(false);
  await Promise.resolve();
  stops.reconcile([session("a", "stopped")], new Set());
  await next.ready;
  expect(stops.has("a")).toBe(false);
  expect(next.cancelled).toBe(false);
});

it("treats stopped as settlement once the chat has been seen live since Stop", () => {
  const stops = createSessionStops(() => new Promise<void>(() => {}), vi.fn(), vi.fn());
  const starting = stops.beginSend("a"); starting.starting();
  stops.request(session("a", "stopped"));
  stops.reconcile([session("a", "idle")], new Set(["a"]));
  starting.stopIfCancelled(); starting.finish();
  // The interrupt never answers, but the durable state says the launch is stopped.
  stops.reconcile([session("a", "stopped")], new Set());
  expect(stops.has("a")).toBe(false);
});
