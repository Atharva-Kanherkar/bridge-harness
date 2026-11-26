import { describe, expect, it } from "vitest";
import type { ReviewJob } from "./protocol/generated/protocol";
import { reviewNotice } from "./reviewAttention";

const job = (over: Partial<ReviewJob> = {}): ReviewJob => ({
  id: "j", workspaceId: "w", number: 7, headSha: "abc", status: "drafted", trigger: "manual", postMode: "draft",
  harnesses: ["claude", "codex"], findingCounts: { total: 3, critical: 1, major: 2, minor: 0, nit: 0 }, createdAt: "now", ...over,
});

describe("reviewNotice", () => {
  it("names a ready draft, a posted review and a failure", () => {
    expect(reviewNotice("reviewing", job())).toEqual({ key: "j:drafted", headline: "Review ready on PR #7", detail: "3 findings to triage from Claude and Codex", tone: "ready" });
    expect(reviewNotice("reviewing", job({ findingCounts: { total: 0, critical: 0, major: 0, minor: 0, nit: 0 }, harnesses: ["opencode"] }))!.detail).toBe("OpenCode found nothing to flag");
    expect(reviewNotice("posting", job({ status: "posted" }))!.headline).toBe("Review posted on PR #7");
    expect(reviewNotice("preparing", job({ status: "failed", error: "Could not fetch pull request #7" }))).toMatchObject({ headline: "Review failed on PR #7", detail: "Could not fetch pull request #7.", tone: "failure" });
    expect(reviewNotice("posting", job({ error: "GitHub refused the review: HTTP 422" }))).toMatchObject({ key: "j:post-failed", headline: "Posting the review on PR #7 failed", detail: "GitHub refused the review.", tone: "failure" });
  });

  it("stays quiet when nothing changed, on first sight, and on auto mode's way to posting", () => {
    expect(reviewNotice("drafted", job()), "a decision republishes the draft").toBeNull();
    expect(reviewNotice(undefined, job()), "already drafted before the app saw it").toBeNull();
    expect(reviewNotice("reviewing", job({ postMode: "auto" }))).toBeNull();
    expect(reviewNotice("queued", job({ status: "preparing" }))).toBeNull();
    expect(reviewNotice("drafted", job({ status: "cancelled" }))).toBeNull();
  });
});
