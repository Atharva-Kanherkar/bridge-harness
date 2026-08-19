import type { WorkFact, WorkFactAction, WorkFactSeverity } from "../protocol/generated/protocol";

// How the Work board reads. The board arrives already ordered — severity, then
// oldest, then key — so nothing here sorts: this module decides what a row *says*,
// which is the part that has to be right in three places at once.
//
// A stale divergence reading is the case worth the care. Its numbers describe the
// past, and a row that looked current would invite acting on them. So freshness
// shows up three ways: the tense of the sentence, the weight of the numbers, and
// which action is offered at all. Any one of those alone could be missed.

/** Which glyph names where a fact came from. Bridge's own facts get plain shapes;
 * connector sources arrive with Suggested work in a later slice. */
export type FactSource = "check" | "approval" | "queue" | "branch";

export const SOURCE_BY_KIND: Record<WorkFact["kind"], FactSource> = {
  failed_completion_check: "check",
  blocked_worker_queue_item: "queue",
  actionable_approval: "approval",
  workspace_behind_base: "branch",
};

/** What the source tile is called, in text, so the glyph is never load-bearing. */
export const SOURCE_LABEL: Record<FactSource, string> = {
  check: "Completion check",
  approval: "Approval",
  queue: "Worker queue",
  branch: "Base branch",
};

/** Severity bands, in the order the backend already sorts by, so grouping under
 * them reorders nothing. */
export const SEVERITY_ORDER: WorkFactSeverity[] = ["blocking", "attention", "info"];

export const SEVERITY_LABEL: Record<WorkFactSeverity, string> = {
  blocking: "Blocking",
  attention: "Attention",
  info: "Info",
};

export const SEVERITY_CAPTION: Record<WorkFactSeverity, string> = {
  blocking: "work is stopped until you act",
  attention: "work continues, something is wrong",
  info: "worth knowing, not worth interrupting for",
};

export type FactBand = { severity: WorkFactSeverity; facts: WorkFact[] };

/** Group by severity while preserving the given order inside each band. A band with
 * no facts is left out rather than rendered empty. */
export function bandFacts(facts: WorkFact[]): FactBand[] {
  return SEVERITY_ORDER
    .map(severity => ({ severity, facts: facts.filter(fact => fact.severity === severity) }))
    .filter(band => band.facts.length > 0);
}

/** What the rail badge counts. Info never counts: a number that includes things
 * nobody has to act on is a number that never reaches zero, and a number that never
 * reaches zero stops being read. */
export function needsYouCount(facts: WorkFact[]): number {
  return facts.filter(fact => fact.severity === "blocking" || fact.severity === "attention").length;
}

/** Whole units only. A board is glanced at, and "4 min ago" is read faster than
 * "4 minutes and 12 seconds ago". */
export function relativeTime(observedAt: string, now: Date): string {
  const seen = Date.parse(observedAt);
  // A timestamp this build cannot parse is not rendered as "Invalid Date"; the
  // reader is told the age is unknown, which is the honest reading.
  if (Number.isNaN(seen)) return "at an unknown time";
  const seconds = Math.max(0, Math.round((now.getTime() - seen) / 1000));
  if (seconds < 45) return "just now";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} h ago`;
  return `${Math.round(hours / 24)} d ago`;
}

/** The freshness line, in words. `stale` and `unknown` say so; `live` does not need
 * to, because current is the unremarkable case. */
export function freshnessText(fact: WorkFact, now: Date): string {
  const when = relativeTime(fact.observedAt, now);
  if (fact.freshness === "unknown") return "not measured";
  if (fact.freshness === "stale") return `measured ${when} · stale`;
  return fact.kind === "workspace_behind_base" ? `measured ${when}` : `seen ${when}`;
}

/** Freshness as part of the accessible name, so a screen reader hears the same
 * claim the colour makes. */
export function freshnessAnnouncement(fact: WorkFact, now: Date): string {
  if (fact.freshness === "unknown") return "not measured, nothing is known";
  if (fact.freshness === "stale") return `measured ${relativeTime(fact.observedAt, now)}, stale`;
  return freshnessText(fact, now);
}

/** Everything a screen reader should hear for one row, in the order a sighted
 * reader takes it in. Colour appears nowhere in it. */
export function factAnnouncement(fact: WorkFact, now: Date): string {
  const source = SOURCE_LABEL[SOURCE_BY_KIND[fact.kind]];
  return `${SEVERITY_LABEL[fact.severity]}, ${source}: ${fact.title}. ${freshnessAnnouncement(fact, now)}.`;
}

/** Whether a fact's numbers describe the present. Only a live reading does, and
 * only divergence has numbers that can go out of date in the first place. */
export function describesThePresent(fact: WorkFact): boolean {
  return fact.freshness === "live";
}

/** The detail line, rendered dimmed when its numbers are no longer current. */
export function detailIsDimmed(fact: WorkFact): boolean {
  return fact.freshness === "stale";
}

/** The clause appended to a stale detail, which is the tense tell.
 *
 * The board's titles come from the backend, and it says "has drifted" whether the
 * reading is current or not — the tense has to come from here. Appending a sentence
 * rather than rewriting the title: string surgery on a verb inside a sentence Bridge
 * composed elsewhere would break the first time that sentence is reworded, and a
 * stated claim is harder to miss than a changed letter anyway.
 *
 * `null` for anything whose numbers are current, and for `unknown`, which has no
 * numbers to qualify. */
export function stalenessClause(fact: WorkFact, now: Date): string | null {
  if (fact.freshness !== "stale") return null;
  return `As of ${relativeTime(fact.observedAt, now)}. These numbers describe the past.`;
}

/** The label on a fact's one action.
 *
 * Exhaustive over the four action variants: a fifth variant added to the contract
 * fails to compile here rather than rendering an unlabelled button. */
export function actionLabel(action: WorkFactAction): string {
  switch (action.kind) {
    case "reviewCompletionCheck":
      return "Review check";
    case "answerApproval":
      return "Answer";
    case "refreshWorkspaceBase":
      return "Fast-forward";
    case "refreshBaseObservation":
      return "Measure again";
  }
}

/** Whether the action is the page's primary one. A fast-forward and a re-measure
 * are offers; a blocking fact's action is the thing you came here to do. */
export function actionIsPrimary(fact: WorkFact): boolean {
  return fact.severity === "blocking";
}

/** What a retry of a failed action is called, which is never the original label —
 * "Fast-forward" after a failed fast-forward reads as though nothing happened. */
export const RETRY_LABEL = "Try again";
