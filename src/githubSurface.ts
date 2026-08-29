import type { GithubPullRequestsResult } from "./protocol/generated/protocol";

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
