/**
 * What one turn changed, and the one-line account that closes it.
 *
 * A settled turn that edited files ends with a card listing those files, and
 * every settled turn ends with a caption: when it finished, how long it
 * worked, and the model that ran it. Both are read off the transcript's own
 * rows, never asked of the backend: the files from the turn's edit calls, the
 * duration from the same wall-clock span the "Worked for" header measures,
 * and the model from the chat's model-change rows.
 *
 * Pure and React-free, so the rules are testable as data.
 */

import { isToolItem, itemSignature, toolCallDisplay, type ConversationItem, type Rendered } from "./conversation";
import { modelLabel } from "./utils";

/** Added, modified, deleted: the letter a file wears in the card. */
export type ChangeMark = "A" | "M" | "D";

export interface TurnFileChange {
  /** The path as the chat shows it: relative to its working directory when inside it. */
  path: string;
  mark: ChangeMark;
  /** Absent when the call named the file but not its share of the counts. */
  additions?: number;
  deletions?: number;
  /** The unified diffs this turn applied to the file, oldest first. */
  patches: string[];
}

export interface TurnChanges {
  /** In the order the turn first touched them. */
  files: TurnFileChange[];
  additions: number;
  deletions: number;
}

export interface TurnFolder {
  /** The directory with a trailing slash, or "" for the working directory itself. */
  folder: string;
  files: TurnFileChange[];
}

/** What closes a settled turn: its changes card and its caption, either of which may be absent. */
export interface TurnEnd {
  turn: number;
  changes: TurnChanges | null;
  line: string | null;
  /** Equal signatures draw the same thing, so a memoized row can keep its props. */
  signature: string;
}

/* ── Reading one call ─────────────────────────────────────────────────── */

/** One file one call changed, before the turn folds repeats together. */
interface FileEdit {
  path: string;
  additions?: number;
  deletions?: number;
  mark?: ChangeMark;
  patch?: string;
}

interface CallEdits {
  files: FileEdit[];
  /** Counts the call reported for files it could not split them across. */
  unattributed?: { additions: number; deletions: number };
}

function bag(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : {};
}

function text(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value : undefined;
}

function count(value: unknown): number | undefined {
  if (typeof value === "number") return Number.isFinite(value) && value >= 0 ? value : undefined;
  if (typeof value === "string" && value.trim()) {
    const parsed = Number(value);
    return Number.isFinite(parsed) && parsed >= 0 ? parsed : undefined;
  }
  return undefined;
}

/** Whether a string is a unified diff rather than a file's bare contents. */
function isUnifiedDiff(value: string): boolean {
  return /^@@ /m.test(value) || /^diff --git /m.test(value) || /^--- \S/m.test(value) && /^\+\+\+ \S/m.test(value);
}

/**
 * Lines added and removed by a unified diff. Mirrors `diffstat` in
 * `bridge-core/src/agent.rs`, which stamps the counts a provider leaves out,
 * so a file split out of a joined patch is counted the way its call was.
 */
export function diffstat(patch: string): { additions: number; deletions: number } {
  let additions = 0;
  let deletions = 0;
  for (const line of patch.split("\n")) {
    if (/^(\+\+\+|---|@@|diff |index |Index:|====|\*\*\*|\\)/.test(line)) continue;
    if (line.startsWith("+")) additions += 1;
    else if (line.startsWith("-")) deletions += 1;
  }
  return { additions, deletions };
}

function lineCount(content: string): number {
  const trimmed = content.endsWith("\n") ? content.slice(0, -1) : content;
  return trimmed ? trimmed.split("\n").length : 0;
}

/** What a patch says about the file's existence: created, removed, or neither. */
function markFromPatch(patch: string | undefined): ChangeMark | undefined {
  if (!patch) return undefined;
  if (/^--- \/dev\/null\s*$/m.test(patch) || /^new file mode /m.test(patch)) return "A";
  if (/^\+\+\+ \/dev\/null\s*$/m.test(patch) || /^deleted file mode /m.test(patch)) return "D";
  return undefined;
}

