/**
 * The chat overview at the head of the dock's Work pane: where the chat runs,
 * its branch, its uncommitted changes, its latest checks, and one line per
 * turn. React-free, so every row's rule tests as data.
 *
 * Every row reads a fact Bridge already holds and hides when that fact is
 * absent. Nothing here estimates: a turn with no timed tool calls has no
 * duration, and a branch with no base comparison has no ahead/behind line.
 */

import { alignTurns, isToolItem, mergeConversationProjections, reduceConversation, toolCallDisplay, type ConversationItem } from "./conversation";
import { projectSessionConversationCached } from "./transcript/projectionCache";
import type { AgentEvent, BaseBranchDivergence, CompletionSummary, SessionEntry } from "./types";

export type OverviewTone = "passed" | "failed" | "waived" | "pending";

export type OverviewRow =
  | { kind: "where"; path: string; display: string; detail?: string }
  | { kind: "branch"; name: string; detail?: string }
  | { kind: "changes"; files: number; additions: number; deletions: number }
  | { kind: "checks"; tone: OverviewTone; label: string; detail?: string };

/** Files a turn's own edit calls touched. Counts are absent when no call reported any. */
export type TurnChanges = { files: number; additions?: number; deletions?: number };

export type TurnHistoryEntry = {
  turn: number;
  key: string;
  /** The first line of the prompt that opened the turn. */
  title: string;
  /** The prompt's forest entry, when it is durable: what a reveal scrolls to. */
  entryId?: string;
  /** The runtime that answered, as stamped on its rows. */
  harness?: string;
  /** Wall-clock span of the turn's tool calls, the same measure as "Worked for". */
  durationMs?: number;
  /** The turn still in flight. Its duration and changes are not final, so neither is shown. */
  live: boolean;
  /** Null when the turn edited nothing. */
  changes: TurnChanges | null;
};

export type ChatOverviewInput = {
  /** Where the chat's agent works: its own cwd, else its workspace checkout. */
  path?: string | null;
  projectName?: string;
  branch?: string | null;
  divergence?: BaseBranchDivergence | null;
  /** The workspace's uncommitted stats, or null when the chat has no repository. */
  changes?: { files: number; additions: number; deletions: number } | null;
  completion?: CompletionSummary | null;
  items: readonly ConversationItem[];
  turnActive: boolean;
};

export type ChatOverviewModel = {
  headline?: string;
  rows: OverviewRow[];
  turns: TurnHistoryEntry[];
};

/** The leaf directories identify a checkout; the ancestors rarely do. */
export function overviewPath(path: string): string {
  const parts = path.split("/").filter(Boolean);
  return parts.length > 3 ? `…/${parts.slice(-3).join("/")}` : path;
}

/** "2 ahead of origin/main", or nothing when the comparison is unavailable. */
export function divergencePhrase(divergence: BaseBranchDivergence | null | undefined): string | undefined {
  if (!divergence || divergence.unavailableReason || !divergence.baseRef) return undefined;
  const { ahead, behind, baseRef } = divergence;
  if (ahead > 0 && behind > 0) return `${ahead} ahead, ${behind} behind ${baseRef}`;
  if (ahead > 0) return `${ahead} ahead of ${baseRef}`;
  if (behind > 0) return `${behind} behind ${baseRef}`;
  return `Up to date with ${baseRef}`;
}

const VERDICT: Record<CompletionSummary["verdict"], { label: string; tone: OverviewTone }> = {
  verified: { label: "Verified", tone: "passed" },
  waived: { label: "Verified with waiver", tone: "waived" },
  changes_requested: { label: "Changes requested", tone: "failed" },
  failed: { label: "Verification failed", tone: "failed" },
  superseded: { label: "Evidence superseded", tone: "pending" },
  verifying: { label: "Verifying", tone: "pending" },
};

