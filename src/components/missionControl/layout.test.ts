import { describe, expect, it } from "vitest";
import { leafIds, type PaneNode } from "../../terminal/layout";
import { arrangeLeaves, groupedOrder, insertLeaf, leafBoxes, parseLayout, reconcileLeaves } from "./layout";

const leaf = (leafId: string): PaneNode => ({ type: "leaf", leafId });
const rows = (node: PaneNode | null): string[][] => {
  if (!node) return [];
  if (node.type === "leaf" || node.direction === "horizontal") return [leafIds(node)];
  return [...rows(node.first), ...rows(node.second)];
};

describe("arrangeLeaves", () => {
  it("builds balanced rows in the order given", () => {
    expect(arrangeLeaves([])).toBeNull();
    expect(arrangeLeaves(["a"])).toEqual(leaf("a"));
    expect(rows(arrangeLeaves(["a", "b"]))).toEqual([["a", "b"]]);
    expect(rows(arrangeLeaves(["a", "b", "c"]))).toEqual([["a", "b"], ["c"]]);
    expect(rows(arrangeLeaves(["a", "b", "c", "d"]))).toEqual([["a", "b"], ["c", "d"]]);
    expect(rows(arrangeLeaves(["a", "b", "c", "d", "e"]))).toEqual([["a", "b", "c"], ["d", "e"]]);
    expect(rows(arrangeLeaves(["a", "a", "b"]))).toEqual([["a", "b"]]);
  });

  it("gives every tile in a row an equal share", () => {
    const boxes = leafBoxes(arrangeLeaves(["a", "b", "c"])!, 3, 1);
    const widths = Object.fromEntries(boxes.map(box => [box.leafId, Number(box.width.toFixed(3))]));
    expect(widths).toEqual({ a: 1.5, b: 1.5, c: 3 });
    const five = leafBoxes(arrangeLeaves(["a", "b", "c", "d", "e"])!, 3, 1);
    expect(five.slice(0, 3).map(box => Number(box.width.toFixed(3)))).toEqual([1, 1, 1]);
  });

  it("survives a round trip through the saved-layout parser", () => {
    const root = arrangeLeaves(["a", "b", "c", "d", "e", "f", "g"]);
    expect(parseLayout(JSON.stringify({ version: 1, root })).root).toEqual(root);
  });
});

describe("insertLeaf near a project", () => {
  const board: PaneNode = { type: "split", direction: "horizontal", ratio: 0.7, first: leaf("big"), second: leaf("mine") };

  it("cuts the new tile from the largest neighbour instead of the largest tile", () => {
    const next = insertLeaf(board, "new", ["mine"]);
    expect(next.type === "split" && leafIds(next.second).sort()).toEqual(["mine", "new"]);
    expect(next.type === "split" && next.first).toEqual(leaf("big"));
  });

  it("falls back to the largest tile when no neighbour is on the board", () => {
    const next = insertLeaf(board, "new", ["absent"]);
    expect(next.type === "split" && leafIds(next.first).sort()).toEqual(["big", "new"]);
  });

  it("reconciles new leaves beside their group", () => {
    const groups: Record<string, string> = { big: "x", mine: "y", new: "y" };
    const next = reconcileLeaves(board, ["big", "mine", "new"], id => groups[id] ?? null)!;
    expect(next.type === "split" && leafIds(next.second).sort()).toEqual(["mine", "new"]);
  });
});

describe("groupedOrder", () => {
  it("keeps groups contiguous, largest first, ties and loose tiles in order", () => {
    const groups: Record<string, string | null> = { k: "kairo", b1: "bridge", p: "portfolio", b2: "bridge", x: null };
    expect(groupedOrder(["k", "b1", "x", "p", "b2"], id => groups[id] ?? null)).toEqual(["b1", "b2", "k", "p", "x"]);
    expect(rows(arrangeLeaves(groupedOrder(["k", "b1", "p", "b2"], id => groups[id] ?? null)))).toEqual([["b1", "b2"], ["k", "p"]]);
  });
});