/** A provider's change kind, a word or `{ type }`: Codex sends both shapes. */
function markFromKind(kind: unknown): ChangeMark | undefined {
  const word = (text(kind) ?? text(bag(kind).type) ?? "").toLowerCase();
  if (/^(add|added|create|created|new)$/.test(word)) return "A";
  if (/^(delete|deleted|remove|removed)$/.test(word)) return "D";
  if (word) return "M";
  return undefined;
}

/** A file entry's own diff, as a unified diff whatever shape it arrived in. */
function entryPatch(path: string, mark: ChangeMark | undefined, body: string | undefined): string | undefined {
  if (!body) return undefined;
  if (isUnifiedDiff(body)) return body;
  // A created or removed file can arrive as its bare contents.
  const lines = body.endsWith("\n") ? body.slice(0, -1).split("\n") : body.split("\n");
  if (mark === "D") return [`--- a/${path}`, "+++ /dev/null", `@@ -1,${lines.length} +0,0 @@`, ...lines.map(line => `-${line}`)].join("\n");
  return [mark === "A" ? "--- /dev/null" : `--- a/${path}`, `+++ b/${path}`, `@@ -0,0 +1,${lines.length} @@`, ...lines.map(line => `+${line}`)].join("\n");
}

/**
 * Per-file entries a call carries as a list: Codex's `changes[]`
 * (`{ path, kind, diff }`), OpenCode's `files[]` (`{ file, type, patch,
 * additions, deletions }`), and ACP's `diff` content blocks (`{ path,
 * oldText, newText }`).
 */
function listedEdits(data: Record<string, unknown>): FileEdit[] {
  const state = bag(data.state);
  const metadata = { ...bag(data.metadata), ...bag(state.metadata) };
  const input = { ...bag(data.input), ...bag(state.input) };
  for (const list of [data.changes, input.changes, metadata.files, data.files]) {
    if (!Array.isArray(list)) continue;
    const edits = list.flatMap((value): FileEdit[] => {
      const entry = bag(value);
      const path = text(entry.path) ?? text(entry.file) ?? text(entry.filePath) ?? text(entry.file_path);
      if (!path) return [];
      const mark = markFromKind(entry.kind ?? entry.type);
      const body = text(entry.diff) ?? text(entry.patch) ?? text(entry.unifiedDiff);
      const patch = entryPatch(path, mark, body);
      const stat = patch ? diffstat(patch) : undefined;
      return [{
        path,
        mark: mark ?? markFromPatch(patch),
        additions: count(entry.additions) ?? stat?.additions,
        deletions: count(entry.deletions) ?? stat?.deletions,
        patch,
      }];
    });
    if (edits.length) return edits;
  }
  const blocks = bag(data.update).content ?? data.content;
  if (Array.isArray(blocks)) {
    const edits = blocks.flatMap((value): FileEdit[] => {
      const entry = bag(value);
      const path = entry.type === "diff" ? text(entry.path) : undefined;
      if (!path) return [];
      const before = typeof entry.oldText === "string" ? entry.oldText : undefined;
      const after = typeof entry.newText === "string" ? entry.newText : "";
      // ACP leaves `oldText` out for a file the call created.
      return [{ path, mark: before === undefined ? "A" : "M", additions: lineCount(after), deletions: lineCount(before ?? "") }];
    });
    if (edits.length) return edits;
  }
  return [];
}

/**
 * A patch that names several files, cut at its file headers: a unified diff's
 * `diff --git`/`---`/`+++` lines, or an apply_patch body's `*** Update File:`
 * markers. Hunk line counts are honoured, so a removed line that happens to
 * start with `-- ` is never read as a header. Undefined when the patch names
 * no file of its own.
 */
