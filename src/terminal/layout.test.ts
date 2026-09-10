import { describe, expect, it } from "vitest";
import { addTab, closeLeaf, emptyLayout, insertSplit, leafIds, resizeNode, restoreLayout } from "./layout";

describe("Orca split-tree actions", () => {
  it("nests splits and collapses redundant parents after close", () => {
    let layout = addTab(emptyLayout(), "a", "Shell");
    layout = insertSplit(layout, "a", "b", "horizontal");
    layout = insertSplit(layout, "b", "c", "vertical");
    expect(layout.tabs[0].root).toMatchObject({ type: "split", direction: "horizontal", second: { type: "split", direction: "vertical" } });
    layout = closeLeaf(layout, "b");
    expect(leafIds(layout.tabs[0].root)).toEqual(["a", "c"]);
    layout = closeLeaf(layout, "c");
    expect(layout.tabs[0].root).toEqual({ type: "leaf", leafId: "a" });
    expect(layout.activeLeafId).toBe("a");
    expect(closeLeaf(layout, "a").tabs).toEqual([]);
  });
  it("moves existing leaves across tabs without changing identities", () => {
    let layout = addTab(addTab(emptyLayout(), "a", "A"), "b", "B");
    layout = insertSplit(layout, "a", "c", "vertical");
    layout = insertSplit(layout, "b", "a", "horizontal", true);
    expect(layout.tabs.flatMap(t => leafIds(t.root))).toEqual(["c", "a", "b"]);
    const detached = addTab(closeLeaf(layout, "a"), "a", "A");
    expect(new Set(detached.tabs.map(t => t.id)).size).toBe(3);
    expect(detached.tabs.flatMap(t => leafIds(t.root)).sort()).toEqual(["a", "b", "c"]);
  });
  it("repairs corrupt, duplicate and missing references without losing valid terminals", () => {
    const layout = restoreLayout({ version: 1, activeLeafId: "gone", tabs: [{ id: "t", root: { type: "split", direction: "horizontal", ratio: 99, first: { type: "leaf", leafId: "a" }, second: { type: "leaf", leafId: "a" } } }, { id: "t", root: { type: "leaf", leafId: "b" } }] }, [{ terminalId: "a", title: "A" }, { terminalId: "b", title: "B" }]);
    expect(layout.tabs.flatMap(t => leafIds(t.root))).toEqual(["a", "b"]);
    expect(layout.activeLeafId).toBe("a");
    expect(layout.expandedLeafId).toBeNull();
  });
  it("retains nested ratios and clamps unusable dimensions", () => {
    const layout = insertSplit(addTab(emptyLayout(), "a", "A"), "a", "b", "vertical");
    expect(resizeNode(layout.tabs[0].root, "", 3)).toMatchObject({ ratio: .9 });
    const resized = { ...layout, tabs: [{ ...layout.tabs[0], root: resizeNode(layout.tabs[0].root, "", .35) }] };
    expect(restoreLayout(resized, [{ terminalId: "a", title: "A" }, { terminalId: "b", title: "B" }])).toEqual(resized);
  });
});
