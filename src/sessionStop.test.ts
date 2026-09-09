import { expect, it, vi } from "vitest";
import { createSessionStops } from "./sessionStop";
const session = (id: string, status: "working" | "idle" = "working") => ({ id, status, activeTurnId: null });
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