export function splitPatchByFile(patch: string | undefined): FileEdit[] | undefined {
  if (!patch) return undefined;
  // Counted here rather than by `diffstat`: inside a hunk the line counts say
  // what a line is, so a removed `-- comment` is a deletion, not a header.
  const files: Array<{ path: string; mark?: ChangeMark; lines: string[]; additions: number; deletions: number }> = [];
  let current: (typeof files)[number] | undefined;
  let oldLeft = 0;
  let newLeft = 0;
  // A `---` header belongs to the file its `+++` names, which may be the next one.
  let pendingOld: string | undefined;
  const open = (path: string, mark?: ChangeMark) => {
    current = { path, mark, lines: [], additions: 0, deletions: 0 };
    files.push(current);
  };
  const strip = (raw: string) => raw.replace(/\t.*$/, "").trim().replace(/^[ab]\//, "");
  for (const line of patch.split("\n")) {
    const inside = oldLeft > 0 || newLeft > 0;
    if (!inside) {
      const marker = /^\*\*\* (Update|Add|Delete) File: (.+)$/.exec(line);
      if (marker) {
        open(marker[2].trim(), marker[1] === "Add" ? "A" : marker[1] === "Delete" ? "D" : "M");
        continue;
      }
      const git = /^diff --git a\/(.+) b\/(.+)$/.exec(line);
      if (git) {
        open(git[2]);
        pendingOld = undefined;
        continue;
      }
      if (line.startsWith("--- ")) {
        pendingOld = line;
        continue;
      }
      if (line.startsWith("+++ ")) {
        const next = strip(line.slice(4));
        const path = next === "/dev/null" && pendingOld ? strip(pendingOld.slice(4)) : next;
        // A `diff --git` header already opened this file; the `+++` only confirms it.
        if (path && path !== "/dev/null" && current?.path !== path) open(path);
        if (pendingOld) current?.lines.push(pendingOld);
        current?.lines.push(line);
        pendingOld = undefined;
        continue;
      }
    }
    const hunk = /^@@+ -\d+(?:,(\d+))? \+\d+(?:,(\d+))? @@/.exec(line);
    if (hunk) {
      oldLeft = hunk[1] === undefined ? 1 : Number(hunk[1]);
      newLeft = hunk[2] === undefined ? 1 : Number(hunk[2]);
      current?.lines.push(line);
      continue;
    }
    if (inside) {
      if (line.startsWith("+")) newLeft -= 1;
      else if (line.startsWith("-")) oldLeft -= 1;
      else if (!line.startsWith("\\")) { oldLeft -= 1; newLeft -= 1; }
    }
    // Outside a counted hunk only an apply_patch body has content lines, and
    // its own markers were taken above.
    if (current && !line.startsWith("***")) {
      if (line.startsWith("+")) current.additions += 1;
      else if (line.startsWith("-")) current.deletions += 1;
    }
    current?.lines.push(line);
  }
  if (!files.length) return undefined;
  return files.map(({ lines, ...file }) => {
    const body = lines.join("\n");
    return { ...file, mark: file.mark ?? markFromPatch(body), patch: body };
  });
}

/**
 * Per-item results, keyed by what the row says. The reducer rebuilds every
 * item on every flush, so identity would never hit; the signature does, and a
 * settled turn's patches are counted once rather than twenty times a second.
 */
const editCache = new Map<string, CallEdits>();
const EDIT_CACHE_LIMIT = 512;

function callEdits(item: ConversationItem): CallEdits {
  const call = toolCallDisplay(item);
  const key = `${itemSignature(item)}\u0000${call.path ?? ""}\u0000${call.patch?.length ?? 0}`;
  const cached = editCache.get(key);
  if (cached) return cached;
  const result = readCallEdits(item);
  if (editCache.size >= EDIT_CACHE_LIMIT) editCache.delete(editCache.keys().next().value!);
  editCache.set(key, result);
  return result;
}

function readCallEdits(item: ConversationItem): CallEdits {
  const call = toolCallDisplay(item);
  const data = item.data;
  const listed = listedEdits(data);
  if (listed.length) return { files: listed };
  const paths = Array.isArray(data.paths) ? data.paths.filter((path): path is string => typeof path === "string" && !!path.trim()) : [];
  const state = bag(data.state);
  const patchText = text(bag(data.input).patchText) ?? text(bag(state.input).patchText);
  const split = splitPatchByFile(call.patch) ?? splitPatchByFile(patchText);
  if (split && (split.length > 1 || paths.length > 1)) return { files: split };
  // A call with no path of its own is a snapshot of the whole turn or session
  // (Codex's turn diff, OpenCode's session diff), not an edit: counting it
  // would count every file a second time.
  if (!call.path) return { files: [] };
  const stat = call.patch ? diffstat(call.patch) : undefined;
  const additions = call.additions ?? stat?.additions;
  const deletions = call.deletions ?? stat?.deletions;
  if (paths.length > 1) {
    // Several files and nothing to split the counts by: name each file, and
    // keep the call's counts in the total without inventing a share.
    return {
      files: paths.map(path => ({ path })),
      unattributed: { additions: additions ?? 0, deletions: deletions ?? 0 },
    };
  }
  return {
    files: [{
      path: call.path,
      additions,
      deletions,
      // ACP names a removal in the call's own category.
      mark: split?.[0]?.mark ?? markFromPatch(call.patch) ?? (data.kind === "delete" ? "D" : undefined),
      patch: call.patch,
    }],
  };
}

/* ── Folding a transcript into turns ──────────────────────────────────── */

/** Relative to the chat's working directory, when the path sits inside it. */
export function relativePath(path: string, cwd?: string | null): string {
  if (!cwd) return path;
  const root = cwd.endsWith("/") ? cwd : `${cwd}/`;
  return path.startsWith(root) ? path.slice(root.length) : path;
}

/**
 * Every turn's file changes, keyed by turn.
 *
 * A file edited twice in one turn is one entry with summed counts. A failed
 * edit changed nothing and a read is not a change, so neither counts. An edit
 * that claims to create a file the chat had already read or changed is a
 * rewrite (Claude's Write reports every write as a creation), so it reads `M`.
 */
export function turnChangesByTurn(items: readonly ConversationItem[], cwd?: string | null): Map<number, TurnChanges> {
  // The newest state of each call wins: a started and a completed row for one
  // call are one edit.
  const identity = (item: ConversationItem) => toolCallDisplay(item).sourceCallId ?? item.identity ?? item.key;
  const last = new Map<string, number>();
  items.forEach((item, index) => { if (isToolItem(item)) last.set(identity(item), index); });

  const seen = new Set<string>();
  const turns = new Map<number, { files: Map<string, TurnFileChange>; additions: number; deletions: number }>();
  items.forEach((item, index) => {
    if (!isToolItem(item) || last.get(identity(item)) !== index) return;
    const call = toolCallDisplay(item);
    if (call.status === "failed" || call.preparing) return;
    if (call.verb === "read") {
      if (call.path) seen.add(relativePath(call.path, cwd));
      return;
    }
    if (call.verb !== "edit") return;
    const { files, unattributed } = callEdits(item);
    if (!files.length) return;
    const turn = turns.get(item.turn) ?? { files: new Map(), additions: 0, deletions: 0 };
    turns.set(item.turn, turn);
    for (const edit of files) {
      const path = relativePath(edit.path, cwd);
      const held = turn.files.get(path);
      if (!held) {
        const mark = edit.mark === "A" ? (seen.has(path) ? "M" : "A") : edit.mark ?? "M";
        turn.files.set(path, { path, mark, additions: edit.additions, deletions: edit.deletions, patches: edit.patch ? [edit.patch] : [] });
      } else {
        // Created this turn stays created; removed at the end of the turn is removed.
        if (edit.mark === "D" && held.mark !== "A") held.mark = "D";
        else if (held.mark === "D") held.mark = "M";
        held.additions = edit.additions === undefined && held.additions === undefined ? undefined : (held.additions ?? 0) + (edit.additions ?? 0);
        held.deletions = edit.deletions === undefined && held.deletions === undefined ? undefined : (held.deletions ?? 0) + (edit.deletions ?? 0);
        if (edit.patch) held.patches.push(edit.patch);
      }
      turn.additions += edit.additions ?? 0;
      turn.deletions += edit.deletions ?? 0;
      seen.add(path);
    }
    if (unattributed) {
      turn.additions += unattributed.additions;
      turn.deletions += unattributed.deletions;
    }
  });
  const out = new Map<number, TurnChanges>();
  for (const [turn, { files, additions, deletions }] of turns) out.set(turn, { files: [...files.values()], additions, deletions });
  return out;
}

/** The files as folders, folders sorted, each folder's files sorted, loose files last. */
export function changeTree(files: readonly TurnFileChange[]): TurnFolder[] {
  const folders = new Map<string, TurnFileChange[]>();
  for (const file of files) {
    const cut = file.path.lastIndexOf("/");
    const folder = cut >= 0 ? file.path.slice(0, cut + 1) : "";
    const held = folders.get(folder);
    if (held) held.push(file);
    else folders.set(folder, [file]);
  }
  const byName = (a: string, b: string) => (a < b ? -1 : a > b ? 1 : 0);
  return [...folders.entries()]
    .sort(([a], [b]) => (a === "" ? 1 : b === "" ? -1 : byName(a, b)))
    .map(([folder, entries]) => ({ folder, files: [...entries].sort((a, b) => byName(a.path, b.path)) }));
}

export function fileName(path: string): string {
  return path.slice(path.lastIndexOf("/") + 1) || path;
}

/* ── The turn line ────────────────────────────────────────────────────── */

/**
 * Wall-clock the turn's tool work occupied: the union span of its calls, each
 * from its start to its start plus its duration. The same measure as the
 * "Worked for" header, so the caption and the header never disagree. Zero
 * when no call carries a timestamp.
 */
export function workedMs(items: readonly ConversationItem[]): number {
  const spans = items.filter(isToolItem).map(item => {
    const start = item.createdAt ? Date.parse(item.createdAt) : NaN;
    return { start, end: start + (toolCallDisplay(item).durationMs ?? 0) };
  }).filter(span => Number.isFinite(span.start));
  return spans.length ? Math.max(...spans.map(span => span.end)) - Math.min(...spans.map(span => span.start)) : 0;
}

/** `34s`, `2m 05s`, `1h 03m`: the same words the "Worked for" header uses. */
export function formatWorked(ms: number): string {
  const total = Math.max(1, Math.round(ms / 1000));
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  if (minutes >= 60) return `${Math.floor(minutes / 60)}h ${String(minutes % 60).padStart(2, "0")}m`;
  if (minutes === 0) return `${seconds}s`;
  return `${minutes}m ${String(seconds).padStart(2, "0")}s`;
}

/** The clock time a turn ended, with its date when that was not today. */
export function formatTurnTime(at: number, now = Date.now()): string {
  const when = new Date(at);
  const time = when.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  if (when.toDateString() === new Date(now).toDateString()) return time;
  return `${when.toLocaleDateString(undefined, { month: "short", day: "numeric" })}, ${time}`;
}

/**
 * The model that ran each turn, keyed by turn.
 *
 * A model change sits at the end of the turn it followed, so a turn is run by
 * the model in effect when it opened. Turns before the first change ran the
 * model that change switched away from; with no change at all, every turn ran
 * the chat's current model.
 */
export function turnModels(items: readonly ConversationItem[], sessionModel?: string | null): Map<number, string> {
  const changes = items.filter(item => item.type === "model-change");
  let current = text(changes[0]?.data.previousModel) ?? (changes.length ? undefined : text(sessionModel));
  const out = new Map<number, string>();
  for (const item of items) {
    // A switch that names only a harness leaves the model unknown, not unchanged.
    if (item.type === "model-change") current = text(item.data.model);
    else if (!out.has(item.turn) && current) out.set(item.turn, current);
  }
  return out;
}

/** Bookkeeping a turn carries past its last real row: never the place its end is drawn. */
function isBookkeeping(item: ConversationItem): boolean {
  return item.type === "checkpoint" || item.type === "compaction" || item.type === "context-compacted"
    || item.type === "branch-summary" || item.type === "model-change" || item.type === "raw" || item.data.staleBase === true;
}

function entryTurn(entry: Rendered): number | undefined {
  return entry.kind === "item" ? entry.item.turn : entry.items[0]?.turn;
}

/**
 * Where each turn ends on screen: the key of its last drawn row that is the
 * agent's (not the person's message, not bookkeeping), keyed by turn.
 */
export function turnEndKeys(entries: readonly Rendered[]): Map<number, string> {
  const out = new Map<number, string>();
  for (const entry of entries) {
    if (entry.kind === "raw-group") continue;
    if (entry.kind === "item" && (isBookkeeping(entry.item) || (entry.item.type === "message" && entry.item.role === "user"))) continue;
    const turn = entryTurn(entry);
    if (turn !== undefined) out.set(turn, entry.key);
  }
  return out;
}

export interface TurnEndOptions {
  /** The turn still running, if any: it has no end yet. */
  liveTurn?: number | null;
  cwd?: string | null;
  sessionModel?: string | null;
  now?: number;
}

/**
 * What closes each settled turn, keyed by the rendered entry it follows.
 * A turn with neither changes nor anything to say in its caption is absent.
 */
export function describeTurnEnds(entries: readonly Rendered[], items: readonly ConversationItem[], options: TurnEndOptions = {}): Map<string, TurnEnd> {
  const changes = turnChangesByTurn(items, options.cwd);
  const models = turnModels(items, options.sessionModel);
  const byTurn = new Map<number, ConversationItem[]>();
  for (const item of items) {
    const rows = byTurn.get(item.turn);
    if (rows) rows.push(item);
    else byTurn.set(item.turn, [item]);
  }
  const out = new Map<string, TurnEnd>();
  for (const [turn, key] of turnEndKeys(entries)) {
    if (options.liveTurn !== undefined && options.liveTurn !== null && turn === options.liveTurn) continue;
    const rows = byTurn.get(turn) ?? [];
    const stamps = rows.map(item => (item.createdAt ? Date.parse(item.createdAt) : NaN)).filter(Number.isFinite);
    const worked = workedMs(rows);
    const model = models.get(turn);
    const line = [
      stamps.length ? formatTurnTime(Math.max(...stamps), options.now) : undefined,
      worked > 0 ? formatWorked(worked) : undefined,
      model ? modelLabel(model) : undefined,
    ].filter(Boolean).join(" · ") || null;
    const turnChanges = changes.get(turn) ?? null;
    if (!turnChanges && !line) continue;
    const signature = [
      turn,
      line ?? "",
      turnChanges ? `${turnChanges.additions}:${turnChanges.deletions}:${turnChanges.files.map(file => `${file.path}:${file.mark}:${file.additions ?? ""}:${file.deletions ?? ""}:${file.patches.reduce((sum, patch) => sum + patch.length, 0)}`).join("|")}` : "",
    ].join("\u0000");
    out.set(key, { turn, changes: turnChanges, line, signature });
  }
  return out;
}

/**
 * The next map, with every entry whose signature did not move swapped for the
 * object the previous render held. A memoized row compares its props by
 * identity, so this is what keeps a live flush from redrawing every settled
 * turn's card.
 */
export function keepStable(previous: ReadonlyMap<string, TurnEnd>, next: Map<string, TurnEnd>): Map<string, TurnEnd> {
  for (const [key, end] of next) {
    const held = previous.get(key);
    if (held && held.signature === end.signature) next.set(key, held);
  }
  return next;
}
