import { describe, expect, it, vi } from "vitest";
import { createCoalescedRefresh, startSerialPoll } from "./polling";

describe("createCoalescedRefresh", () => {
  it("merges a notification burst and still reads changes arriving mid-flight", async () => {
    const finish: Array<() => void> = [];
    let source = 0;
    let displayed = -1;
    const task = vi.fn(() => {
      const snapshot = source;
      return new Promise<void>(resolve => finish.push(() => { displayed = snapshot; resolve(); }));
    });
    const refresh = createCoalescedRefresh(task);
    const initial = refresh();
    await Promise.resolve();
    expect(task).toHaveBeenCalledTimes(1);

    source = 1;
    const burst = Array.from({ length: 60 }, () => refresh());
    expect(task).toHaveBeenCalledTimes(1);
    finish.shift()!();
    await Promise.resolve();
    expect(task).toHaveBeenCalledTimes(2);
    expect(displayed).toBe(0);
    finish.shift()!();
    await Promise.all([initial, ...burst]);
    expect(displayed).toBe(1);
    expect(task).toHaveBeenCalledTimes(2);
  });

  it("can refresh again after a failure without retrying forever", async () => {
    const failure = new Error("disconnected");
    const task = vi.fn().mockRejectedValueOnce(failure).mockResolvedValue(undefined);
    const refresh = createCoalescedRefresh(task);
    await expect(refresh()).rejects.toBe(failure);
    await expect(refresh()).resolves.toBeUndefined();
    expect(task).toHaveBeenCalledTimes(2);
  });
});

describe("startSerialPoll", () => {
  it("never overlaps a slow poll with the next interval", async () => {
    let resolveCurrent: (() => void) | undefined;
    let active = 0;
    let peakActive = 0;
    const task = vi.fn(() => new Promise<void>(resolve => {
      active += 1;
      peakActive = Math.max(peakActive, active);
      resolveCurrent = () => { active -= 1; resolve(); };
    }));
    const scheduled: Array<() => void> = [];
    const stop = startSerialPoll(task, 3_000, callback => {
      scheduled.push(callback);
      return scheduled.length as unknown as ReturnType<typeof setTimeout>;
    }, () => undefined);

    expect(task).toHaveBeenCalledTimes(1);
    expect(scheduled).toHaveLength(0);
    await Promise.resolve();
    expect(scheduled).toHaveLength(0);

    resolveCurrent?.();
    await Promise.resolve();
    expect(scheduled).toHaveLength(1);
    scheduled.shift()?.();
    expect(task).toHaveBeenCalledTimes(2);
    expect(peakActive).toBe(1);
    stop();
    resolveCurrent?.();
  });

  it("stops scheduling after cleanup while a poll is in flight", async () => {
    let resolveCurrent: (() => void) | undefined;
    const task = () => new Promise<void>(resolve => { resolveCurrent = resolve; });
    const schedule = vi.fn(() => 1 as unknown as ReturnType<typeof setTimeout>);
    const stop = startSerialPoll(task, 5_000, schedule, () => undefined);

    stop();
    resolveCurrent?.();
    await Promise.resolve();
    expect(schedule).not.toHaveBeenCalled();
  });

  it("keeps polling after a transient task failure", async () => {
    const scheduled: Array<() => void> = [];
    const task = vi.fn().mockRejectedValueOnce(new Error("temporary")).mockResolvedValue(undefined);
    const stop = startSerialPoll(task, 3_000, callback => {
      scheduled.push(callback);
      return scheduled.length as unknown as ReturnType<typeof setTimeout>;
    }, () => undefined);

    await Promise.resolve();
    await Promise.resolve();
    expect(scheduled).toHaveLength(1);
    scheduled.shift()?.();
    await Promise.resolve();
    expect(task).toHaveBeenCalledTimes(2);
    stop();
  });
});
