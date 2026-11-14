// Which dock panes a chat shows: the ones it uses, not all ten.
//
// A pane earns its tab by being core to every chat (when it can work here),
// by having something to say (a count or an alert), by being open or opened
// already, or by the person adding it to this chat. Everything else waits behind
// the dock's + menu, one click away.

import { useCallback, useSyncExternalStore } from "react";
import { DOCK_PANES, type DockPaneId } from "./dockLayout";

export const CORE_DOCK_PANES: readonly DockPaneId[] = ["work", "changes", "terminal", "browser"];
export const DOCK_ADDED_KEY = "bridge.dock.added.v1";
/** Chats whose added panes are remembered. Older ones are forgotten first. */
export const MAX_REMEMBERED_CHATS = 200;

type PaneLike = { id: DockPaneId; available: boolean; badge?: number; alert?: boolean };

/** Split the strip, keeping the order it already has. */
export function splitDockPanes<T extends PaneLike>(panes: readonly T[], { active, added, visited = [] }: { active?: DockPaneId | null; added: readonly DockPaneId[]; visited?: readonly DockPaneId[] }): { shown: T[]; hidden: T[] } {
  const shown: T[] = [];
  const hidden: T[] = [];
  for (const pane of panes) {
    // A pane opened in this dock keeps its content mounted, so its tab stays
    // too: hiding it would leave a live terminal with no way back to it.
    const earned = pane.id === active
      || visited.includes(pane.id)
      || added.includes(pane.id)
      || (pane.available && (CORE_DOCK_PANES.includes(pane.id) || !!pane.badge || !!pane.alert));
    (earned ? shown : hidden).push(pane);
  }
  return { shown, hidden };
}

type Stored = Record<string, { panes: DockPaneId[]; at: number }>;

const EMPTY: readonly DockPaneId[] = [];
const listeners = new Set<() => void>();
let cache: Stored | null = null;

function isPane(value: unknown): value is DockPaneId {
  return typeof value === "string" && (DOCK_PANES as readonly string[]).includes(value);
}

export function parseAddedPanes(raw: string | null): Stored {
  if (!raw) return {};
  let stored: unknown;
  try {
    stored = JSON.parse(raw);
  } catch {
    return {};
  }
  if (!stored || typeof stored !== "object" || Array.isArray(stored)) return {};
  const clean: Stored = {};
  for (const [sessionId, entry] of Object.entries(stored as Record<string, unknown>)) {
    if (!entry || typeof entry !== "object") continue;
    const { panes, at } = entry as { panes?: unknown; at?: unknown };
    if (!Array.isArray(panes) || typeof at !== "number") continue;
    const valid = [...new Set(panes.filter(isPane))];
    if (valid.length) clean[sessionId] = { panes: valid, at };
  }
  return clean;
}

function read(): Stored {
  if (cache === null) {
    let raw: string | null = null;
    try {
      raw = typeof localStorage === "undefined" ? null : localStorage.getItem(DOCK_ADDED_KEY);
    } catch {
      raw = null;
    }
    cache = parseAddedPanes(raw);
  }
  return cache;
}

/** Keep the most recently touched chats, up to the cap. */
export function pruneAddedPanes(stored: Stored, cap = MAX_REMEMBERED_CHATS): Stored {
  const entries = Object.entries(stored);
  if (entries.length <= cap) return stored;
  return Object.fromEntries(entries.sort(([, a], [, b]) => b.at - a.at).slice(0, cap));
}

export function readAddedPanes(sessionId: string | null | undefined): readonly DockPaneId[] {
  if (!sessionId) return EMPTY;
  return read()[sessionId]?.panes ?? EMPTY;
}

export function addPaneToChat(sessionId: string, pane: DockPaneId, now = Date.now()): readonly DockPaneId[] {
  const stored = read();
  const current = stored[sessionId]?.panes ?? [];
  const next = pruneAddedPanes({ ...stored, [sessionId]: { panes: current.includes(pane) ? current : [...current, pane], at: now } });
  cache = next;
  try {
    localStorage.setItem(DOCK_ADDED_KEY, JSON.stringify(next));
  } catch {
    // Storage full or blocked: the pane stays for this session.
  }
  for (const listener of listeners) listener();
  return next[sessionId].panes;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

/** The panes added to this chat, and a way to add one. */
export function useAddedPanes(sessionId: string | null | undefined): [readonly DockPaneId[], (pane: DockPaneId) => void] {
  const added = useSyncExternalStore(subscribe, () => readAddedPanes(sessionId), () => readAddedPanes(sessionId));
  const add = useCallback((pane: DockPaneId) => { if (sessionId) addPaneToChat(sessionId, pane); }, [sessionId]);
  return [added, add];
}

/** Tests only: forget the cached read. */
export function resetAddedPanesCache() {
  cache = null;
}
