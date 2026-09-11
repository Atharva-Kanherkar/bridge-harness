import { expect, it, vi } from "vitest";
import { TerminalReplay } from "./replay";
import type { TerminalFrame, TerminalSnapshot } from "./types";

const snapshot = (sequence = 1, generation = "g"): TerminalSnapshot => ({ record: { workspaceId: "w", terminalId: "t", generation, title: "Shell", cwd: "/tmp", rows: 24, cols: 80, status: "running", createdAt: "now" }, sequence, ansi: `screen-${sequence}` });
const frame = (sequence: number, data = "output", generation = "g"): TerminalFrame => ({ workspaceId: "w", terminalId: "t", generation, sequence, data });
const settle = () => new Promise(resolve => setTimeout(resolve, 0));

it("merges the snapshot race exactly once and parses resizes in order", async () => {
  let resolve!: (value: TerminalSnapshot) => void;
  const restore = vi.fn(async (_snapshot: TerminalSnapshot) => {}), output = vi.fn(async (_frame: TerminalFrame) => {});
  const replay = new TerminalReplay({ snapshot: () => new Promise(done => { resolve = done; }), restore, frame: output, error: vi.fn() });
  const ready = replay.recover();
  replay.receive(frame(1, "already in snapshot"));
  replay.receive({ ...frame(2, ""), rows: 30, cols: 90 });
  replay.receive(frame(3, "new"));
  resolve(snapshot()); await ready;
  replay.receive(frame(3, "duplicate")); await settle();
  expect(restore).toHaveBeenCalledOnce();
  expect(output.mock.calls.map(call => call[0].sequence)).toEqual([2, 3]);
  replay.dispose();
});
it("recovers a missing event from the host instead of appending a broken tail", async () => {
  const get = vi.fn().mockResolvedValueOnce(snapshot(1)).mockResolvedValueOnce(snapshot(5));
  const restore = vi.fn(async () => {}), output = vi.fn(async () => {});
  const replay = new TerminalReplay({ snapshot: get, restore, frame: output, error: vi.fn() });
  await replay.recover(); replay.receive(frame(5)); await settle();
  expect(get).toHaveBeenCalledTimes(2);
  expect(output).not.toHaveBeenCalled();
  replay.receive(frame(6)); await settle(); expect(output).toHaveBeenCalledOnce();
  replay.dispose();
});
it("ignores retired generations and stops delivery after disposal", async () => {
  const get = vi.fn().mockResolvedValueOnce(snapshot(1)).mockResolvedValueOnce(snapshot(0, "new"));
  const output = vi.fn(async () => {});
  const replay = new TerminalReplay({ snapshot: get, restore: async () => {}, frame: output, error: vi.fn() });
  await replay.recover(); replay.receive(frame(1, "replacement", "new")); await settle();
  replay.receive(frame(99, "stale", "g")); await settle();
  expect(output).toHaveBeenCalledTimes(1);
  replay.dispose(); replay.receive(frame(2, "after detach", "new")); await settle();
  expect(output).toHaveBeenCalledTimes(1);
});
it("exposes snapshot failures and allows an explicit reconnect", async () => {
  const error = vi.fn();
  const get = vi.fn().mockRejectedValueOnce(new Error("offline")).mockResolvedValueOnce(snapshot());
  const restore = vi.fn(async () => {});
  const replay = new TerminalReplay({ snapshot: get, restore, frame: async () => {}, error });
  await replay.recover(); expect(error).toHaveBeenCalledOnce();
  await replay.recover(); expect(restore).toHaveBeenCalledOnce(); replay.dispose();
});