/** A check run settled one way or the other, read the way the transcript's check rows read it. */
function settledCheck(item: ConversationItem): "passed" | "failed" | undefined {
  const call = toolCallDisplay(item);
  if (!call.check || !call.command) return undefined;
  if (call.status === "failed" || (call.exitCode !== undefined && call.exitCode !== 0) || call.check.failures) return "failed";
  return call.status === "completed" ? "passed" : undefined;
}

/**
 * The latest verification result. Bridge's own completion proof wins; without
 * one, the newest settled test, build, typecheck or lint run in the chat.
 */
export function checksRow(completion: CompletionSummary | null | undefined, items: readonly ConversationItem[]): Extract<OverviewRow, { kind: "checks" }> | undefined {
  if (completion) {
    const verdict = VERDICT[completion.verdict];
    const failing = completion.checks.find(check => check.required && check.status === "failed");
    const counts = `${completion.passedRequired} of ${completion.totalRequired} required checks passed`;
    return {
      kind: "checks",
      tone: verdict.tone,
      label: verdict.label,
      detail: failing ? `${failing.command || failing.checkId} failed · ${counts}` : counts,
    };
  }
  for (let index = items.length - 1; index >= 0; index--) {
    const item = items[index];
    if (!isToolItem(item)) continue;
    const outcome = settledCheck(item);
    if (!outcome) continue;
    const call = toolCallDisplay(item);
    return {
      kind: "checks",
      tone: outcome,
      label: call.command!,
      detail: [outcome === "passed" ? "Passed" : "Failed", call.check?.summary].filter(Boolean).join(" · "),
    };
  }
  return undefined;
}

/** The prompt's first line, whitespace folded. CSS truncates the rest. */
export function turnTitle(text: string): string {
  const line = text.split("\n").map(part => part.trim()).find(Boolean) ?? "";
  return line.replace(/\s+/g, " ") || "Message with no text";
}

/** The transcript's own duration words, so a turn reads the same here as under "Worked for". */
export function formatTurnDuration(ms: number): string {
  const total = Math.max(1, Math.round(ms / 1000));
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  if (minutes >= 60) return `${Math.floor(minutes / 60)}h ${String(minutes % 60).padStart(2, "0")}m`;
  if (minutes === 0) return `${seconds}s`;
  return `${minutes}m ${String(seconds).padStart(2, "0")}s`;
}

function stringList(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((entry): entry is string => typeof entry === "string" && entry !== "") : [];
}

/**
 * What one turn's edit calls changed. A file edited twice counts once with its
 * counts summed. A multi-file summary (a turn diff) adds its files, and its
 * counts only when none of those files had a call of its own, so a summary of
 * edits already counted is not counted twice. Failed and unfinished edits
 * changed nothing yet.
 */
export function turnChanges(items: readonly ConversationItem[]): TurnChanges | null {
  const perFile = new Map<string, { additions?: number; deletions?: number }>();
  const summaries: { paths: string[]; additions?: number; deletions?: number }[] = [];
  const seen = new Set<string>();
  for (const item of items) {
    if (!isToolItem(item)) continue;
    const call = toolCallDisplay(item);
    if (call.verb !== "edit" || call.status === "failed" || call.status === "running" || call.preparing) continue;
    const identity = call.sourceCallId ?? item.identity ?? item.key;
    if (seen.has(identity)) continue;
    seen.add(identity);
    const paths = stringList(item.data.paths);
    if (paths.length > 1) {
      summaries.push({ paths, additions: call.additions, deletions: call.deletions });
      continue;
    }
    const path = call.path ?? call.target ?? identity;
    const current = perFile.get(path) ?? {};
    perFile.set(path, {
      additions: call.additions === undefined ? current.additions : (current.additions ?? 0) + call.additions,
      deletions: call.deletions === undefined ? current.deletions : (current.deletions ?? 0) + call.deletions,
    });
  }
  const files = new Set(perFile.keys());
  let additions: number | undefined;
  let deletions: number | undefined;
  const add = (counts: { additions?: number; deletions?: number }) => {
    if (counts.additions !== undefined) additions = (additions ?? 0) + counts.additions;
    if (counts.deletions !== undefined) deletions = (deletions ?? 0) + counts.deletions;
  };
  for (const counts of perFile.values()) add(counts);
  for (const summary of summaries) {
    const overlaps = summary.paths.some(path => perFile.has(path));
    for (const path of summary.paths) files.add(path);
    if (!overlaps) add(summary);
  }
  if (files.size === 0) return null;
  return { files: files.size, additions, deletions };
}

