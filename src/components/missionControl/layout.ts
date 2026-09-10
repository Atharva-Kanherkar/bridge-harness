import { leafIds, removeLeaf, splitLeaf, type PaneNode, type SplitDirection } from "../../terminal/layout";

export const MISSION_LAYOUT_KEY = "bridge.mission-control.layout";
export type MissionLayout = { version: 1; root: PaneNode | null; expandedLeafId: string | null; pinnedSessionIds: string[] };
export type DropEdge = "left" | "right" | "top" | "bottom";

// assumed screen aspect for picking which way to cut a tile. unmeasured on
// purpose: the tree stays deterministic in tests and across window resizes.
const ASPECT = 1.6;

type LeafBox = { leafId: string; width: number; height: number };

// Keep transcripts and composers usable, even in deeply nested saved splits.
export function minimumSize(node: PaneNode): { width: number; height: number } {
  if (node.type === "leaf") return { width: 420, height: 360 };
  const first = minimumSize(node.first), second = minimumSize(node.second);
  return node.direction === "horizontal"
    ? { width: first.width + second.width + 6, height: Math.max(first.height, second.height) }
    : { width: Math.max(first.width, second.width), height: first.height + second.height + 6 };
}

export function leafBoxes(node: PaneNode, width = ASPECT, height = 1): LeafBox[] {
  if (node.type === "leaf") return [{ leafId: node.leafId, width, height }];
  return node.direction === "horizontal"
    ? [...leafBoxes(node.first, width * node.ratio, height), ...leafBoxes(node.second, width * (1 - node.ratio), height)]
    : [...leafBoxes(node.first, width, height * node.ratio), ...leafBoxes(node.second, width, height * (1 - node.ratio))];
}

// split the largest tile along its longer side so the grid stays balanced.
export function insertLeaf(root: PaneNode | null, leafId: string): PaneNode {
  if (!root) return { type: "leaf", leafId };
  if (leafIds(root).includes(leafId)) return root;
  const largest = leafBoxes(root).reduce((best, box) => box.width * box.height > best.width * best.height ? box : best);
  const direction: SplitDirection = largest.width >= largest.height ? "horizontal" : "vertical";
  return splitLeaf(root, largest.leafId, leafId, direction);
}

// drop leaves that are no longer wanted, then insert the new ones in order.
export function reconcileLeaves(root: PaneNode | null, ids: readonly string[]): PaneNode | null {
  const wanted = new Set(ids);
  let next: PaneNode | null = root;
  if (next) for (const id of leafIds(next)) if (!wanted.has(id) && next) next = removeLeaf(next, id);
  for (const id of ids) if (!next || !leafIds(next).includes(id)) next = insertLeaf(next, id);
  return next;
}

export function moveLeaf(root: PaneNode, id: string, target: string, direction: SplitDirection, before: boolean): PaneNode {
  if (id === target || !leafIds(root).includes(target)) return root;
  const stripped = leafIds(root).includes(id) ? removeLeaf(root, id) : root;
  return stripped ? splitLeaf(stripped, target, id, direction, before) : root;
}

export function dropEdge(rect: { left: number; top: number; width: number; height: number }, clientX: number, clientY: number): DropEdge {
  const x = rect.width ? (clientX - rect.left) / rect.width : 0.5, y = rect.height ? (clientY - rect.top) / rect.height : 0.5;
  const nearX = Math.min(x, 1 - x), nearY = Math.min(y, 1 - y);
  return nearX < nearY ? (x < 0.5 ? "left" : "right") : (y < 0.5 ? "top" : "bottom");
}

// persisted input is untrusted: bounded depth, deduped leaves, clamped ratios.
export function parseLayout(raw: string | null): MissionLayout {
  const empty: MissionLayout = { version: 1, root: null, expandedLeafId: null, pinnedSessionIds: [] };
  if (!raw) return empty;
  let value: unknown;
  try { value = JSON.parse(raw); } catch { return empty; }
  if (!value || typeof value !== "object" || (value as { version?: unknown }).version !== 1) return empty;
  const seen = new Set<string>();
  const parseNode = (node: unknown, depth = 0): PaneNode | null => {
    if (!node || typeof node !== "object" || depth > 32) return null;
    const n = node as Record<string, unknown>;
    if (n.type === "leaf") {
      if (typeof n.leafId !== "string" || seen.has(n.leafId)) return null;
      seen.add(n.leafId); return { type: "leaf", leafId: n.leafId };
    }
    if (n.type !== "split" || (n.direction !== "horizontal" && n.direction !== "vertical")) return null;
    const first = parseNode(n.first, depth + 1), second = parseNode(n.second, depth + 1);
    if (!first) return second;
    if (!second) return first;
    const ratio = typeof n.ratio === "number" && Number.isFinite(n.ratio) ? Math.min(0.9, Math.max(0.1, n.ratio)) : 0.5;
    return { type: "split", direction: n.direction, first, second, ratio };
  };
  const stored = value as { root?: unknown; expandedLeafId?: unknown; pinnedSessionIds?: unknown };
  const root = parseNode(stored.root);
  const expanded = typeof stored.expandedLeafId === "string" && root && leafIds(root).includes(stored.expandedLeafId) ? stored.expandedLeafId : null;
  const pinnedSessionIds = Array.isArray(stored.pinnedSessionIds)
    ? [...new Set(stored.pinnedSessionIds.filter((id): id is string => typeof id === "string" && seen.has(id)))] : [];
  return { version: 1, root, expandedLeafId: expanded, pinnedSessionIds };
}

export function readLayout(): MissionLayout {
  try { return parseLayout(globalThis.localStorage?.getItem(MISSION_LAYOUT_KEY) ?? null); } catch { return { version: 1, root: null, expandedLeafId: null, pinnedSessionIds: [] }; }
}

export function writeLayout(layout: MissionLayout) {
  try { globalThis.localStorage?.setItem(MISSION_LAYOUT_KEY, JSON.stringify(layout)); } catch { /* storage unavailable */ }
}
