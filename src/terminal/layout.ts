// Adapted from Orca's terminal-tab-types.ts and headless-terminal-split-layout.ts
// at f2d5711b2d32e9f11277cd63805c76b0b5f9ddf7. Copyright (c) 2026 Lovecast Inc.
// MIT; see THIRD_PARTY_NOTICES.md. Bridge adds validated workspace tab actions.
export type SplitDirection = "horizontal" | "vertical";
export type PaneNode = { type: "leaf"; leafId: string } | {
  type: "split"; direction: SplitDirection; first: PaneNode; second: PaneNode; ratio: number;
};
export type TerminalTab = { id: string; title: string; root: PaneNode };
export type TerminalLayout = { version: 1; tabs: TerminalTab[]; activeTabId: string | null; activeLeafId: string | null; expandedLeafId: string | null };
export const emptyLayout = (): TerminalLayout => ({ version: 1, tabs: [], activeTabId: null, activeLeafId: null, expandedLeafId: null });
export const leafIds = (node: PaneNode): string[] => node.type === "leaf" ? [node.leafId] : [...leafIds(node.first), ...leafIds(node.second)];

export function splitLeaf(node: PaneNode, target: string, created: string, direction: SplitDirection, before = false): PaneNode {
  if (node.type === "leaf") {
    if (node.leafId !== target) return node;
    const leaf: PaneNode = { type: "leaf", leafId: created };
    return { type: "split", direction, first: before ? leaf : node, second: before ? node : leaf, ratio: 0.5 };
  }
  return { ...node, first: splitLeaf(node.first, target, created, direction, before), second: splitLeaf(node.second, target, created, direction, before) };
}

export function removeLeaf(node: PaneNode, leafId: string): PaneNode | null {
  if (node.type === "leaf") return node.leafId === leafId ? null : node;
  const first = removeLeaf(node.first, leafId);
  const second = removeLeaf(node.second, leafId);
  if (!first) return second;
  if (!second) return first;
  return { ...node, first, second };
}

export function resizeNode(node: PaneNode, path: string, ratio: number): PaneNode {
  if (node.type === "leaf") return node;
  if (!path) return { ...node, ratio: Math.min(0.9, Math.max(0.1, ratio)) };
  return path[0] === "0" ? { ...node, first: resizeNode(node.first, path.slice(1), ratio) } : { ...node, second: resizeNode(node.second, path.slice(1), ratio) };
}

export function addTab(layout: TerminalLayout, leafId: string, title: string): TerminalLayout {
  const id = crypto.randomUUID();
  return { ...layout, tabs: [...layout.tabs, { id, title, root: { type: "leaf", leafId } }], activeTabId: id, activeLeafId: leafId, expandedLeafId: null };
}

export function closeLeaf(layout: TerminalLayout, leafId: string): TerminalLayout {
  const tabs = layout.tabs.flatMap(tab => {
    const root = removeLeaf(tab.root, leafId);
    return root ? [{ ...tab, root }] : [];
  });
  const active = tabs.find(tab => tab.id === layout.activeTabId) ?? tabs.at(-1);
  const activeLeafId = active ? (leafIds(active.root).includes(layout.activeLeafId ?? "") ? layout.activeLeafId : leafIds(active.root)[0]) : null;
  return { ...layout, tabs, activeTabId: active?.id ?? null, activeLeafId, expandedLeafId: layout.expandedLeafId === leafId ? null : layout.expandedLeafId };
}

export function insertSplit(layout: TerminalLayout, target: string, leafId: string, direction: SplitDirection, before = false): TerminalLayout {
  const targetTab = layout.tabs.find(tab => leafIds(tab.root).includes(target));
  if (!targetTab || target === leafId) return layout;
  const stripped = layout.tabs.some(tab => leafIds(tab.root).includes(leafId)) ? closeLeaf(layout, leafId) : layout;
  return { ...stripped, tabs: stripped.tabs.map(tab => tab.id === targetTab.id ? { ...tab, root: splitLeaf(tab.root, target, leafId, direction, before) } : tab), activeTabId: targetTab.id, activeLeafId: leafId, expandedLeafId: null };
}

/** Persisted input is untrusted. Repair missing/duplicate terminal references
 * and bounded-depth trees without launching anything while restoring. */
export function restoreLayout(value: unknown, terminals: { terminalId: string; title: string }[]): TerminalLayout {
  const available = new Set(terminals.map(t => t.terminalId));
  const seen = new Set<string>();
  const parseNode = (raw: unknown, depth = 0): PaneNode | null => {
    if (!raw || typeof raw !== "object" || depth > 32) return null;
    const n = raw as Record<string, unknown>;
    if (n.type === "leaf" && typeof n.leafId === "string" && available.has(n.leafId) && !seen.has(n.leafId)) {
      seen.add(n.leafId); return { type: "leaf", leafId: n.leafId };
    }
    if (n.type !== "split" || !["horizontal", "vertical"].includes(String(n.direction))) return null;
    const first = parseNode(n.first, depth + 1), second = parseNode(n.second, depth + 1);
    if (!first) return second;
    if (!second) return first;
    return { type: "split", direction: n.direction as SplitDirection, first, second, ratio: typeof n.ratio === "number" && Number.isFinite(n.ratio) ? Math.min(0.9, Math.max(0.1, n.ratio)) : 0.5 };
  };
  const raw = value && typeof value === "object" ? value as Partial<TerminalLayout> : {};
  const tabIds = new Set<string>();
  const tabs = raw.version === 1 && Array.isArray(raw.tabs) ? raw.tabs.flatMap(tab => {
    if (!tab || typeof tab.id !== "string" || tabIds.has(tab.id)) return [];
    tabIds.add(tab.id);
    const root = parseNode(tab.root);
    return root ? [{ id: tab.id, title: typeof tab.title === "string" ? tab.title.slice(0, 120) : "Terminal", root }] : [];
  }) : [];
  for (const terminal of terminals) if (!seen.has(terminal.terminalId)) tabs.push({ id: `restored-${terminal.terminalId}`, title: terminal.title, root: { type: "leaf", leafId: terminal.terminalId } });
  const active = tabs.find(tab => tab.id === raw.activeTabId) ?? tabs[0];
  const ids = active ? leafIds(active.root) : [];
  return { version: 1, tabs, activeTabId: active?.id ?? null, activeLeafId: ids.includes(raw.activeLeafId ?? "") ? raw.activeLeafId! : ids[0] ?? null, expandedLeafId: ids.includes(raw.expandedLeafId ?? "") ? raw.expandedLeafId! : null };
}
