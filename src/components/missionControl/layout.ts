export const MISSION_LAYOUT_KEY = "bridge.mission-control.layout";
export const STAGE_LIMITS = [1, 2, 4, 6] as const;
export type StageLimit = (typeof STAGE_LIMITS)[number];
export type MissionLayout = {
  version: 2;
  /** The board's chats in your order; the stage takes the first ones that fit. */
  order: string[];
  pinnedSessionIds: string[];
  dismissedSessionIds: string[];
  expandedId: string | null;
  /** At most this many tiles on the stage; null fits as many as the board holds. */
  stageLimit: StageLimit | null;
};
export type DropPlace = "before" | "after";
export type QueuePlacement = "rail" | "strip";

export const TILE_MIN_WIDTH = 360;
export const TILE_MIN_HEIGHT = 300;
export const TILE_GAP = 8;
export const QUEUE_RAIL_WIDTH = 256;
export const QUEUE_STRIP_HEIGHT = 36;
export const QUEUE_RAIL_MIN_BOARD = 900;
const TARGET_ASPECT = 1.15;
const EMPTY_CELL_COST = 0.35;
const MAX_ORDER = 64;

export const emptyLayout = (): MissionLayout => ({ version: 2, order: [], pinnedSessionIds: [], dismissedSessionIds: [], expandedId: null, stageLimit: null });

export type GridFit = { cols: number; rows: number; shown: number };

// how many equal tiles the measured space holds, and in what shape. never asks
// for more room than it was given: one tile always shows, sized to the box.
export function fitGrid(width: number, height: number, n: number, limit: number | null = null): GridFit {
  const cap = limit ?? Infinity;
  if (n <= 0) return { cols: 0, rows: 0, shown: 0 };
  if (!(width > 0) || !(height > 0)) {
    const shown = Math.min(n, cap);
    const cols = Math.ceil(Math.sqrt(shown));
    return { cols, rows: Math.ceil(shown / cols), shown };
  }
  const maxCols = Math.max(1, Math.floor((width + TILE_GAP) / (TILE_MIN_WIDTH + TILE_GAP)));
  const maxRows = Math.max(1, Math.floor((height + TILE_GAP) / (TILE_MIN_HEIGHT + TILE_GAP)));
  const shown = Math.min(n, maxCols * maxRows, cap);
  let best: GridFit & { score: number } = { cols: maxCols, rows: maxRows, shown, score: Infinity };
  for (let cols = 1; cols <= Math.min(maxCols, shown); cols += 1) {
    const rows = Math.ceil(shown / cols);
    if (rows > maxRows) continue;
    const tileWidth = (width - (cols - 1) * TILE_GAP) / cols;
    const tileHeight = (height - (rows - 1) * TILE_GAP) / rows;
    const score = Math.abs(Math.log(tileWidth / tileHeight / TARGET_ASPECT)) + EMPTY_CELL_COST * (cols * rows - shown);
    if (score < best.score) best = { cols, rows, shown, score };
  }
  return { cols: best.cols, rows: best.rows, shown: best.shown };
}

export type BoardPlan = GridFit & { queue: QueuePlacement | null };

// fit everything if it fits; otherwise make room for the queue and fit again.
export function planBoard(width: number, height: number, n: number, limit: number | null = null): BoardPlan {
  const full = fitGrid(width, height, n, limit);
  if (full.shown >= n) return { ...full, queue: null };
  if (!(width > 0) || !(height > 0)) return { ...full, queue: "strip" };
  const queue: QueuePlacement = width >= QUEUE_RAIL_MIN_BOARD ? "rail" : "strip";
  const fit = queue === "rail"
    ? fitGrid(width - QUEUE_RAIL_WIDTH - TILE_GAP, height, n, limit)
    : fitGrid(width, height - QUEUE_STRIP_HEIGHT - TILE_GAP, n, limit);
  return { ...fit, queue };
}

