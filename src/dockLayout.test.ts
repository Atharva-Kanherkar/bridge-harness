import { describe, expect, it } from "vitest";
import {
  DEFAULT_DOCK_WIDTH,
  DOCK_SHEET_THRESHOLD,
  MAX_DOCK_WIDTH,
  MIN_CONVERSATION_WIDTH,
  MIN_DOCK_WIDTH,
  defaultDockState,
  dockReducer,
  readDockState,
  writeDockState,
  type DockState,
} from "./dockLayout";

// Contract: testing/feat-dock-shell.md §1 and §2.

function memoryStorage(initial: Record<string, string> = {}) {
  const map = new Map(Object.entries(initial));
  return {
    map,
    getItem: (key: string) => map.get(key) ?? null,
    setItem: (key: string, value: string) => void map.set(key, value),
  };
}

describe("dockReducer", () => {
  it("starts collapsed with the documented defaults", () => {
    expect(defaultDockState()).toEqual({
      open: false,
      width: DEFAULT_DOCK_WIDTH,
      pane: "changes",
      expanded: false,
      visited: [],
    });
    expect(DOCK_SHEET_THRESHOLD).toBe(MIN_DOCK_WIDTH + MIN_CONVERSATION_WIDTH);
  });

  it("toggle opens a collapsed dock and collapses an open one", () => {
    const opened = dockReducer(defaultDockState(), { type: "toggle" });
    expect(opened.open).toBe(true);
    expect(dockReducer(opened, { type: "toggle" }).open).toBe(false);
  });

  it("toggle opening visits the active pane so it has a body to show", () => {
    const opened = dockReducer(defaultDockState(), { type: "toggle" });
    expect(opened.visited).toEqual(["changes"]);
  });

  it("collapsing resets expand", () => {
    const state: DockState = { ...defaultDockState(), open: true, expanded: true };
    const collapsed = dockReducer(state, { type: "toggle" });
    expect(collapsed.open).toBe(false);
    expect(collapsed.expanded).toBe(false);
  });

  it("open-pane activates the pane and opens the dock in one action", () => {
    const state = dockReducer(defaultDockState(), { type: "open-pane", pane: "terminal" });
    expect(state.open).toBe(true);
    expect(state.pane).toBe("terminal");
  });

  it("open-pane records a visit exactly once", () => {
    const once = dockReducer(defaultDockState(), { type: "open-pane", pane: "code" });
    const twice = dockReducer(once, { type: "open-pane", pane: "code" });
    expect(twice.visited).toEqual(["code"]);
  });

  it("visits accumulate across pane switches", () => {
    let state = defaultDockState();
    for (const pane of ["changes", "code", "changes"] as const) {
      state = dockReducer(state, { type: "open-pane", pane });
    }
    expect(state.visited).toEqual(["changes", "code"]);
  });

  it("set-width clamps to the dock minimum", () => {
    const state = dockReducer(defaultDockState(), { type: "set-width", width: 100, available: 1200 });
    expect(state.width).toBe(MIN_DOCK_WIDTH);
  });

  it("set-width clamps to the dock maximum", () => {
    const state = dockReducer(defaultDockState(), { type: "set-width", width: 2000, available: 4000 });
    expect(state.width).toBe(MAX_DOCK_WIDTH);
  });

  it("set-width never starves the conversation", () => {
    const state = dockReducer(defaultDockState(), { type: "set-width", width: 700, available: 1000 });
    expect(state.width).toBe(1000 - MIN_CONVERSATION_WIDTH);
  });

  it("a viewport too narrow for both minimums pins the dock minimum", () => {
    const state = dockReducer(defaultDockState(), { type: "set-width", width: 999, available: 600 });
    expect(state.width).toBe(MIN_DOCK_WIDTH);
  });

  it("toggle-expanded flips only while open", () => {
    expect(dockReducer(defaultDockState(), { type: "toggle-expanded" }).expanded).toBe(false);
    const open = dockReducer(defaultDockState(), { type: "toggle" });
    const expanded = dockReducer(open, { type: "toggle-expanded" });
    expect(expanded.expanded).toBe(true);
    expect(dockReducer(expanded, { type: "toggle-expanded" }).expanded).toBe(false);
  });

  it("never mutates its input", () => {
    const state = defaultDockState();
    const snapshot = structuredClone(state);
    const next = dockReducer(state, { type: "open-pane", pane: "code" });
    expect(next).not.toBe(state);
    expect(state).toEqual(snapshot);
  });
});

describe("dock persistence", () => {
  it("round-trips the durable fields", () => {
    const storage = memoryStorage();
    const state: DockState = { open: true, width: 512, pane: "terminal", expanded: true, visited: ["changes", "terminal"] };
    writeDockState("ws-1", state, storage);
    const restored = readDockState("ws-1", storage);
    expect(restored.open).toBe(true);
    expect(restored.width).toBe(512);
    expect(restored.pane).toBe("terminal");
    expect(restored.expanded).toBe(true);
  });

  it("does not persist visited", () => {
    const storage = memoryStorage();
    writeDockState("ws-1", { ...defaultDockState(), visited: ["changes", "code", "terminal"] }, storage);
    expect(readDockState("ws-1", storage).visited).toEqual([]);
  });

  it("seeds the active pane as visited when restoring an open dock", () => {
    const storage = memoryStorage();
    writeDockState("ws-1", { ...defaultDockState(), open: true, pane: "code" }, storage);
    expect(readDockState("ws-1", storage).visited).toEqual(["code"]);
  });

  it("falls back to defaults on corrupt JSON", () => {
    const storage = memoryStorage({ "bridge.dock.v1.ws-1": "{not json" });
    expect(readDockState("ws-1", storage)).toEqual(defaultDockState());
  });

  it("falls back to defaults on wrong field types", () => {
    const storage = memoryStorage({
      "bridge.dock.v1.ws-1": JSON.stringify({ open: "yes", width: 500, pane: "changes", expanded: false }),
    });
    expect(readDockState("ws-1", storage)).toEqual(defaultDockState());
  });

  it("falls back to the default pane on an unknown pane id", () => {
    const storage = memoryStorage({
      "bridge.dock.v1.ws-1": JSON.stringify({ open: false, width: 500, pane: "tasks", expanded: false }),
    });
    expect(readDockState("ws-1", storage).pane).toBe("changes");
  });

  it("clamps an out-of-range width on read", () => {
    const wide = memoryStorage({
      "bridge.dock.v1.ws-1": JSON.stringify({ open: false, width: 5000, pane: "changes", expanded: false }),
    });
    expect(readDockState("ws-1", wide).width).toBe(MAX_DOCK_WIDTH);
    const thin = memoryStorage({
      "bridge.dock.v1.ws-1": JSON.stringify({ open: false, width: 10, pane: "changes", expanded: false }),
    });
    expect(readDockState("ws-1", thin).width).toBe(MIN_DOCK_WIDTH);
  });

  it("isolates keys", () => {
    const storage = memoryStorage();
    writeDockState("ws-1", { ...defaultDockState(), width: 500 }, storage);
    writeDockState("ws-2", { ...defaultDockState(), width: 600 }, storage);
    expect(readDockState("ws-1", storage).width).toBe(500);
    expect(readDockState("ws-2", storage).width).toBe(600);
  });

  it("survives a throwing storage", () => {
    const throwing = {
      getItem: () => {
        throw new Error("denied");
      },
      setItem: () => {
        throw new Error("denied");
      },
    };
    expect(readDockState("ws-1", throwing)).toEqual(defaultDockState());
    expect(() => writeDockState("ws-1", defaultDockState(), throwing)).not.toThrow();
  });
});
