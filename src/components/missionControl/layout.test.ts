import { describe, expect, it } from "vitest";
import {
  displayOrder, fitGrid, moveInOrder, moveToEnd, moveToFront, parseLayout, planBoard, reconcileOrder,
  QUEUE_RAIL_WIDTH, QUEUE_STRIP_HEIGHT, TILE_GAP, TILE_MIN_HEIGHT, TILE_MIN_WIDTH,
} from "./layout";

// the board Mission Control gets at each window size: sidebar (248px) and the
// title bar plus toolbar (about 110px) come off, then the board's own padding.
const WINDOWS = [[720, 800], [1024, 700], [1380, 860], [2560, 1440]] as const;
const ZOOMS = [0.67, 1, 1.5] as const;
const board = (width: number, height: number, zoom: number) => ({ width: Math.max(0, width / zoom - 248 - 16), height: Math.max(0, height / zoom - 110 - 16) });

describe("fitGrid", () => {
  for (const [w, h] of WINDOWS) for (const zoom of ZOOMS) {
    const { width, height } = board(w, h, zoom);
    it(`fits 1 to 12 chats without overflow in a ${w}x${h} window at ${zoom}x`, () => {
      const maxCols = Math.max(1, Math.floor((width + TILE_GAP) / (TILE_MIN_WIDTH + TILE_GAP)));
      const maxRows = Math.max(1, Math.floor((height + TILE_GAP) / (TILE_MIN_HEIGHT + TILE_GAP)));
      for (let n = 1; n <= 12; n += 1) {
        const fit = fitGrid(width, height, n);
        expect(fit.shown).toBe(Math.min(n, maxCols * maxRows));
        expect(fit.shown).toBeGreaterThanOrEqual(1);
        expect(fit.cols * fit.rows).toBeGreaterThanOrEqual(fit.shown);
        expect(fit.cols).toBeLessThanOrEqual(maxCols);
        expect(fit.rows).toBeLessThanOrEqual(maxRows);
        const tileWidth = (width - (fit.cols - 1) * TILE_GAP) / fit.cols;
        const tileHeight = (height - (fit.rows - 1) * TILE_GAP) / fit.rows;
        expect(tileWidth).toBeGreaterThan(0);
        expect(tileHeight).toBeGreaterThan(0);
        // tiles only drop below the minimum when not even one would fit.
        if (maxCols * TILE_MIN_WIDTH <= width) expect(tileWidth).toBeGreaterThanOrEqual(TILE_MIN_WIDTH);
        if (maxRows * TILE_MIN_HEIGHT <= height) expect(tileHeight).toBeGreaterThanOrEqual(TILE_MIN_HEIGHT);
        // a full row or column never wastes a whole empty track.
        expect(fit.cols * (fit.rows - 1)).toBeLessThan(fit.shown);
      }
    });
  }

  it("shows one tile when the board is smaller than a tile", () => {
    expect(fitGrid(200, 150, 5)).toEqual({ cols: 1, rows: 1, shown: 1 });
  });

  it("caps the stage at the chosen limit", () => {
    expect(fitGrid(2312, 1290, 7, 2).shown).toBe(2);
    expect(fitGrid(2312, 1290, 7, 1)).toEqual({ cols: 1, rows: 1, shown: 1 });
    expect(fitGrid(2312, 1290, 3, 6).shown).toBe(3);
  });

  it("picks a balanced shape on a wide board", () => {
    expect(fitGrid(1116, 694, 2)).toMatchObject({ cols: 2, rows: 1 });
    expect(fitGrid(1116, 694, 4)).toMatchObject({ cols: 2, rows: 2 });
    expect(fitGrid(2296, 1274, 7)).toMatchObject({ cols: 4, rows: 2, shown: 7 });
  });

  it("shows everything in a square-ish grid before the board is measured", () => {
    expect(fitGrid(0, 0, 5)).toEqual({ cols: 3, rows: 2, shown: 5 });
    expect(fitGrid(0, 0, 5, 2)).toEqual({ cols: 2, rows: 1, shown: 2 });
    expect(fitGrid(0, 0, 0)).toEqual({ cols: 0, rows: 0, shown: 0 });
  });
});

