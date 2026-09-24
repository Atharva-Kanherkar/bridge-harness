import { leafIds, removeLeaf, splitLeaf, type PaneNode, type SplitDirection } from "../../terminal/layout";

export const MISSION_LAYOUT_KEY = "bridge.mission-control.layout";
export type MissionLayout = { version: 1; root: PaneNode | null; expandedLeafId: string | null; pinnedSessionIds: string[]; dismissedSessionIds: string[] };
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
// `near` keeps a project together: when any of those tiles are on the board,
// the new one is cut from the largest of them instead.
export function insertLeaf(root: PaneNode | null, leafId: string, near: readonly string[] = []): PaneNode {
  if (!root) return { type: "leaf", leafId };
  if (leafIds(root).includes(leafId)) return root;
  const boxes = leafBoxes(root);
  const neighbours = boxes.filter(box => near.includes(box.leafId));
  const largest = (neighbours.length ? neighbours : boxes).reduce((best, box) => box.width * box.height > best.width * best.height ? box : best);
  const direction: SplitDirection = largest.width >= largest.height ? "horizontal" : "vertical";
  return splitLeaf(root, largest.leafId, leafId, direction);
}

// drop leaves that are no longer wanted, then insert the new ones in order.
export function reconcileLeaves(root: PaneNode | null, ids: readonly string[], groupOf?: (id: string) => string | null): PaneNode | null {
  const wanted = new Set(ids);
  let next: PaneNode | null = root;
  if (next) for (const id of leafIds(next)) if (!wanted.has(id) && next) next = removeLeaf(next, id);
  for (const id of ids) {
    if (next && leafIds(next).includes(id)) continue;
    const group = groupOf?.(id);
    const near = group && next ? leafIds(next).filter(other => groupOf?.(other) === group) : [];
    next = insertLeaf(next, id, near);
  }
  return next;
}

// equal shares: the first of k nodes takes 1/k, the rest split what is left.
function chain(nodes: PaneNode[], direction: SplitDirection): PaneNode {
  if (nodes.length === 1) return nodes[0];
  return { type: "split", direction, ratio: 1 / nodes.length, first: nodes[0], second: chain(nodes.slice(1), direction) };
}

// the order Arrange lays tiles out in: one run per group, largest group first
// so it fills a row of its own, ties by first appearance. ungrouped tiles trail.
export function groupedOrder(ids: readonly string[], groupOf: (id: string) => string | null): string[] {
  const groups = new Map<string, string[]>();
  const loose: string[] = [];
  for (const id of ids) {
    const group = groupOf(id);
    if (group == null) loose.push(id);
    else groups.set(group, [...(groups.get(group) ?? []), id]);
  }
  return [...[...groups.values()].sort((a, b) => b.length - a.length).flat(), ...loose];
}

// a balanced grid in reading order, so tiles given next to each other stay next
// to each other: ceil(sqrt(n)) columns per row, the last row taking the rest.
export function arrangeLeaves(ids: readonly string[]): PaneNode | null {
  const unique = [...new Set(ids)];
  if (!unique.length) return null;
  const columns = Math.ceil(Math.sqrt(unique.length));
  const rows: PaneNode[] = [];
  for (let start = 0; start < unique.length; start += columns) {
    rows.push(chain(unique.slice(start, start + columns).map(leafId => ({ type: "leaf", leafId })), "horizontal"));
  }
  return chain(rows, "vertical");
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
  const empty: MissionLayout = { version: 1, root: null, expandedLeafId: null, pinnedSessionIds: [], dismissedSessionIds: [] };
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
  const stored = value as { root?: unknown; expandedLeafId?: unknown; pinnedSessionIds?: unknown; dismissedSessionIds?: unknown };
  const root = parseNode(stored.root);
  const expanded = typeof stored.expandedLeafId === "string" && root && leafIds(root).includes(stored.expandedLeafId) ? stored.expandedLeafId : null;
  const pinnedSessionIds = Array.isArray(stored.pinnedSessionIds)
    ? [...new Set(stored.pinnedSessionIds.filter((id): id is string => typeof id === "string" && seen.has(id)))] : [];
  // Dismissed leaves are deliberately absent from the tree, so membership in
  // `seen` cannot gate them the way it gates pins.
  const dismissedSessionIds = Array.isArray(stored.dismissedSessionIds)
    ? [...new Set(stored.dismissedSessionIds.filter((id): id is string => typeof id === "string"))] : [];
  return { version: 1, root, expandedLeafId: expanded, pinnedSessionIds, dismissedSessionIds };
}

export function readLayout(): MissionLayout {
  try { return parseLayout(globalThis.localStorage?.getItem(MISSION_LAYOUT_KEY) ?? null); } catch { return { version: 1, root: null, expandedLeafId: null, pinnedSessionIds: [], dismissedSessionIds: [] }; }
}

export function writeLayout(layout: MissionLayout) {
  try { globalThis.localStorage?.setItem(MISSION_LAYOUT_KEY, JSON.stringify(layout)); } catch { /* storage unavailable */ }
}
