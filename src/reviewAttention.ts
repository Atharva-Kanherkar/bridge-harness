import type { ReviewJob, ReviewJobStatus } from "./protocol/generated/protocol";
import { harnessLabel } from "./components/github/reviewLabels";
import { friendlyError } from "./errors";

// When a review job needs the person, or is done: a draft is ready, the review
// is posted, it failed, or a post went back to the draft with a reason. Reviews
// have no session, so the core attention ledger (which skips session-less
// rows) cannot carry this; the app follows `reviews/job_updated` itself.

export type ReviewNotice = { key: string; headline: string; detail: string; tone: "ready" | "done" | "failure" };

const harnesses = (job: ReviewJob) => job.harnesses.map(harnessLabel).join(" and ");
const findings = (count: number) => `${count} ${count === 1 ? "finding" : "findings"}`;

/**
 * The notice for a job whose status went from `previous` to `job.status`, or
 * null. Only a change of status counts: deciding a finding republishes a
 * drafted job, and that is not news. A job first seen already drafted (the
 * app opened after it drafted) is not news either.
 */
export function reviewNotice(previous: ReviewJobStatus | undefined, job: ReviewJob): ReviewNotice | null {
  if (previous === undefined || previous === job.status) return null;
  const pr = `PR #${job.number}`;
  switch (job.status) {
    case "drafted":
      if (previous === "posting") {
        return { key: `${job.id}:post-failed`, headline: `Posting the review on ${pr} failed`, detail: job.error ? friendlyError(job.error, "Open the draft to try again.") : "Open the draft to try again.", tone: "failure" };
      }
      // Auto mode passes through the draft on its way to posting.
      if (job.postMode === "auto") return null;
      return {
        key: `${job.id}:drafted`,
        headline: `Review ready on ${pr}`,
        detail: job.findingCounts.total === 0
          ? `${harnesses(job)} found nothing to flag`
          : `${findings(job.findingCounts.total)} to triage from ${harnesses(job)}`,
        tone: "ready",
      };
    case "posted":
      return { key: `${job.id}:posted`, headline: `Review posted on ${pr}`, detail: "Posted as one comment review", tone: "done" };
    case "failed":
      return { key: `${job.id}:failed`, headline: `Review failed on ${pr}`, detail: job.error ? friendlyError(job.error, "Open the pull request to retry.") : "Open the pull request to retry.", tone: "failure" };
    default:
      return null;
  }
}
