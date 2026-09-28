// @vitest-environment jsdom
// The zoom arithmetic is pure and needs no webview; the mutation path at the
// bottom dispatches a DOM event to announce the level, which does. The point of
// most of these is that a step is *small*: the bug they guard is the one where
// a single press jumped a fifth of the window and a single pinch jumped to 360%.

import { describe, expect, it, vi } from "vitest";
import {
  ACTUAL_SIZE,
  canZoom,
  chargeWheel,
  readZoom,
  snapZoom,
  stepZoom,
  WHEEL_RUNG_PX,
  writeZoom,
  zoomIn,
  zoomOut,
  zoomReset,
  ZOOM_STEPS,
  ZOOM_STORAGE_KEY,
} from "./zoom";

/** A storage double, so the persistence tests never touch the real one. */
function storage(seed: Record<string, string> = {}) {
  const map = new Map(Object.entries(seed));
  return {
    getItem: (key: string) => map.get(key) ?? null,
    setItem: (key: string, value: string) => { map.set(key, value); },
    read: (key: string) => map.get(key) ?? null,
  };
}

describe("the zoom ladder", () => {
  it("takes a tenth on the first press, not a fifth", () => {
    // The literal that matters. Tauri's polyfill stepped a flat 0.2.
    expect(zoomIn(ACTUAL_SIZE)).toBe(1.1);
    expect(zoomOut(ACTUAL_SIZE)).toBe(0.9);
  });

  it("gets coarser the further you go, so a step still reads as a step", () => {
    const above = ZOOM_STEPS.filter(step => step > ACTUAL_SIZE);
    const gaps = above.map((step, index) => step - (index === 0 ? ACTUAL_SIZE : above[index - 1]));
    // The first press is a tenth, the last is a whole 100%.
    expect(gaps[0]).toBeCloseTo(0.1, 5);
    expect(gaps[gaps.length - 1]).toBeGreaterThan(gaps[0]);
    for (let index = 1; index < gaps.length; index += 1) {
      expect(gaps[index], `gap ${index} grows`).toBeGreaterThanOrEqual(gaps[index - 1]);
    }
  });

  it("is sorted and holds actual size", () => {
    expect([...ZOOM_STEPS].sort((a, b) => a - b)).toEqual([...ZOOM_STEPS]);
    expect(ZOOM_STEPS).toContain(ACTUAL_SIZE);
    expect(new Set(ZOOM_STEPS).size).toBe(ZOOM_STEPS.length);
  });

  it("stops at both ends rather than running off them", () => {
    const top = ZOOM_STEPS[ZOOM_STEPS.length - 1];
    const bottom = ZOOM_STEPS[0];
    expect(zoomIn(top)).toBe(top);
    expect(zoomOut(bottom)).toBe(bottom);
    expect(canZoom(top, 1)).toBe(false);
    expect(canZoom(bottom, -1)).toBe(false);
    expect(canZoom(ACTUAL_SIZE, 1)).toBe(true);
  });

  it("returns to actual size", () => {
    expect(zoomReset()).toBe(1);
    expect(zoomReset()).toBe(ACTUAL_SIZE);
  });

  it("lands back where it started after a long in, out, in", () => {
    // The polyfill kept a running float and added 0.2 each time. Moving by
    // index means there is no total to drift. Four each way stays clear of both
    // ends, which the clamp test above covers.
    let level = ACTUAL_SIZE;
    for (let i = 0; i < 4; i += 1) level = zoomIn(level);
    expect(level).toBe(1.75);
    for (let i = 0; i < 4; i += 1) level = zoomOut(level);
    expect(level).toBe(ACTUAL_SIZE);
  });

  it("does not drift across a hundred alternations", () => {
    let level = ACTUAL_SIZE;
    for (let i = 0; i < 100; i += 1) level = zoomIn(level);
    for (let i = 0; i < 100; i += 1) level = zoomOut(level);
    // Ends included: the clamp has to be lossless in the other direction too.
    expect(level).toBe(ZOOM_STEPS[0]);
    for (let i = 0; i < 100; i += 1) level = zoomIn(level);
    expect(level).toBe(ZOOM_STEPS[ZOOM_STEPS.length - 1]);
  });

  it("snaps anything off the ladder onto it", () => {
    expect(snapZoom(1.3)).toBe(1.25);
    expect(snapZoom(0.1)).toBe(ZOOM_STEPS[0]);
    expect(snapZoom(99)).toBe(ZOOM_STEPS[ZOOM_STEPS.length - 1]);
    expect(snapZoom(Number.NaN)).toBe(ACTUAL_SIZE);
    expect(ZOOM_STEPS).toContain(snapZoom(1.234));
  });

  it("takes several rungs at once, for a pinch that crossed two", () => {
    expect(stepZoom(ACTUAL_SIZE, 3)).toBe(1.5);
    expect(stepZoom(ACTUAL_SIZE, -3)).toBe(0.75);
    expect(stepZoom(ACTUAL_SIZE, 0)).toBe(ACTUAL_SIZE);
  });
});

