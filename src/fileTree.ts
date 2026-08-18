/** A node in the editor's file tree, built from a flat list of paths. */
export interface TreeNode {
  name: string;
  /** Repository-relative path. Directories carry a trailing-free path too. */
  path: string;
  /** Present on directories only; files have no children. */
  children?: TreeNode[];
}

/**
 * Build a nested tree from `git ls-files` output.
 *
 * Directories sort before files and both sort case-insensitively, which is the
 * order every file explorer uses and the only one that reads as alphabetical
 * to a human.
 */
export function buildFileTree(paths: string[]): TreeNode[] {
  const root: TreeNode = { name: "", path: "", children: [] };
  for (const path of paths) {
    const segments = path.split("/").filter(Boolean);
    if (!segments.length) continue;
    let node = root;
    segments.forEach((segment, index) => {
      const isLeaf = index === segments.length - 1;
      const childPath = segments.slice(0, index + 1).join("/");
      const children = node.children ?? (node.children = []);
      let next = children.find(child => child.name === segment && !child.children === isLeaf);
      if (!next) {
        next = isLeaf ? { name: segment, path: childPath } : { name: segment, path: childPath, children: [] };
        children.push(next);
      }
      node = next;
    });
  }
  sortTree(root.children ?? []);
  return root.children ?? [];
}

function sortTree(nodes: TreeNode[]): void {
  nodes.sort((a, b) => {
    const aDir = a.children ? 0 : 1;
    const bDir = b.children ? 0 : 1;
    if (aDir !== bDir) return aDir - bDir;
    return a.name.localeCompare(b.name, undefined, { sensitivity: "base" });
  });
  for (const node of nodes) if (node.children) sortTree(node.children);
}

/**
 * Collapse directory chains that contain nothing but one more directory, so a
 * tree shows `src/components/ui` on one row instead of three rows deep with
 * nothing to choose between them.
 */
export function collapseChains(nodes: TreeNode[]): TreeNode[] {
  return nodes.map(node => {
    if (!node.children) return node;
    let current = node;
    let name = node.name;
    while (current.children?.length === 1 && current.children[0].children) {
      current = current.children[0];
      name = `${name}/${current.name}`;
    }
    return { name, path: current.path, children: collapseChains(current.children ?? []) };
  });
}

/** Every directory path on the way to `path`, outermost first. */
export function ancestorPaths(path: string): string[] {
  const segments = path.split("/").filter(Boolean);
  segments.pop();
  return segments.map((_, index) => segments.slice(0, index + 1).join("/"));
}

/** Score a path against a ⌘P query. Higher is better; 0 means no match. */
function fuzzyScore(path: string, query: string): number {
  const haystack = path.toLowerCase();
  const needle = query.toLowerCase();
  if (!needle) return 1;
  // A run of the query inside the basename is what people usually mean.
  const base = haystack.slice(haystack.lastIndexOf("/") + 1);
  const baseHit = base.indexOf(needle);
  if (baseHit === 0) return 1000 - path.length;
  if (baseHit > 0) return 800 - path.length;
  const pathHit = haystack.indexOf(needle);
  if (pathHit >= 0) return 600 - path.length;
  // Otherwise fall back to subsequence matching, rewarding matches that stay
  // close together ("apptsx" → "App.tsx").
  let index = 0;
  let last = -1;
  let gaps = 0;
  for (const char of needle) {
    const found = haystack.indexOf(char, index);
    if (found < 0) return 0;
    if (last >= 0) gaps += found - last - 1;
    last = found;
    index = found + 1;
  }
  return Math.max(1, 400 - gaps - path.length);
}

/** Rank paths for the ⌘P palette, best first. */
export function rankPaths(paths: string[], query: string, limit = 50): string[] {
  const trimmed = query.trim();
  if (!trimmed) return paths.slice(0, limit);
  return paths
    .map(path => ({ path, score: fuzzyScore(path, trimmed) }))
    .filter(entry => entry.score > 0)
    .sort((a, b) => b.score - a.score || a.path.localeCompare(b.path))
    .slice(0, limit)
    .map(entry => entry.path);
}