/** Wall-clock the turn's tool calls occupied: the union's span, as "Worked for" measures it. */
function turnDuration(items: readonly ConversationItem[]): number | undefined {
  let start = Infinity;
  let end = -Infinity;
  for (const item of items) {
    if (!isToolItem(item)) continue;
    const at = item.createdAt ? Date.parse(item.createdAt) : NaN;
    if (!Number.isFinite(at)) continue;
    start = Math.min(start, at);
    end = Math.max(end, at + (toolCallDisplay(item).durationMs ?? 0));
  }
  const span = end - start;
  return Number.isFinite(span) && span > 0 ? span : undefined;
}

function isUserTurn(item: ConversationItem): boolean {
  return item.type === "message" && item.role === "user";
}

/**
 * One entry per turn the person opened, newest first. Rows before the first
 * prompt (turn 0) belong to no turn and are skipped.
 */
export function turnHistory(items: readonly ConversationItem[], turnActive: boolean): TurnHistoryEntry[] {
  const turns = new Map<number, ConversationItem[]>();
  for (const item of items) {
    if (item.turn <= 0) continue;
    const rows = turns.get(item.turn);
    if (rows) rows.push(item);
    else turns.set(item.turn, [item]);
  }
  const last = Math.max(0, ...turns.keys());
  const entries: TurnHistoryEntry[] = [];
  for (const [turn, rows] of turns) {
    const prompt = rows.find(isUserTurn);
    if (!prompt) continue;
    const live = turnActive && turn === last;
    entries.push({
      turn,
      key: prompt.identity ?? prompt.key,
      title: turnTitle(prompt.text),
      entryId: prompt.entryId,
      harness: rows.find(row => !isUserTurn(row) && row.harness)?.harness,
      durationMs: live ? undefined : turnDuration(rows),
      live,
      changes: live ? null : turnChanges(rows),
    });
  }
  return entries.sort((a, b) => b.turn - a.turn);
}

/** The chat's rows, durable and live merged and numbered by turn, the way the transcript sees them. */
export function chatConversationItems(entries: SessionEntry[] | undefined, activeLeafId: string | null, events: AgentEvent[]): ConversationItem[] {
  const durable = entries?.length ? projectSessionConversationCached(entries, activeLeafId) : [];
  return alignTurns(mergeConversationProjections(durable, reduceConversation(events)));
}

function headline(input: ChatOverviewInput, checks: OverviewRow | undefined): string | undefined {
  if (input.turnActive) return "Working";
  if (checks?.kind === "checks" && checks.tone === "failed") return "Checks failed";
  if (!input.changes) return undefined;
  return input.changes.files > 0 ? "Changes to review" : "No uncommitted changes";
}

export function chatOverview(input: ChatOverviewInput): ChatOverviewModel {
  const rows: OverviewRow[] = [];
  if (input.path) rows.push({ kind: "where", path: input.path, display: overviewPath(input.path), detail: input.projectName || undefined });
  const branch = input.divergence && !input.divergence.unavailableReason && input.divergence.branch ? input.divergence.branch : input.branch;
  if (branch) rows.push({ kind: "branch", name: branch, detail: divergencePhrase(input.divergence) });
  if (input.changes) rows.push({ kind: "changes", ...input.changes });
  const checks = checksRow(input.completion, input.items);
  if (checks) rows.push(checks);
  return { headline: headline(input, checks), rows, turns: turnHistory(input.items, input.turnActive) };
}