describe("charging a pinch against the ladder", () => {
  it("spends one rung on a whole gesture, however many events it arrives in", () => {
    // The reported bug: eight events at a flat 0.2 each reached 360%.
    let pending = 0;
    let rungs = 0;
    for (let event = 0; event < 8; event += 1) {
      const charged = chargeWheel(pending, -15);
      pending = charged.pending;
      rungs += charged.rungs;
    }
    expect(rungs).toBe(1);
  });

  it("ignores travel too small to be a gesture", () => {
    // Ordinary scrolling must never zoom.
    expect(chargeWheel(0, -1).rungs).toBe(0);
    expect(chargeWheel(0, 1).rungs).toBe(0);
  });

  it("zooms in on upward travel and out on downward", () => {
    expect(chargeWheel(0, -WHEEL_RUNG_PX).rungs).toBe(1);
    expect(chargeWheel(0, WHEEL_RUNG_PX).rungs).toBe(-1);
  });

  it("carries the remainder, so several nudges add up to a step", () => {
    let pending = 0;
    let rungs = 0;
    for (let event = 0; event < 10; event += 1) {
      const charged = chargeWheel(pending, -10);
      pending = charged.pending;
      rungs += charged.rungs;
    }
    expect(rungs).toBe(1);
    expect(pending).toBe(0);
  });

  it("survives a nonsense delta or rung size without moving", () => {
    expect(chargeWheel(0, Number.NaN)).toEqual({ pending: 0, rungs: 0 });
    expect(chargeWheel(0, -500, 0)).toEqual({ pending: 0, rungs: 0 });
  });
});

describe("remembering the level", () => {
  it("survives a relaunch", () => {
    const store = storage();
    writeZoom(1.5, store);
    // A fresh read is a fresh process, which is the whole point.
    expect(readZoom(storage({ [ZOOM_STORAGE_KEY]: store.read(ZOOM_STORAGE_KEY)! }))).toBe(1.5);
  });

  it("reads actual size when there is nothing stored, or nothing usable", () => {
    expect(readZoom(storage())).toBe(ACTUAL_SIZE);
    expect(readZoom(storage({ [ZOOM_STORAGE_KEY]: "not a number" }))).toBe(ACTUAL_SIZE);
    expect(readZoom(storage({ [ZOOM_STORAGE_KEY]: "" }))).toBe(ACTUAL_SIZE);
  });

  it("snaps a stored value onto the ladder rather than trusting it", () => {
    expect(readZoom(storage({ [ZOOM_STORAGE_KEY]: "1.3" }))).toBe(1.25);
  });

  it("still renders when storage throws", () => {
    const hostile = {
      getItem: () => { throw new Error("denied"); },
      setItem: () => { throw new Error("denied"); },
    };
    expect(readZoom(hostile)).toBe(ACTUAL_SIZE);
    expect(() => writeZoom(1.25, hostile)).not.toThrow();
  });
});

// The mutation path, with Tauri's command behind a promise we control. The
// regression these guard: a level reserved only after the apply resolves, so a
// second command arriving in the meantime re-requested the same rung and the
// user saw one step where they asked for two.
//
// This project's jsdom does not provide localStorage, though the webview does.
// Installing a double makes the module's default storage resolve to it, which is
// the same injection missionControlSettings.test.ts does by parameter.
function installStorage(): Map<string, string> {
  const map = new Map<string, string>();
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    writable: true,
    value: {
      get length() { return map.size; },
      clear: () => map.clear(),
      getItem: (key: string) => map.get(key) ?? null,
      key: (index: number) => [...map.keys()][index] ?? null,
      removeItem: (key: string) => { map.delete(key); },
      setItem: (key: string, value: string) => { map.set(key, String(value)); },
    },
  });
  return map;
}

