import { describe, expect, it, vi } from "vitest";
import { startSerialPoll } from "./polling";

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