// chats waiting on you lead, keeping their relative order; the rest follow yours.
export function displayOrder(order: readonly string[], needsYou: (id: string) => boolean): string[] {
  return [...order.filter(needsYou), ...order.filter(id => !needsYou(id))];
}

// moves one id next to another; nothing else changes position.
export function moveInOrder(order: readonly string[], id: string, target: string, place: DropPlace): string[] {
  if (id === target) return [...order];
  const rest = order.filter(other => other !== id);
  const at = rest.indexOf(target);
  if (at < 0) return order.includes(id) ? [...order] : [...rest, id];
  rest.splice(place === "before" ? at : at + 1, 0, id);
  return rest;
}

export function moveToFront(order: readonly string[], id: string): string[] {
  return [id, ...order.filter(other => other !== id)];
}

export function moveToEnd(order: readonly string[], id: string): string[] {
  return [...order.filter(other => other !== id), id];
}

// existing chats never move. a newly active chat takes the slot of the first
// finished, unpinned one, so a turn ending never reflows the board and finished
// chats do not pile up; with no such slot it joins the end.
export function reconcileOrder(order: readonly string[], options: { keep: (id: string) => boolean; incoming: readonly string[]; replaceable: (id: string) => boolean }): string[] {
  const next = [...new Set(order)].filter(options.keep);
  const placed = new Set(next);
  for (const id of options.incoming) {
    if (placed.has(id)) continue;
    const slot = next.findIndex(options.replaceable);
    if (slot >= 0) { placed.delete(next[slot]); next[slot] = id; } else next.push(id);
    placed.add(id);
  }
  return next;
}

const strings = (value: unknown): string[] => Array.isArray(value) ? [...new Set(value.filter((id): id is string => typeof id === "string" && id.length > 0))] : [];

// persisted input is untrusted: string ids only, deduped, bounded.
export function parseLayout(raw: string | null): MissionLayout {
  if (!raw) return emptyLayout();
  let value: unknown;
  try { value = JSON.parse(raw); } catch { return emptyLayout(); }
  if (!value || typeof value !== "object") return emptyLayout();
  const stored = value as Record<string, unknown>;
  let order: string[];
  let expanded: unknown;
  if (stored.version === 1) {
    order = strings(parseV1Root(stored.root));
    expanded = stored.expandedLeafId;
  } else if (stored.version === 2) {
    order = strings(stored.order);
    expanded = stored.expandedId;
  } else return emptyLayout();
  order = order.slice(0, MAX_ORDER);
  const onBoard = new Set(order);
  const limit = STAGE_LIMITS.find(candidate => candidate === stored.stageLimit) ?? null;
  return {
    version: 2,
    order,
    pinnedSessionIds: strings(stored.pinnedSessionIds).filter(id => onBoard.has(id)),
    // closed chats are deliberately off the board, so `order` cannot gate them.
    dismissedSessionIds: strings(stored.dismissedSessionIds).slice(0, MAX_ORDER * 4),
    expandedId: typeof expanded === "string" && onBoard.has(expanded) ? expanded : null,
    stageLimit: limit,
  };
}

// a v1 split tree read in its own order: left to right, top to bottom.
function parseV1Root(node: unknown, depth = 0): string[] {
  if (!node || typeof node !== "object" || depth > 32) return [];
  const n = node as Record<string, unknown>;
  if (n.type === "leaf") return typeof n.leafId === "string" ? [n.leafId] : [];
  if (n.type !== "split") return [];
  return [...parseV1Root(n.first, depth + 1), ...parseV1Root(n.second, depth + 1)];
}

export function readLayout(): MissionLayout {
  try { return parseLayout(globalThis.localStorage?.getItem(MISSION_LAYOUT_KEY) ?? null); } catch { return emptyLayout(); }
}

export function writeLayout(layout: MissionLayout) {
  try { globalThis.localStorage?.setItem(MISSION_LAYOUT_KEY, JSON.stringify(layout)); } catch { /* storage unavailable */ }
}
