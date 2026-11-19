// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { onTerminalRun, requestTerminalRun, type TerminalRun } from "./runQueue";

// Contract: testing/feat-run-in-terminal.md §Unit Tests (runQueue).

let seq = 0;
let ws = "";
const offs: (() => void)[] = [];
const listen = (workspaceId: string, listener: (run: TerminalRun) => void) => { offs.push(onTerminalRun(workspaceId, listener)); };

beforeEach(() => { vi.useFakeTimers(); ws = `ws-${++seq}`; });
afterEach(() => { offs.splice(0).forEach(off => off()); vi.useRealTimers(); });

describe("terminal run queue", () => {
  it("holds a request until a pane for its workspace listens", async () => {
    const result = requestTerminalRun(ws, "ls");
    const taken: TerminalRun[] = [];
    listen(ws, run => taken.push(run));
    expect(taken.map(run => run.command)).toEqual(["ls"]);
    taken[0].settle("shell 1");
    await expect(result).resolves.toEqual({ shell: "shell 1" });
  });

  it("hands a request to a live pane at once", () => {
    const taken: string[] = [];
    listen(ws, run => taken.push(run.command));
    void requestTerminalRun(ws, "pwd");
    expect(taken).toEqual(["pwd"]);
  });

  it("does not deliver another workspace's request", () => {
    const taken: string[] = [];
    listen(ws, run => taken.push(run.command));
    void requestTerminalRun(`${ws}-other`, "pwd").catch(() => undefined);
    expect(taken).toEqual([]);
  });

  it("fails a request nobody takes in time and never delivers it later", async () => {
    const result = requestTerminalRun(ws, "rm -rf build", 1_000);
    const failed = expect(result).rejects.toThrow("The terminal did not open in time.");
    vi.advanceTimersByTime(1_000);
    await failed;
    const taken: string[] = [];
    listen(ws, run => taken.push(run.command));
    expect(taken).toEqual([]);
  });

  it("closes a run once settled so a late timeout or second settle is a no-op", async () => {
    let taken: TerminalRun | undefined;
    listen(ws, run => { taken = run; });
    const result = requestTerminalRun(ws, "ls", 1_000);
    expect(taken?.open()).toBe(true);
    taken!.settle("shell 2");
    taken!.fail(new Error("late"));
    vi.advanceTimersByTime(2_000);
    expect(taken?.open()).toBe(false);
    await expect(result).resolves.toEqual({ shell: "shell 2" });
  });

  it("delivers to the newest pane and stops after it unsubscribes", () => {
    const first: string[] = [], second: string[] = [];
    const offFirst = onTerminalRun(ws, run => first.push(run.command));
    const offSecond = onTerminalRun(ws, run => second.push(run.command));
    offFirst();
    void requestTerminalRun(ws, "a");
    offSecond();
    void requestTerminalRun(ws, "b").catch(() => undefined);
    expect(first).toEqual([]);
    expect(second).toEqual(["a"]);
  });
});
