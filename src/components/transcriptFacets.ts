/**
 * What the raw event stream is *about*, computed once.
 *
 * The transcript pane shows frames as frames — that is its job, and why it is
 * a named raw-stream site in `src/transcript/harnessBranchGate.test.ts`. But a
 * flat list of a few thousand frames answers no question a reader actually
 * has, and the first question is nearly always "where did this go wrong". So
 * every row carries three derived facts: which bucket it belongs to, which
 * turn it fell in, and whether it is a failure.
 *
 * None of it branches on harness identity. A failed tool call looks the same
 * whether Codex, Claude, OpenCode, Cursor or Grok produced it, because the
 * normalizers in `bridge-core` already agreed on the vocabulary before the
 * event reached here.
 */

import { readWireKind } from "../transcript/wire";
import type { AgentEvent } from "../types";

export const TRANSCRIPT_FACETS = [
  "all",
  "messages",
  "thinking",
  "tools",
  "turns",
  "usage",
  "approvals",
  "delegation",
  "problems",
] as const;

export type TranscriptFacet = (typeof TRANSCRIPT_FACETS)[number];

export const FACET_LABELS: Record<TranscriptFacet, string> = {
  all: "All",
  messages: "Messages",
  thinking: "Thinking",
  tools: "Tools",
  turns: "Turns",
  usage: "Usage",
  approvals: "Approvals",
  delegation: "Delegation",
  problems: "Problems",
};

/** The bucket a row lives in. Exactly one, chosen by first match. */
export type PrimaryFacet = Exclude<TranscriptFacet, "all" | "problems"> | "other";

export type TranscriptRow = {
  event: AgentEvent;
  /** The wire kind, already opened. Rows are the one place that happens. */
  kind: string;
  /** Which turn this fell in. Zero until the first `turn.started`. */
  turnIndex: number;
  facet: PrimaryFacet;
  /** Why this row is a failure, in words, or null if it is not one. */
  problem: string | null;
  /** The one line worth reading without expanding the row. */
  detail: string;
};

/** Statuses that mean the thing did not work. */
const FAILED_STATUSES = new Set(["failed", "error", "errored"]);

function bucket(kind: string): PrimaryFacet {
  if (kind === "user.message" || kind === "assistant.message" || kind.startsWith("message.")) return "messages";
  if (kind === "reasoning" || kind.startsWith("reasoning.")) return "thinking";
  if (kind.startsWith("tool.") || kind.startsWith("command.")) return "tools";
  if (kind.startsWith("turn.")) return "turns";
  if (kind.startsWith("usage.")) return "usage";
  if (kind.startsWith("approval.") || kind.startsWith("permission.") || kind.startsWith("question.")) return "approvals";
  if (kind.startsWith("delegation.") || kind.startsWith("worker.")) return "delegation";
  return "other";
}

/**
 * Whether this event is evidence something went wrong, and what to say about
 * it. Kept as rules over the normalized vocabulary rather than a list of
 * kinds, so a kind added by a future adapter is classified without anyone
 * remembering to come back here.
 */
export function problemReason(event: AgentEvent, kind: string): string | null {
  const status = (event.status ?? "").toLowerCase();
  if (kind === "worker.result") {
    // A worker that did not complete is the single most expensive failure in
    // Bridge: a whole delegated session's work, ending in nothing.
    return status && status !== "completed" ? `A delegated worker ended ${status}.` : null;
  }
  if (FAILED_STATUSES.has(status)) {
    if (kind.startsWith("tool.") || kind.startsWith("command.")) return "This tool call failed.";
    if (kind.startsWith("turn.")) return "The turn ended in failure.";
    return "This step reported a failure.";
  }
  if (kind === "error" || kind.endsWith(".error")) return "The provider reported an error.";
  if (kind.endsWith(".failed") || kind.endsWith("_failed")) return "This step failed.";
  return null;
}

/**
 * The line a reader scans. A settled thought whose status is `completed` is
 * the case that matters: showing the status there says nothing, and the
 * thought itself is the whole reason the row exists.
 */
function detailFor(event: AgentEvent, facet: PrimaryFacet): string {
  const text = (event.text ?? "").trim();
  const title = (event.title ?? "").trim();
  if (facet === "thinking" || facet === "messages") return text || title;
  return title || text || (event.status ?? "");
}

export function buildTranscriptRows(events: AgentEvent[]): TranscriptRow[] {
  let turnIndex = 0;
  return events.map(event => {
    const kind = readWireKind(event.kind);
    // The same rule the JSONL export uses, so a reader comparing the pane with
    // an exported file never has to reconcile two numbering schemes.
    if (kind === "turn.started") turnIndex += 1;
    const facet = bucket(kind);
    return { event, kind, turnIndex, facet, problem: problemReason(event, kind), detail: detailFor(event, facet) };
  });
}

export function matchesFacet(row: TranscriptRow, facet: TranscriptFacet): boolean {
  if (facet === "all") return true;
  if (facet === "problems") return row.problem !== null;
  return row.facet === facet;
}

/** Counts for every chip, including the ones that would read zero. */
export function countFacets(rows: TranscriptRow[]): Record<TranscriptFacet, number> {
  const counts = Object.fromEntries(TRANSCRIPT_FACETS.map(facet => [facet, 0])) as Record<TranscriptFacet, number>;
  for (const row of rows) {
    counts.all += 1;
    if (row.problem) counts.problems += 1;
    if (row.facet !== "other") counts[row.facet] += 1;
  }
  return counts;
}

/** The facet narrows; the query narrows again inside it. */
export function filterTranscriptRows(
  rows: TranscriptRow[],
  facet: TranscriptFacet,
  query: string,
): TranscriptRow[] {
  const needle = query.trim().toLowerCase();
  return rows.filter(row => {
    if (!matchesFacet(row, facet)) return false;
    if (!needle) return true;
    return row.kind.toLowerCase().includes(needle)
      || row.detail.toLowerCase().includes(needle)
      || (row.event.text ?? "").toLowerCase().includes(needle)
      || (row.event.title ?? "").toLowerCase().includes(needle);
  });
}
