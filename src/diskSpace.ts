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

export const SNAPSHOT_FENCE = "storage-snapshot";

/** What the Storage page measured, as the agent reads it. Sent at the end of a
 *  message only when it changed since the last one, so the chat never carries
 *  the same numbers twice. The agent's brief is its system prompt, not this. */
export function storageSnapshot({ overview, listing, home: homeListing, selected, trashed = [] }: {
  overview: DiskOverview | null;
  listing: DiskListing | null;
  home?: DiskListing | null;
  selected: readonly DiskEntry[];
  trashed?: readonly { path: string; sizeBytes: number | null }[];
}): string {
  const home = overview?.home;
  const lines: string[] = [];
  if (overview?.volume) {
    const { totalBytes, freeBytes, usedBytes } = overview.volume;
    lines.push(`Disk: ${diskBytes(freeBytes)} free of ${diskBytes(totalBytes)} (${Math.round((usedBytes / totalBytes) * 100)}% used).`);
  }
  const homeTop = (homeListing?.entries ?? []).filter(entry => (entry.sizeBytes ?? 0) > 0).slice(0, 8);
  if (homeTop.length > 0) lines.push(`Largest in ~: ${homeTop.map(entry => `${entry.name} ${diskBytes(entry.sizeBytes)}`).join(", ")}.`);
  const suggestions = [...(overview?.suggestions ?? [])]
    .filter(item => (item.sizeBytes ?? 0) > 0)
    .sort((a, b) => (b.sizeBytes ?? 0) - (a.sizeBytes ?? 0))
    .slice(0, 12);
  if (suggestions.length > 0) {
    lines.push("Known cleanup candidates:");
    for (const item of suggestions) lines.push(`- ${item.label}: ${diskBytes(item.sizeBytes)} at ${displayPath(item.path, home)} (${item.safety === "safe" ? "rebuilt on demand" : "review first"})`);
  }
  if (listing && listing.entries.length > 0 && listing.path !== homeListing?.path) {
    lines.push(`Viewing ${displayPath(listing.path, home)} (${diskBytes(listing.sizeBytes)}${listing.measuring ? ", still measuring" : ""}):`);
    for (const entry of listing.entries.slice(0, 12)) lines.push(`- ${entry.name}${entry.kind === "directory" ? "/" : ""}: ${diskBytes(entry.sizeBytes)}`);
  }
  if (selected.length > 0) {
    lines.push("Selected on the page:");
    for (const entry of selected) lines.push(`- ${displayPath(entry.path, home)} (${diskBytes(entry.sizeBytes)})`);
  }
  if (trashed.length > 0) {
    lines.push("Moved to the Trash from your plans (Trash not yet emptied):");
    for (const entry of trashed) lines.push(`- ${displayPath(entry.path, home)} (${diskBytes(entry.sizeBytes)})`);
  }
  return lines.join("\n");
}

/** A message with the page's snapshot folded onto its end. */
export function withSnapshot(question: string, snapshot: string): string {
  const text = question.trim();
  return snapshot ? `${text}\n\n\`\`\`${SNAPSHOT_FENCE}\n${snapshot}\n\`\`\`` : text;
}

/** Undo `withSnapshot` for display: the person sees what they typed and a chip. */
export function splitSnapshot(text: string): { text: string; snapshot: string | null } {
  const marker = `\`\`\`${SNAPSHOT_FENCE}\n`;
  const at = text.lastIndexOf(marker);
  if (at < 0) return { text, snapshot: null };
  const body = text.slice(at + marker.length).replace(/\n?```\s*$/, "");
  return { text: text.slice(0, at).trimEnd(), snapshot: body };
}

export interface StoragePlanItem { path: string; sizeBytes: number | null; why: string; safety: "safe" | "review" }
export interface StoragePlanCommand { run: string; why: string; frees: number | null }
export interface StoragePlan { title: string; items: StoragePlanItem[]; commands: StoragePlanCommand[] }

const MAX_PLAN_ROWS = 20;
const text = (value: unknown) => typeof value === "string" ? value.trim() : "";
const bytes = (value: unknown) => typeof value === "number" && Number.isFinite(value) && value >= 0 ? value : null;

/** Parse the agent's ```storage-plan block. Anything malformed is dropped row
 *  by row; a plan with nothing left (or a block still streaming in) is null. */
export function parseStoragePlan(body: string, home: string | undefined): StoragePlan | null {
  let raw: unknown;
  try { raw = JSON.parse(body); } catch { return null; }
  if (!raw || typeof raw !== "object") return null;
  const record = raw as Record<string, unknown>;
  const items = (Array.isArray(record.items) ? record.items : []).flatMap((item): StoragePlanItem[] => {
    if (!item || typeof item !== "object") return [];
    const row = item as Record<string, unknown>;
    let path = text(row.path);
    if (path.startsWith("~/")) { if (!home) return []; path = `${home}${path.slice(1)}`; }
    if (!path.startsWith("/") || path === "/" || path.split("/").includes("..")) return [];
    return [{ path, sizeBytes: bytes(row.sizeBytes), why: text(row.why), safety: row.safety === "safe" ? "safe" : "review" }];
  }).slice(0, MAX_PLAN_ROWS);
  const commands = (Array.isArray(record.commands) ? record.commands : []).flatMap((item): StoragePlanCommand[] => {
    if (!item || typeof item !== "object") return [];
    const row = item as Record<string, unknown>;
    const run = text(row.run);
    return run ? [{ run, why: text(row.why), frees: bytes(row.frees) }] : [];
  }).slice(0, MAX_PLAN_ROWS);
  if (items.length === 0 && commands.length === 0) return null;
  return { title: text(record.title) || "Cleanup plan", items, commands };
}
