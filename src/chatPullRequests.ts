import type { GithubCheckConclusion, PullRequestCheck, SessionPullRequest } from "./protocol/generated/protocol";

// Pure logic behind the in-chat PR card: one status per PR, one tone per
// check, freshness text, and which transitions deserve an announcement. Kept
// out of the component so every state the issue names is assertable without a
// DOM — and so the card and the strip above the composer can never disagree.

export type ChatPrPhase = "merged" | "closed" | "failing" | "running" | "passing" | "none";
export type ChatPrTone = "merged" | "closed" | "danger" | "warning" | "success" | "muted";

export type ChatPrStatus = {
  phase: ChatPrPhase;
  tone: ChatPrTone;
  /** The card's headline: short, sentence case, never a false green. */
  headline: string;
  /** Always-present text alternative for the strip and screen readers. */
  summary: string;
  /** Checks still moving: drives the live hint and the refresh cadence. */
  live: boolean;
  /** Merged or closed: nothing will change without a human, so polling stops. */
  terminal: boolean;
};

const plural = (count: number, word: string) => `${count} ${word}${count === 1 ? "" : "s"}`;

export function chatPrStatus(pr: Pick<SessionPullRequest, "state" | "isDraft" | "checks">): ChatPrStatus {
  const { checks } = pr;
  const moving = checks.queued + checks.inProgress;
  if (pr.state === "merged") {
    return { phase: "merged", tone: "merged", headline: "Merged", summary: "Merged", live: false, terminal: true };
  }
  if (pr.state === "closed") {
    return { phase: "closed", tone: "closed", headline: "Closed without merging", summary: "Closed", live: false, terminal: true };
  }
  const draft = pr.isDraft ? "Draft · " : "";
  if (checks.failed > 0) {
    const headline = moving > 0
      ? `${plural(checks.failed, "check")} failing · ${moving} still running`
      : `${plural(checks.failed, "check")} failing`;
    return { phase: "failing", tone: "danger", headline, summary: `${draft}${plural(checks.failed, "check")} failing`, live: moving > 0, terminal: false };
  }
  if (moving > 0) {
    const settled = checks.total - moving;
    // Queued-only is its own honest state: nothing has started yet.
    const headline = checks.inProgress === 0
      ? `${plural(checks.queued, "check")} queued`
      : `Checks running · ${settled} of ${checks.total} done`;
    return { phase: "running", tone: "warning", headline, summary: `${draft}CI running ${settled}/${checks.total}`, live: true, terminal: false };
  }
  if (checks.passed > 0) {
    return { phase: "passing", tone: "success", headline: checks.passed === checks.total ? "All checks passed" : `${checks.passed} of ${checks.total} checks passed`, summary: `${draft}Checks passed`, live: false, terminal: false };
  }
  if (checks.total > 0) {
    // Everything skipped or cancelled: not a failure, and not a pass either.
    return { phase: "none", tone: "muted", headline: `${plural(checks.total, "check")} skipped or cancelled`, summary: `${draft}No checks passed`, live: false, terminal: false };
  }
  return { phase: "none", tone: "muted", headline: "No checks reported", summary: `${draft}No checks`, live: false, terminal: false };
}

export type CheckTone = "failed" | "running" | "queued" | "passed" | "skipped";

const FAILED: ReadonlyArray<GithubCheckConclusion> = ["failure", "timedOut", "startupFailure", "actionRequired"];

export function checkTone(check: Pick<PullRequestCheck, "status" | "conclusion">): CheckTone {
  if (check.status === "queued") return "queued";
  if (check.status === "inProgress") return "running";
  if (check.conclusion && FAILED.includes(check.conclusion)) return "failed";
  if (check.conclusion === "success") return "passed";
  if (!check.conclusion) return "queued";
  return "skipped";
}

const TONE_ORDER: Record<CheckTone, number> = { failed: 0, running: 1, queued: 2, passed: 3, skipped: 4 };

/** Failures first — they are why the list gets opened — then what is still
 * moving, then the settled rest. Stable within a tone. */
export function orderChecks<T extends Pick<PullRequestCheck, "status" | "conclusion" | "name">>(checks: readonly T[]): T[] {
  return checks
    .map((check, index) => ({ check, index }))
    .sort((a, b) => TONE_ORDER[checkTone(a.check)] - TONE_ORDER[checkTone(b.check)] || a.index - b.index)
    .map(({ check }) => check);
}

/** Relative freshness for the card footer, e.g. "just now", "4m ago". */
export function freshness(fetchedAt: string | null | undefined, now: number): string {
  if (!fetchedAt) return "never refreshed";
  const at = Date.parse(fetchedAt);
  if (Number.isNaN(at)) return "never refreshed";
  const seconds = Math.max(0, Math.round((now - at) / 1000));
  if (seconds < 10) return "just now";
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 48) return `${hours}h ago`;
  return `${Math.round(hours / 24)}d ago`;
}

/** Stable identity of one card across refetches. */
export const chatPrKey = (pr: Pick<SessionPullRequest, "url" | "number">) => `${pr.url}#${pr.number}`;

/**
 * The sentence a screen reader hears when a card's status changes — or null
 * when nothing worth interrupting for happened. A refetch that lands the same
 * phase (every poll tick while CI runs) is silent; that is the difference
 * between announced state changes and live-region noise.
 */
export function announcement(previous: ChatPrPhase | undefined, pr: SessionPullRequest): string | null {
  const next = chatPrStatus(pr);
  if (previous === undefined || previous === next.phase) return null;
  return `Pull request #${pr.number}: ${next.headline}.`;
}

/** Background refresh cadence while a card is visible. The server poller does
 * the GitHub work; this only re-reads its cache, and stops once every PR is
 * terminal. */
export function refreshInterval(prs: readonly SessionPullRequest[]): number | null {
  if (prs.length === 0) return null;
  const statuses = prs.map(chatPrStatus);
  if (statuses.every(status => status.terminal)) return null;
  if (prs.some(pr => pr.stale)) return 30_000;
  return statuses.some(status => status.live) ? 20_000 : 90_000;
}

/** Which PRs a GitHub event concerns: the poller keys events by workspace and
 * number, and a chat's rows are already verified against its workspace. */
export function eventTouches(prs: readonly SessionPullRequest[], number: number): boolean {
  return prs.some(pr => pr.number === number);
}