describe("planBoard", () => {
  it("has no queue when everything fits", () => {
    expect(planBoard(1116, 694, 4).queue).toBeNull();
  });

  it("puts the queue in a right rail on a wide board and refits around it", () => {
    const plan = planBoard(1116, 694, 7);
    expect(plan.queue).toBe("rail");
    expect(plan).toEqual({ ...fitGrid(1116 - QUEUE_RAIL_WIDTH - TILE_GAP, 694, 7), queue: "rail" });
  });

  it("puts the queue in a chip strip on a narrow board", () => {
    const plan = planBoard(760, 694, 7);
    expect(plan.queue).toBe("strip");
    expect(plan).toEqual({ ...fitGrid(760, 694 - QUEUE_STRIP_HEIGHT - TILE_GAP, 7), queue: "strip" });
  });

  it("queues what a tile limit holds back", () => {
    expect(planBoard(2296, 1274, 3, 1)).toMatchObject({ shown: 1, queue: "rail" });
  });
});

describe("order helpers", () => {
  it("moves one chat before or after another and leaves the rest alone", () => {
    expect(moveInOrder(["a", "b", "c", "d"], "d", "b", "before")).toEqual(["a", "d", "b", "c"]);
    expect(moveInOrder(["a", "b", "c", "d"], "a", "c", "after")).toEqual(["b", "c", "a", "d"]);
    expect(moveInOrder(["a", "b"], "a", "a", "before")).toEqual(["a", "b"]);
    expect(moveInOrder(["a", "b"], "x", "b", "before")).toEqual(["a", "x", "b"]);
    expect(moveInOrder(["a", "b"], "x", "missing", "before")).toEqual(["a", "b", "x"]);
    expect(moveToFront(["a", "b", "c"], "c")).toEqual(["c", "a", "b"]);
    expect(moveToEnd(["a", "b", "c"], "a")).toEqual(["b", "c", "a"]);
  });

  it("puts chats that need you first and keeps both groups stable", () => {
    const waiting = new Set(["d", "b"]);
    expect(displayOrder(["a", "b", "c", "d", "e"], id => waiting.has(id))).toEqual(["b", "d", "a", "c", "e"]);
  });

  it("keeps finished chats in place and lets a new chat take a finished slot", () => {
    const active = new Set(["a", "c", "new"]);
    const keep = (id: string) => id !== "gone";
    const next = reconcileOrder(["a", "done", "gone", "c"], { keep, incoming: ["a", "c", "new"], replaceable: id => !active.has(id) });
    expect(next).toEqual(["a", "new", "c"]);
  });

  it("appends when no finished, unpinned slot is free", () => {
    expect(reconcileOrder(["a", "pinned-done"], { keep: () => true, incoming: ["a", "pinned-done", "b"], replaceable: () => false })).toEqual(["a", "pinned-done", "b"]);
    expect(reconcileOrder(["a", "a"], { keep: () => true, incoming: [], replaceable: () => true })).toEqual(["a"]);
  });
});

describe("parseLayout", () => {
  it("migrates a v1 tree into the same reading order", () => {
    const root = { type: "split", direction: "vertical", ratio: 0.5,
      first: { type: "split", direction: "horizontal", ratio: 0.5, first: { type: "leaf", leafId: "a" }, second: { type: "leaf", leafId: "b" } },
      second: { type: "split", direction: "horizontal", ratio: 0.5, first: { type: "leaf", leafId: "c" }, second: { type: "leaf", leafId: "a" } } };
    expect(parseLayout(JSON.stringify({ version: 1, root, expandedLeafId: "b", pinnedSessionIds: ["c", "x"], dismissedSessionIds: ["z", "z", 7] }))).toEqual({
      version: 2, order: ["a", "b", "c"], expandedId: "b", pinnedSessionIds: ["c"], dismissedSessionIds: ["z"], stageLimit: null,
    });
  });

  it("round-trips v2 and rejects bad values", () => {
    const layout = { version: 2, order: ["a", "b"], pinnedSessionIds: ["b"], dismissedSessionIds: [], expandedId: "a", stageLimit: 4 };
    expect(parseLayout(JSON.stringify(layout))).toEqual(layout);
    expect(parseLayout(JSON.stringify({ ...layout, stageLimit: 3, expandedId: "missing", order: ["a", 1, "a", ""] }))).toMatchObject({ order: ["a"], stageLimit: null, expandedId: null, pinnedSessionIds: [] });
    expect(parseLayout(JSON.stringify({ version: 9, order: ["a"] })).order).toEqual([]);
    expect(parseLayout("{nope").order).toEqual([]);
    expect(parseLayout(JSON.stringify({ version: 2, order: Array.from({ length: 100 }, (_, i) => `s${i}`) })).order).toHaveLength(64);
  });
});
