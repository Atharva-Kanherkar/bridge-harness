import { useCallback, useEffect, useState } from "react";

// Layout state for the right-hand dock: a trailing pane beside the
// conversation that hosts Changes, Code, and Terminal without covering the
// chat. This module is the pure half — a reducer over a small record plus a
// persistence round-trip — so the split, the divider, and restart restore can
// be asserted without mounting anything.

export type DockPaneId = "changes" | "code" | "terminal" | "transcript";

export type DockState = {
  open: boolean;
  /** Dock width in px. Clamped: the conversation keeps its minimum first. */
  width: number;
  pane: DockPaneId;
  /** The active pane fills the window; collapsing always clears it. */
  expanded: boolean;
  /** Panes that have been activated once — they mount lazily and then stay
   * mounted, so switching away never tears one down. Runtime-only. */
  visited: DockPaneId[];
};

export type DockAction =
  | { type: "toggle" }
  | { type: "open-pane"; pane: DockPaneId }
  | { type: "set-width"; width: number; available: number }
  | { type: "toggle-expanded" };

export const DOCK_PANES: readonly DockPaneId[] = ["changes", "code", "terminal", "transcript"];

export const MIN_DOCK_WIDTH = 320;
export const MAX_DOCK_WIDTH = 760;
export const MIN_CONVERSATION_WIDTH = 440;
export const DEFAULT_DOCK_WIDTH = 440;
/** Below this section width a split starves one side or the other, so an
 * open dock renders as an overlay sheet instead. */
export const DOCK_SHEET_THRESHOLD = MIN_DOCK_WIDTH + MIN_CONVERSATION_WIDTH;

const DEFAULT_PANE: DockPaneId = "changes";

export function isDockPaneId(value: unknown): value is DockPaneId {
  return DOCK_PANES.includes(value as DockPaneId);
}

export function defaultDockState(): DockState {
  return { open: false, width: DEFAULT_DOCK_WIDTH, pane: DEFAULT_PANE, expanded: false, visited: [] };
}

/** The conversation's minimum wins over the dock's maximum; the dock's
 * minimum wins over everything, because sheet mode takes over below the
 * threshold rather than letting the pane shrink into uselessness. */
export function clampDockWidth(width: number, available: number): number {
  const max = Math.min(MAX_DOCK_WIDTH, available - MIN_CONVERSATION_WIDTH);
  return Math.max(MIN_DOCK_WIDTH, Math.min(max, width));
}

export function dockReducer(state: DockState, action: DockAction): DockState {
  switch (action.type) {
    case "toggle":
      if (state.open) return { ...state, open: false, expanded: false };
      return {
        ...state,
        open: true,
        visited: state.visited.includes(state.pane) ? state.visited : [...state.visited, state.pane],
      };
    case "open-pane": {
      const visited = state.visited.includes(action.pane) ? state.visited : [...state.visited, action.pane];
      return { ...state, open: true, pane: action.pane, visited };
    }
    case "set-width":
      return { ...state, width: clampDockWidth(action.width, action.available) };
    case "toggle-expanded":
      return state.open ? { ...state, expanded: !state.expanded } : state;
  }
}

// ── Persistence ──────────────────────────────────────────────────────────────
// One record per workspace (direct chats key by session id), versioned so a
// future shape change can migrate rather than misread. `visited` is not
// stored: restoring every previously opened pane would mount all of them at
// startup, so only the active pane of an open dock comes back mounted.

const STORAGE_PREFIX = "bridge.dock.v1.";

export function readDockState(key: string, storage: Pick<Storage, "getItem"> = localStorage): DockState {
  let raw: string | null = null;
  try {
    raw = storage.getItem(STORAGE_PREFIX + key);
  } catch {
    return defaultDockState();
  }
  if (!raw) return defaultDockState();

  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return defaultDockState();
  }
  if (typeof parsed !== "object" || parsed === null) return defaultDockState();

  const record = parsed as Record<string, unknown>;
  if (typeof record.open !== "boolean" || typeof record.expanded !== "boolean" || typeof record.width !== "number" || !Number.isFinite(record.width)) {
    return defaultDockState();
  }
  const pane = isDockPaneId(record.pane) ? record.pane : DEFAULT_PANE;
  const width = Math.max(MIN_DOCK_WIDTH, Math.min(MAX_DOCK_WIDTH, record.width));
  return {
    open: record.open,
    width,
    pane,
    expanded: record.expanded,
    visited: record.open ? [pane] : [],
  };
}

export function writeDockState(key: string, state: DockState, storage: Pick<Storage, "setItem"> = localStorage): void {
  const record = { open: state.open, width: state.width, pane: state.pane, expanded: state.expanded };
  try {
    storage.setItem(STORAGE_PREFIX + key, JSON.stringify(record));
  } catch {
    // A read-only storage should never stop the dock from working.
  }
}

/**
 * Dock state for one workspace (or one direct chat), restored synchronously so
 * the first paint already has the remembered layout, and written through on
 * every action. Changing key swaps to that key's stored state — two
 * workspaces are two docks.
 */
export function useDockLayout(key: string | undefined): [DockState, (action: DockAction) => void] {
  const [state, setState] = useState<DockState>(() => (key ? readDockState(key) : defaultDockState()));

  useEffect(() => {
    setState(key ? readDockState(key) : defaultDockState());
  }, [key]);

  const dispatch = useCallback((action: DockAction) => {
    setState(previous => {
      const next = dockReducer(previous, action);
      if (key) writeDockState(key, next);
      return next;
    });
  }, [key]);

  return [state, dispatch];
}
