// Formatting and the copilot briefing for the Storage page.
//
// Disk figures use decimal units, the way Finder and System Settings report
// them, so "18 GB free" here reads the same number macOS shows.

import type { DiskEntry, DiskListing, DiskOverview, DiskSuggestion } from "./types";

export function diskBytes(value: number | null | undefined): string {
  if (value === null || value === undefined) return "—";
  if (value < 1000) return `${value} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let size = value / 1000;
  let unit = 0;
  while (size >= 1000 && unit < units.length - 1) {
    size /= 1000;
    unit += 1;
  }
  return `${size >= 100 ? size.toFixed(0) : size.toFixed(1)} ${units[unit]}`;
}

/** Suggestions can nest (`~/Library/Caches` holds Homebrew's cache), so a
 *  total counts only the outermost of each chain. */
export function cleanupTotal(suggestions: readonly DiskSuggestion[], safety?: "safe" | "review"): number {
  const chosen = suggestions.filter(item => !safety || item.safety === safety);
  return chosen
    .filter(item => !chosen.some(other => other !== item && item.path.startsWith(`${other.path}/`)))
    .reduce((total, item) => total + (item.sizeBytes ?? 0), 0);
}

export function displayPath(path: string, home: string | undefined): string {
  if (home && (path === home || path.startsWith(`${home}/`))) return `~${path.slice(home.length)}`;
  return path;
}

/** The first message of a storage chat: what Bridge measured, and the rules
 *  the agent works under. The person's question goes last. */
export function storageBriefing({ question, overview, listing, selected }: {
  question: string;
  overview: DiskOverview | null;
  listing: DiskListing | null;
  selected: readonly DiskEntry[];
}): string {
  const home = overview?.home;
  const lines: string[] = ["I want help freeing disk space on this Mac. Here is what Bridge's Storage page measured just now."];
  if (overview?.volume) {
    const { totalBytes, freeBytes } = overview.volume;
    lines.push("", `Disk: ${diskBytes(freeBytes)} free of ${diskBytes(totalBytes)}.`);
  }
  const suggestions = [...(overview?.suggestions ?? [])]
    .filter(item => (item.sizeBytes ?? 0) > 0)
    .sort((a, b) => (b.sizeBytes ?? 0) - (a.sizeBytes ?? 0))
    .slice(0, 12);
  if (suggestions.length > 0) {
    lines.push("", "Known cleanup candidates:");
    for (const item of suggestions) lines.push(`- ${item.label}: ${diskBytes(item.sizeBytes)} at ${displayPath(item.path, home)} (${item.safety === "safe" ? "rebuilt on demand" : "review first"})`);
  }
  if (listing && listing.entries.length > 0) {
    lines.push("", `Largest items in ${displayPath(listing.path, home)}:`);
    for (const entry of listing.entries.slice(0, 15)) lines.push(`- ${entry.name}${entry.kind === "directory" ? "/" : ""}: ${diskBytes(entry.sizeBytes)}`);
  }
  if (selected.length > 0) {
    lines.push("", "I have selected:");
    for (const entry of selected) lines.push(`- ${displayPath(entry.path, home)} (${diskBytes(entry.sizeBytes)})`);
  }
  lines.push(
    "",
    "How to help: investigate with read-only commands first (du, find, ls). Explain what each large item is and whether it comes back on its own. Prefer the owning tool's own cleanup (brew cleanup, docker system prune, xcrun simctl delete unavailable, npm cache clean) over deleting its folder. Before deleting anything, list exactly what you will remove and how much it frees, and wait for me to say yes. Move things to the Trash rather than deleting outright unless I ask. Never touch system folders, keychains, or ~/.ssh.",
    "",
    `My question: ${question.trim()}`,
  );
  return lines.join("\n");
}