// A command that holds at the webview until the test lets it through. The gate
// stays open once opened, because a queued apply only starts after the one
// before it settles and would otherwise wait on a gate nobody holds.
async function withMockedInvoke() {
  vi.resetModules();
  const store = installStorage();
  const calls: number[] = [];
  let open = false;
  let waiters: Array<() => void> = [];
  vi.doMock("@tauri-apps/api/core", () => ({
    invoke: async (_command: string, args: { value: number }) => {
      calls.push(args.value);
      if (!open) await new Promise<void>(resolve => { waiters.push(resolve); });
      return null;
    },
  }));
  const zoom = await import("./zoom");
  return {
    ...zoom,
    calls,
    store,
    openGate: () => { open = true; waiters.splice(0).forEach(resolve => resolve()); },
  };
}

describe("serializing the level", () => {
  it("composes two zoom-ins that overlap, instead of collapsing to one", async () => {
    // The regression: a level reserved only after the apply resolves, so the
    // second command re-requested the same rung and one step was lost.
    const zoom = await withMockedInvoke();

    // Neither awaited: the second lands while the first is still in flight,
    // which is a key repeat or a second threshold inside one pinch.
    const first = zoom.nudgeZoom(1);
    const second = zoom.nudgeZoom(1);
    zoom.openGate();
    await Promise.all([first, second]);

    expect(zoom.calls).toEqual([1.1, 1.25]);
    // The intent is persisted before the apply, so it survives either way.
    expect(zoom.store.get(ZOOM_STORAGE_KEY)).toBe("1.25");
  });

  it("keeps the last write, so a slow apply cannot undo a later one", async () => {
    const zoom = await withMockedInvoke();
    const settled: number[] = [];
    const first = zoom.nudgeZoom(1).then(level => settled.push(level));
    const second = zoom.nudgeZoom(1).then(level => settled.push(level));
    zoom.openGate();
    await Promise.all([first, second]);

    // Both applies ran, in order, and the second is where the webview ends up.
    expect(zoom.calls).toEqual([1.1, 1.25]);
    expect(settled).toEqual([1.1, 1.25]);
  });

  it("composes a pinch that crosses two rungs with a key pressed between", async () => {
    const zoom = await withMockedInvoke();
    const pinch = zoom.setZoomLevel(zoom.stepZoom(1, 2));
    const key = zoom.nudgeZoom(1);
    zoom.openGate();
    await Promise.all([pinch, key]);

    expect(zoom.calls).toEqual([1.25, 1.5]);
  });

  it("falls back to the level the webview kept when an apply fails", async () => {
    vi.resetModules();
    const store = installStorage();
    const calls: number[] = [];
    let failNext = true;
    vi.doMock("@tauri-apps/api/core", () => ({
      invoke: async (_command: string, args: { value: number }) => {
        calls.push(args.value);
        if (failNext) { failNext = false; throw new Error("no webview"); }
        return null;
      },
    }));
    const zoom = await import("./zoom");

    // The first apply fails, so the webview is still at 100%.
    await expect(zoom.nudgeZoom(1)).resolves.toBe(1);
    expect(store.get(ZOOM_STORAGE_KEY)).toBe("1");
    // The next command starts from 100%, not from the 110% that never arrived.
    await expect(zoom.nudgeZoom(1)).resolves.toBe(1.1);
    expect(calls).toEqual([1.1, 1.1]);
  });

  it("keeps working after a failure, rather than wedging the queue", async () => {
    vi.resetModules();
    installStorage();
    vi.doMock("@tauri-apps/api/core", () => ({
      invoke: async (_command: string, args: { value: number }) => {
        if (args.value === 1.25) throw new Error("transient");
        return null;
      },
    }));
    const zoom = await import("./zoom");

    await zoom.nudgeZoom(1);          // 1.1, fine
    await expect(zoom.nudgeZoom(1)).resolves.toBe(1.1); // 1.25, fails
    await expect(zoom.nudgeZoom(1)).resolves.toBe(1.1); // and the next still runs
  });
});
