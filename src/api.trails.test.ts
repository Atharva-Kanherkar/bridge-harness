import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "./api";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

beforeEach(() => {
  invoke.mockReset();
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
});
afterEach(() => vi.unstubAllGlobals());

describe("listTrails", () => {
  it("shares one request between calls made in the same task", async () => {
    let served = 0;
    invoke.mockImplementation(() => Promise.resolve({ trails: [{ id: `read-${++served}` }] }));
    const badge = bridgeApi.listTrails(true);
    const screen = bridgeApi.listTrails(true);
    const unarchived = bridgeApi.listTrails(false);
    const [first, second, third] = await Promise.all([badge, screen, unarchived]);
    expect(first).toBe(second);
    expect(third).not.toBe(first);
    expect(invoke).toHaveBeenCalledTimes(2);
    expect(invoke.mock.calls.map(call => call[1])).toEqual([
      expect.objectContaining({ includeArchived: true }),
      expect.objectContaining({ includeArchived: false }),
    ]);
  });

  it("never hands a later call a request that was already sent", async () => {
    let release!: (value: unknown) => void;
    invoke.mockImplementationOnce(() => new Promise(resolve => { release = resolve; }));
    invoke.mockImplementationOnce(() => Promise.resolve({ trails: [{ id: "after the change" }] }));
    const before = bridgeApi.listTrails(true);
    await Promise.resolve();
    await Promise.resolve();
    expect(invoke).toHaveBeenCalledTimes(1);
    // A trails/changed lands while the first request is in flight.
    const after = bridgeApi.listTrails(true);
    release({ trails: [{ id: "before the change" }] });
    expect((await before).trails[0].id).toBe("before the change");
    expect((await after).trails[0].id).toBe("after the change");
    expect(invoke).toHaveBeenCalledTimes(2);
  });
});
