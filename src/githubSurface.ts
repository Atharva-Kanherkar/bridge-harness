import type { GithubChecksResult, GithubPullRequestsResult } from "./protocol/generated/protocol";

// Pure logic for the GitHub surface: rollup classification, CI-finished
// notification rendering, and the jump-to-diff fallback decision. Kept out of
// the components so the routing rules are assertable without a DOM.

export type PullRequestListItem = GithubPullRequestsResult["pullRequests"][number];

export type RollupState = "failing" | "running" | "passing" | "none";

export function rollupState(pr: Pick<PullRequestListItem, "checks">): RollupState {
  if (pr.checks.failed) return "failing";
  if (pr.checks.queued || pr.checks.inProgress) return "running";
  if (pr.checks.passed) return "passing";
  return "none";
}

export function checksNeedPolling(checks: GithubChecksResult["checks"]): boolean {
  return checks.some(check => check.status === "queued" || check.status === "inProgress");
}

/** The `github/ci_finished` wire payload. */
export type GithubCiFinishedPayload = {
  workspaceId: string;
  number: number;
  headBranch: string;
  title: string;
  failed: number;
  total: number;
};

/** One toast per terminal check set: the poller dedups server-side, and this
 * key keeps a re-delivered payload (reconnect, mock replays) from stacking a
 * duplicate client-side. */
export function ciToastKey(payload: GithubCiFinishedPayload): string {
  return `${payload.workspaceId}#${payload.number}:${payload.failed}/${payload.total}`;
}

export function ciNotificationText(payload: GithubCiFinishedPayload): {
  headline: string;
  detail: string;
  tone: "success" | "failure";
} {
  const failed = payload.failed > 0;
  return {
    headline: failed
      ? `CI failed on ${payload.headBranch} — ${payload.failed} check${payload.failed === 1 ? "" : "s"}`
      : `CI passed on ${payload.headBranch}`,
    detail: `#${payload.number} ${payload.title}`,
    tone: failed ? "failure" : "success",
  };
}

/** Jump-to-diff fallback: null when the PR head branch is what this workspace
 * has checked out (the jump lands on the right tree), otherwise the hint the
 * UI shows beside the read-only open. */
export function jumpFallbackHint(
  workspaceBranch: string | null | undefined,
  headBranch: string,
): string | null {
  if (workspaceBranch === headBranch) return null;
  const shown = workspaceBranch ? `showing the file on ${workspaceBranch}` : "showing the file from this workspace";
  return `${headBranch} isn’t checked out here — ${shown}.`;
}

// ── List filtering ──────────────────────────────────────────────────────────
// The pane reads whatever `gh` returns for the repository, which on a busy
// repository is more rows than anybody scans by eye. Filtering is client-side
// and free: the list is already in memory, and a server round-trip per
// keystroke would make the pane feel worse, not better.

/** The facet chips above the pull-request list. */
export type PullRequestFacet = "all" | "ready" | "draft" | "failing" | "approved";

export const PULL_REQUEST_FACETS: ReadonlyArray<{ id: PullRequestFacet; label: string }> = [
  { id: "all", label: "All" },
  { id: "ready", label: "Ready" },
  { id: "draft", label: "Draft" },
  { id: "failing", label: "Failing" },
  { id: "approved", label: "Approved" },
];

function matchesFacet(pr: PullRequestListItem, facet: PullRequestFacet): boolean {
  switch (facet) {
    case "ready": return !pr.isDraft;
    case "draft": return pr.isDraft;
    case "failing": return rollupState(pr) === "failing";
    case "approved": return pr.reviewDecision === "approved";
    default: return true;
  }
}

/** Case-insensitive substring match over the fields a reader would search by.
 * `#341` and `341` both find PR 341. */
function matchesQuery(fields: Array<string | null | undefined>, query: string): boolean {
  const needle = query.trim().toLowerCase().replace(/^#/, "");
  if (!needle) return true;
  return fields.some(field => field?.toLowerCase().includes(needle));
}

export function filterPullRequests(
  pullRequests: PullRequestListItem[],
  query: string,
  facet: PullRequestFacet,
): PullRequestListItem[] {
  return pullRequests.filter(pr =>
    matchesFacet(pr, facet)
    && matchesQuery([String(pr.number), pr.title, pr.headBranch, pr.author?.login], query));
}

export function filterIssues<T extends {
  number: number;
  title: string;
  author?: { login: string } | null;
  labels: Array<{ name: string }>;
}>(issues: T[], query: string): T[] {
  return issues.filter(issue => matchesQuery(
    [String(issue.number), issue.title, issue.author?.login, ...issue.labels.map(label => label.name)],
    query,
  ));
}

/** Checks grouped by the workflow that produced them, in first-seen order —
 * a flat list of forty rows named "test (18)" tells a reader nothing about
 * which workflow is red. */
export function groupChecksByWorkflow(
  checks: GithubChecksResult["checks"],
): Array<{ workflow: string; checks: GithubChecksResult["checks"] }> {
  const groups = new Map<string, GithubChecksResult["checks"]>();
  for (const check of checks) {
    const key = check.workflow || "Checks";
    const existing = groups.get(key);
    if (existing) existing.push(check);
    else groups.set(key, [check]);
  }
  return [...groups].map(([workflow, grouped]) => ({ workflow, checks: grouped }));
}
