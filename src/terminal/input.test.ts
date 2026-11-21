import { describe, expect, it, vi } from "vitest";
import { TerminalInput } from "./input";

// Contract: testing/fix-terminal-sweep.md §3.
function deferred() {
  let resolve!: () => void, reject!: (error: unknown) => void;
  const promise = new Promise<void>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
const settle = () => new Promise(resolve => setTimeout(resolve, 0));

describe("TerminalInput", () => {
  it("keeps one write in flight per terminal", () => {
    const first = deferred();
    const write = vi.fn(() => first.promise);
    const input = new TerminalInput(write, vi.fn());
    input.send("a");
    input.send("b");
    expect(write).toHaveBeenCalledTimes(1);
    expect(write).toHaveBeenCalledWith("a", false);
  });

  it("coalesces keystrokes typed during a write and keeps their order", async () => {
    const pending = [deferred(), deferred()];
    const write = vi.fn((_data: string, _binary: boolean) => pending[write.mock.calls.length - 1].promise);
    const input = new TerminalInput(write, vi.fn());
    input.send("a"); input.send("b"); input.send("c");
    pending[0].resolve();
    await settle();
    expect(write.mock.calls.map(call => call[0])).toEqual(["a", "bc"]);
    pending[1].resolve();
  });

  it("never merges binary input with text", async () => {
    const first = deferred();
    const write = vi.fn((_data: string, _binary: boolean) => write.mock.calls.length === 1 ? first.promise : Promise.resolve());
    const input = new TerminalInput(write, vi.fn());
    input.send("x");
    input.send("ls"); input.send("\x1b[M \xff!", true); input.send("\r");
    first.resolve();
    await settle();
    expect(write.mock.calls).toEqual([["x", false], ["ls", false], ["\x1b[M \xff!", true], ["\r", false]]);
  });

  it("reports a failed write and keeps sending", async () => {
    const onError = vi.fn();
    const write = vi.fn((_data: string, _binary: boolean) => write.mock.calls.length === 1 ? Promise.reject(new Error("busy")) : Promise.resolve());
    const input = new TerminalInput(write, onError);
    input.send("a");
    await settle();
    input.send("b");
    await settle();
    expect(onError).toHaveBeenCalledWith(new Error("busy"));
    expect(write.mock.calls.map(call => call[0])).toEqual(["a", "b"]);
  });

  it("drops input after close", async () => {
    const write = vi.fn(() => Promise.resolve());
    const input = new TerminalInput(write, vi.fn());
    input.close();
    input.send("a");
    await settle();
    expect(write).not.toHaveBeenCalled();
  });
});
