// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../../api";
import type { ReviewJob, ReviewJobDetail } from "../../protocol/generated/protocol";
import { formatElapsed, ReviewJobs } from "./ReviewJobs";

let root: Root | undefined;
let host: HTMLDivElement | undefined;
const flush = async () => { for (let index = 0; index < 5; index += 1) await Promise.resolve(); };

export const reviewJob = (over: Partial<ReviewJob> = {}): ReviewJob => ({
  id: "job-1", workspaceId: "w", number: 7, headSha: "abc", status: "reviewing", trigger: "manual", postMode: "draft",
  harnesses: ["claude"], findingCounts: { total: 0, critical: 0, major: 0, minor: 0, nit: 0 },
  createdAt: "2026-11-24T10:00:00.000Z", startedAt: "2026-11-24T10:00:00.000Z", ...over,
});

const emptyDetail = (job: ReviewJob): ReviewJobDetail => ({ job, passes: [], findings: [] });

function listen() {
  const handlers: Array<(payload: { job: ReviewJob }) => void> = [];
  vi.spyOn(bridgeApi, "onReviewJobUpdated").mockImplementation(async handler => { handlers.push(handler); return () => {}; });
  return (job: ReviewJob) => act(() => { handlers.forEach(handler => handler({ job })); });
}

async function mount(element: React.ReactElement) {
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  await act(async () => { root!.render(element); await flush(); });
}

const row = (id: string) => host!.querySelector<HTMLElement>(`[data-review-job="${id}"]`);
const button = (scope: HTMLElement, name: string) => [...scope.querySelectorAll("button")].find(item => item.textContent?.includes(name))!;

afterEach(() => {
  act(() => root?.unmount());
  host?.remove();
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("ReviewJobs", () => {
  it("lists the PR's jobs and applies live updates, ignoring other PRs", async () => {
    const emit = listen();
    vi.spyOn(bridgeApi, "listReviews").mockResolvedValue({ jobs: [reviewJob({ id: "old", status: "posted", postedReviewUrl: "https://github.com/o/r/pull/7#pullrequestreview-1", createdAt: "2026-11-24T09:00:00.000Z", finishedAt: "2026-11-24T09:02:05.000Z", startedAt: "2026-11-24T09:00:00.000Z", findingCounts: { total: 2, critical: 1, major: 0, minor: 1, nit: 0 } })] });
    vi.spyOn(bridgeApi, "getReview").mockImplementation(async id => emptyDetail(reviewJob({ id, status: "drafted" })));
    await mount(<ReviewJobs workspaceId="w" number={7} />);
    const posted = row("old")!;
    expect(posted.textContent).toContain("Posted");
    expect(posted.textContent).toContain("2m 05s");
    expect(posted.textContent).toContain("1 critical");
    expect(posted.querySelector("a")!.getAttribute("href")).toBe("https://github.com/o/r/pull/7#pullrequestreview-1");

    // A job started elsewhere appears, newest first, and moves on as it runs.
    emit(reviewJob({ id: "new", status: "queued", createdAt: "2026-11-24T10:00:00.000Z" }));
    expect([...host!.querySelectorAll("[data-review-job]")].map(item => item.getAttribute("data-review-job"))).toEqual(["new", "old"]);
    expect(row("new")!.textContent).toContain("Queued");
    emit(reviewJob({ id: "new", status: "reviewing", harnesses: ["claude", "codex"] }));
    expect(row("new")!.textContent).toContain("Reviewing");
    expect(row("new")!.textContent).toContain("Codex");
    // Another PR's job, or another workspace's, never shows here.
    emit(reviewJob({ id: "elsewhere", number: 8 }));
    emit(reviewJob({ id: "other-workspace", workspaceId: "w2" }));
    expect(row("elsewhere")).toBeNull();
    expect(row("other-workspace")).toBeNull();
    // A draft opens its triage.
    await act(async () => { emit(reviewJob({ id: "new", status: "drafted" })); await flush(); });
    expect(host!.querySelector('[data-review-draft="new"]')).not.toBeNull();
  });

  it("cancels a running job, retries a failed one, and shows the reason", async () => {
    listen();
    vi.spyOn(bridgeApi, "listReviews").mockResolvedValue({ jobs: [
      reviewJob({ id: "running", status: "reviewing" }),
      reviewJob({ id: "failed", status: "failed", error: "claude could not review: signed out", createdAt: "2026-11-24T09:00:00.000Z" }),
    ] });
    const cancel = vi.spyOn(bridgeApi, "cancelReview").mockResolvedValue({ job: reviewJob({ id: "running", status: "cancelled" }) });
    const retry = vi.spyOn(bridgeApi, "retryReview").mockResolvedValue({ job: reviewJob({ id: "retry", status: "queued", retryOf: "failed", createdAt: "2026-11-24T11:00:00.000Z" }) });
    await mount(<ReviewJobs workspaceId="w" number={7} />);
    expect(row("failed")!.querySelector('[role="alert"]')!.textContent).toBe("claude could not review: signed out");
    expect(button(row("failed")!, "Cancel")).toBeUndefined();

    await act(async () => { button(row("running")!, "Cancel").click(); await flush(); });
    expect(cancel).toHaveBeenCalledWith("running");
    expect(row("running")!.textContent).toContain("Cancelled");
    expect(button(row("running")!, "Retry")).toBeDefined();

    await act(async () => { button(row("failed")!, "Retry").click(); await flush(); });
    expect(retry).toHaveBeenCalledWith("failed");
    expect(row("retry")!.textContent).toContain("Queued");
  });

  it("shows a job the pane just started, and ticks while it runs", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: false });
    vi.setSystemTime(new Date("2026-11-24T10:00:05.000Z"));
    listen();
    vi.spyOn(bridgeApi, "listReviews").mockResolvedValue({ jobs: [] });
    const element = (started?: ReviewJob) => <ReviewJobs workspaceId="w" number={7} started={started} />;
    await mount(element());
    expect(host!.textContent).toBe("");
    await act(async () => { root!.render(element(reviewJob({ id: "mine", status: "preparing" }))); await flush(); });
    expect(row("mine")!.textContent).toContain("5s");
    await act(async () => { vi.advanceTimersByTime(3000); await flush(); });
    expect(row("mine")!.textContent).toContain("8s");
  });

  it("formats elapsed time", () => {
    expect(formatElapsed(42_000)).toBe("42s");
    expect(formatElapsed(65_000)).toBe("1m 05s");
    expect(formatElapsed(3_780_000)).toBe("1h 03m");
  });
});
