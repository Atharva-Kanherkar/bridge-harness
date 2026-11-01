// A page's history as the person reads it: runs of edits by one author folded
// into one entry, a plain name for who wrote each, and a line diff of a version
// against the page as it is now. Pure, so it is tested without a DOM.

import { diff, lines } from "./notesDraft";
import { harnessLabel } from "./utils";
import type { NoteVersion } from "./types";

/** Edits by one author closer together than this read as one sitting. */
export const RUN_GAP_MS = 10 * 60 * 1000;

export interface HistoryEntry {
  /** The newest version in the run: the one a restore brings back. */
  seq: number;
  actor: string;
  at: string;
  label: string;
  added: number;
  removed: number;
  /** How many versions the run folds together. */
  count: number;
}

export function versionLabel(actor: string, chatTitle: (sessionId: string) => string | undefined): string {
  if (actor === "person") return "You";
  if (actor === "file-import") return "Edited outside Bridge";
  const agent = /^agent:([^:]+):(.+)$/.exec(actor);
  if (agent) return `${harnessLabel(agent[1])} in ${chatTitle(agent[2]) ?? "a chat"}`;
  if (actor.startsWith("model:tasks")) return "Turn into tasks";
  if (actor.startsWith("model:structure")) return "Structure";
  return actor;
}

/** Versions that changed the words, newest first, folded into runs. */
export function historyEntries(versions: NoteVersion[], chatTitle: (sessionId: string) => string | undefined): HistoryEntry[] {
  const entries: HistoryEntry[] = [];
  // Oldest first, so a run grows forward in time.
  for (const version of [...versions].filter(item => item.bodyChanged).reverse()) {
    const last = entries[entries.length - 1];
    if (last && last.actor === version.actor && Date.parse(version.at) - Date.parse(last.at) <= RUN_GAP_MS) {
      last.seq = version.seq;
      last.at = version.at;
      last.added += version.added;
      last.removed += version.removed;
      last.count += 1;
      continue;
    }
    entries.push({ seq: version.seq, actor: version.actor, at: version.at, label: versionLabel(version.actor, chatTitle), added: version.added, removed: version.removed, count: 1 });
  }
  return entries.reverse();
}

export interface DiffLine { kind: "same" | "added" | "removed"; text: string }

/** `before -> after` line by line: what a version would add and remove. */
export function lineDiff(before: string, after: string): DiffLine[] {
  const a = lines(before);
  const b = lines(after);
  const strip = (line: string) => line.replace(/\n$/, "");
  const rows: DiffLine[] = [];
  let at = 0;
  for (const hunk of diff(a, b)) {
    for (const line of a.slice(at, hunk.start)) rows.push({ kind: "same", text: strip(line) });
    for (const line of a.slice(hunk.start, hunk.end)) rows.push({ kind: "removed", text: strip(line) });
    for (const line of hunk.tokens) rows.push({ kind: "added", text: strip(line) });
    at = hunk.end;
  }
  for (const line of a.slice(at)) rows.push({ kind: "same", text: strip(line) });
  return rows;
}
