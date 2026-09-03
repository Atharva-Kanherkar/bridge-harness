// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import { GitHubPane } from "./GitHubPane";
import type { GithubPullRequestResult, GithubStatusResult } from "../protocol/generated/protocol";

let root: Root | undefined;
let host: HTMLDivElement | undefined;
const flush = async () => { await Promise.resolve(); await Promise.resolve(); await Promise.resolve(); };

const status: GithubStatusResult = { availability: { status: "available" }, repository: { host: "github.com", owner: "bridge", name: "harness" } };
const summary = {
  number: 1, title: "Safe GitHub surface", state: "open" as const, isDraft: false,
  author: { login: "atharva" }, headBranch: "feat/safe", reviewDecision: "approved" as const,
  mergeability: "mergeable" as const, mergeStateStatus: "CLEAN",
  checks: { total: 2, queued: 0, inProgress: 0, passed: 1, failed: 1, skipped: 0, cancelled: 0 },
  url: "https://example.test/pr/1",
};
const detail: GithubPullRequestResult = {
  pullRequest: {
    summary, baseBranch: "main", body: "<script>window.pwned = true</script>",
    comments: [{ id: "conversation", author: { login: "maintainer" }, body: "Main conversation comment", createdAt: "now", url: "https://example.test" }],
    labels: [{ name: "bug", color: "d73a4a", description: "Broken" }],
    additions: 3, deletions: 1, changedFiles: 2,
  },
  reviewThreads: [{
    id: "t", isResolved: false, isOutdated: false, path: "src/api.ts", line: 42, originalLine: null,
    comments: [{ id: "c", databaseId: 9, author: { login: "reviewer" }, body: "<b>inert</b>", createdAt: "now", url: "https://example.test", replyToId: null }],
  }],
  files: [
    { path: "src/api.ts", previousPath: null, status: "modified", additions: 3, deletions: 1, patch: "@@ -1 +1 @@\n-old\n+new" },
    { path: "assets/icon.png", previousPath: null, status: "added", additions: 0, deletions: 0, patch: null },
  ],
};

function mockReads() {
  vi.spyOn(bridgeApi, "githubStatus").mockResolvedValue(status);
  const list = vi.spyOn(bridgeApi, "githubPullRequests").mockResolvedValue({ pullRequests: [summary] });
  vi.spyOn(bridgeApi, "githubPullRequest").mockResolvedValue(detail);
  vi.spyOn(bridgeApi, "githubChecks").mockResolvedValue({ checks: [
    { name: "build", status: "completed", conclusion: "failure", workflow: "CI", logUrl: "https://example.test/log" },
    { name: "test", status: "completed", conclusion: "success", workflow: "CI", logUrl: "" },
  ] });
  return list;
}

async function mount(props: Partial<Parameters<typeof GitHubPane>[0]> = {}) {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await act(async () => {
    root?.render(<GitHubPane workspaceId="w" workspaceBranch="main" onJumpToFile={() => undefined} {...props} />);
    await flush();
  });
}

const click = async (button: HTMLButtonElement) => { await act(async () => { button.click(); await flush(); }); };
const buttonByText = (text: string) => [...(host?.querySelectorAll("button") ?? [])].find(candidate => candidate.textContent?.includes(text)) as HTMLButtonElement;

afterEach(async () => {
  await act(async () => { root?.unmount(); });
  host?.remove(); root = undefined; host = undefined; vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("GitHubPane", () => {
  it("renders the PR list, opens the detail, and keeps remote HTML inert", async () => {
    mockReads();
    await mount();
    expect(host!.textContent).toContain("Safe GitHub surface");
    expect(host!.textContent).toContain("bridge/harness");
    expect(host!.textContent).toContain("feat/safe");
    expect(host!.textContent).toContain("1/2");

    await click(buttonByText("Safe GitHub surface"));
    expect(host!.textContent).toContain("REVIEW THREADS");
    expect(host!.textContent).toContain("Main conversation comment");
    expect(host!.textContent).toContain("feat/safe → main");
    expect(host!.querySelector("script")).toBeNull();
    expect(host!.innerHTML).toContain("&lt;script&gt;");

    await click(host!.querySelector('button[role="tab"]:nth-of-type(2)') as HTMLButtonElement);
    expect(host!.textContent).toContain("src/api.ts");
    expect(host!.textContent).toContain("Binary file or patch unavailable");

    // Back returns to the list.
    await click(buttonByText("All pull requests"));
    expect(host!.textContent).toContain("Safe GitHub surface");
    expect(host!.textContent).not.toContain("REVIEW THREADS");
  });

  it("switches between pull requests, issues, and the repository overview", async () => {
    mockReads();
    vi.spyOn(bridgeApi, "githubIssues").mockResolvedValue({ issues: [{
      number: 17, title: "Stay inside Bridge", state: "open", author: { login: "atharva" }, labels: [],
      createdAt: "now", updatedAt: "now", url: "https://example.test/issues/17",
    }] });
    vi.spyOn(bridgeApi, "githubIssue").mockResolvedValue({ issue: {
      summary: { number: 17, title: "Stay inside Bridge", state: "open", author: { login: "atharva" }, labels: [], createdAt: "now", updatedAt: "now", url: "https://example.test/issues/17" },
      body: "Issue body <script>inert</script>", comments: [{ id: "i1", author: { login: "reviewer" }, body: "Issue comment", createdAt: "now", url: "https://example.test" }],
    } });
    vi.spyOn(bridgeApi, "githubRepository").mockResolvedValue({ nameWithOwner: "bridge/harness", description: "Repository overview", visibility: "PRIVATE", defaultBranch: "main", primaryLanguage: "Rust", url: "https://example.test", openIssues: 1, openPullRequests: 2, labels: [] });
    await mount();

    await click(host!.querySelector('button[aria-label="Issues"]') as HTMLButtonElement);
    expect(host!.textContent).toContain("Stay inside Bridge");
    await click(buttonByText("Stay inside Bridge"));
    expect(host!.textContent).toContain("Issue comment");
    expect(host!.querySelector("script")).toBeNull();

    await click(host!.querySelector('button[aria-label="Repository"]') as HTMLButtonElement);
    expect(host!.textContent).toContain("Repository overview");
    expect(host!.textContent).toContain("Default branch");
  });

  it("confirms label changes before calling the typed action", async () => {
    mockReads();
    vi.spyOn(bridgeApi, "githubRepository").mockResolvedValue({ nameWithOwner: "bridge/harness", description: "", visibility: "PRIVATE", defaultBranch: "main", primaryLanguage: "Rust", url: "https://example.test", openIssues: 1, openPullRequests: 1, labels: [
      { name: "bug", color: "d73a4a", description: "Broken" },
      { name: "enhancement", color: "a2eeef", description: "New" },
    ] });
    const act = vi.spyOn(bridgeApi, "githubAct").mockResolvedValue({ executed: true, message: "Added" });
    await mount();
    await click(buttonByText("Safe GitHub surface"));
    await click(buttonByText("Manage labels"));
    await click(buttonByText("enhancement"));
    expect(host!.textContent).toContain('add label "enhancement" to PR #1');
    expect(act).not.toHaveBeenCalled();
    await click(buttonByText("Confirm"));
    expect(act).toHaveBeenCalledWith("w", { kind: "label", target: "pullRequest", number: 1, label: "enhancement", operation: "add" }, true);
  });

  it("polls running checks and stops after they become terminal", async () => {
    vi.useFakeTimers();
    const list = mockReads();
    const checks = vi.spyOn(bridgeApi, "githubChecks")
      .mockResolvedValueOnce({ checks: [{ name: "build", status: "inProgress", conclusion: null, workflow: "CI", logUrl: "" }] })
      .mockResolvedValue({ checks: [{ name: "build", status: "completed", conclusion: "success", workflow: "CI", logUrl: "" }] });
    await mount();
    await click(buttonByText("Safe GitHub surface"));
    const before = checks.mock.calls.length;
    const listBefore = list.mock.calls.length;
    await act(async () => { await vi.advanceTimersByTimeAsync(8_000); await flush(); });
    expect(checks.mock.calls.length).toBe(before + 1);
    await act(async () => { await vi.advanceTimersByTimeAsync(16_000); await flush(); });
    expect(checks.mock.calls.length).toBe(before + 1);
    // The timer reads one PR's checks only; the expensive list refetch is
    // reserved for checks-changed/ci-finished events.
    expect(list.mock.calls.length).toBe(listBefore);
  });

  it("collapses patches above the eager threshold and caps giant patches", async () => {
    mockReads();
    const bigPatch = Array.from({ length: 700 }, (_, index) => `+line ${index}`).join("\n");
    const files = Array.from({ length: 7 }, (_, index) => ({
      path: `src/file-${index}.ts`, previousPath: null, status: "modified",
      additions: 1, deletions: 0, patch: index === 0 ? bigPatch : "@@ -1 +1 @@\n+x",
    }));
    vi.spyOn(bridgeApi, "githubPullRequest").mockResolvedValue({
      ...detail,
      pullRequest: { ...detail.pullRequest, changedFiles: files.length },
      files,
    });
    await mount();
    await click(buttonByText("Safe GitHub surface"));
    await click(host!.querySelector('button[role="tab"]:nth-of-type(2)') as HTMLButtonElement);
    expect(host!.textContent).toContain("src/file-0.ts");
    expect(host!.querySelector('pre[aria-label="File patch"]')).toBeNull();

    await click(buttonByText("src/file-0.ts"));
    const patch = host!.querySelector('pre[aria-label="File patch"]');
    expect(patch).not.toBeNull();
    expect(host!.textContent).toContain("+line 599");
    expect(host!.textContent).not.toContain("+line 600");
    expect(host!.textContent).toContain("Patch truncated at 600 lines");
  });

  it("shows a list skeleton while pull requests load", async () => {
    vi.spyOn(bridgeApi, "githubStatus").mockResolvedValue(status);
    const never = new Promise<never>(() => {});
    vi.spyOn(bridgeApi, "githubPullRequests").mockReturnValue(never);
    vi.spyOn(bridgeApi, "githubIssues").mockReturnValue(never);
    vi.spyOn(bridgeApi, "githubRepository").mockReturnValue(never);
    await mount();
    expect(host!.querySelector('[role="status"][aria-label="Loading pull requests"]')).not.toBeNull();
  });

  it("renders pull requests without waiting for other tabs", async () => {
    vi.spyOn(bridgeApi, "githubStatus").mockResolvedValue(status);
    vi.spyOn(bridgeApi, "githubPullRequests").mockResolvedValue({ pullRequests: [summary] });
    const never = new Promise<never>(() => {});
    vi.spyOn(bridgeApi, "githubIssues").mockReturnValue(never);
    vi.spyOn(bridgeApi, "githubRepository").mockReturnValue(never);

    await mount();

    expect(host!.textContent).toContain("Safe GitHub surface");
    expect(host!.querySelector('[role="status"][aria-label="Loading pull requests"]')).toBeNull();
  });

  it("shows a detail skeleton while a pull request opens", async () => {
    mockReads();
    vi.spyOn(bridgeApi, "githubPullRequest").mockReturnValue(new Promise(() => {}));
    await mount();
    await click(buttonByText("Safe GitHub surface"));
    expect(host!.querySelector('[role="status"][aria-label="Opening pull request #1"]')).not.toBeNull();
  });

  it("renders bodies and comments as markdown with parseable timestamps only", async () => {
    mockReads();
    vi.spyOn(bridgeApi, "githubPullRequest").mockResolvedValue({
      ...detail,
      pullRequest: {
        ...detail.pullRequest,
        body: "Some **bold** intro\n\n```ts\nconst x = 1;\n```",
        comments: [
          { id: "c1", author: { login: "maintainer" }, body: "A `code` remark", createdAt: "2026-08-29T10:00:00Z", url: "https://example.test" },
          { id: "c2", author: { login: "other" }, body: "plain words", createdAt: "not-a-date", url: "https://example.test" },
        ],
      },
    });
    await mount();
    await click(buttonByText("Safe GitHub surface"));
    const description = host!.querySelector('[aria-label="Description"]')!;
    expect(description.querySelector("strong")?.textContent).toBe("bold");
    expect(description.querySelector(".code-block")).not.toBeNull();
    const comments = host!.querySelector('[aria-label="Conversation comments"]')!;
    expect(comments.querySelector("code")?.textContent).toBe("code");
    expect(comments.querySelectorAll("time")).toHaveLength(1);
  });

  it("keeps loaded content visible when a refresh fails", async () => {
    const list = mockReads();
    const statusRead = vi.mocked(bridgeApi.githubStatus);
    await mount();
    list.mockRejectedValueOnce(new Error("temporary GitHub outage"));
    await click(host!.querySelector('button[aria-label="Refresh GitHub"]') as HTMLButtonElement);
    expect(host!.textContent).toContain("Safe GitHub surface");
    expect(host!.textContent).toContain("Refresh failed: temporary GitHub outage");
    expect(statusRead).toHaveBeenLastCalledWith("w", true);
  });

  it("renders explicit unavailable, sign-in, empty, and error states", async () => {
    const statusSpy = vi.spyOn(bridgeApi, "githubStatus");
    statusSpy.mockResolvedValue({ availability: { status: "notInstalled" }, repository: null });
    await mount();
    expect(host!.textContent).toContain("GitHub CLI is not installed");
    await act(async () => { root?.unmount(); }); host?.remove();

    statusSpy.mockResolvedValue({ availability: { status: "notAuthenticated", remediation: "gh auth login" }, repository: null });
    await mount();
    expect(host!.textContent).toContain("gh auth login");
    await act(async () => { root?.unmount(); }); host?.remove();

    statusSpy.mockResolvedValue(status);
    vi.spyOn(bridgeApi, "githubPullRequests").mockResolvedValue({ pullRequests: [] });
    await mount();
    expect(host!.textContent).toContain("No open pull requests");
    await act(async () => { root?.unmount(); }); host?.remove();

    statusSpy.mockRejectedValue(new Error("gh exploded"));
    await mount();
    expect(host!.textContent).toContain("GitHub is unreachable");
    expect(host!.textContent).toContain("gh exploded");
  });

  it("jumps to the commented file and line through the callback", async () => {
    mockReads();
    const onJumpToFile = vi.fn();
    await mount({ onJumpToFile });
    await click(buttonByText("Safe GitHub surface"));
    await click(buttonByText("src/api.ts:42"));
    expect(onJumpToFile).toHaveBeenCalledWith("src/api.ts", 42, "feat/safe");
  });

  it("checks out behind a native confirmation and narrates fresh vs reused", async () => {
    mockReads();
    const checkout = vi.spyOn(bridgeApi, "githubCheckout").mockResolvedValue({ workspaceId: "ws2", path: "/w/github/pr-1-feat-safe", branch: "feat/safe", reused: false });
    await mount();
    await click(buttonByText("Safe GitHub surface"));

    // Declining runs nothing.
    await click(buttonByText("Check out"));
    expect(host!.textContent).toContain("check out PR #1 (feat/safe) into a task worktree");
    await click(buttonByText("Cancel"));
    expect(checkout).not.toHaveBeenCalled();

    await click(buttonByText("Check out"));
    await click([...host!.querySelectorAll("button")].filter(candidate => candidate.textContent === "Check out").at(-1) as HTMLButtonElement);
    expect(checkout).toHaveBeenCalledWith("w", 1);
    expect(host!.textContent).toContain("Checked out into a task worktree on ");
    expect(host!.textContent).toContain("/w/github/pr-1-feat-safe");

    checkout.mockResolvedValue({ workspaceId: "ws2", path: "/w/github/pr-1-feat-safe", branch: "feat/safe", reused: true });
    await click(buttonByText("Check out"));
    await click([...host!.querySelectorAll("button")].filter(candidate => candidate.textContent === "Check out").at(-1) as HTMLButtonElement);
    expect(host!.textContent).toContain("Reusing the task worktree already on ");
  });

  it("starts a subagent review with the picked harness and shows a success notice", async () => {
    mockReads();
    const review = vi.spyOn(bridgeApi, "githubReview").mockResolvedValue({
      status: "launched", sessionId: "child-1", message: "Review started with codex — comments will post to PR #1 shortly.",
    });
    await mount({ sessionId: "sess-1" });
    await click(buttonByText("Safe GitHub surface"));

    // The button is present for an open PR; opening it reveals the harness picker.
    await click(buttonByText("Review"));
    expect(host!.textContent).toContain("RUN REVIEW WITH");
    expect(review).not.toHaveBeenCalled();

    await click(buttonByText("Codex"));
    expect(review).toHaveBeenCalledWith("w", 1, "codex", "sess-1");
    expect(host!.textContent).toContain("Review started with codex");
  });

  it("surfaces a failed review launch as an error notice", async () => {
    mockReads();
    vi.spyOn(bridgeApi, "githubReview").mockResolvedValue({
      status: "failed", sessionId: null, message: "The review worker could not launch; the reason is on the conversation.",
    });
    await mount({ sessionId: "sess-1" });
    await click(buttonByText("Safe GitHub surface"));
    await click(buttonByText("Review"));
    await click(buttonByText("Claude"));
    expect(host!.textContent).toContain("could not launch");
  });

  it("hides the checkout affordance when the PR head is already checked out here", async () => {
    mockReads();
    await mount({ workspaceBranch: "feat/safe" });
    await click(buttonByText("Safe GitHub surface"));
    expect(host!.textContent).toContain("checked out here");
    expect(buttonByText("Check out")).toBeUndefined();
  });

  it("refetches the list when a CI-finished event lands for this workspace", async () => {
    const list = mockReads();
    let fire: ((payload: { workspaceId: string; number: number }) => void) | undefined;
    vi.spyOn(bridgeApi, "onGithubCiFinished").mockImplementation(async handler => { fire = handler as typeof fire; return () => undefined; });
    await mount();
    const before = list.mock.calls.length;
    await act(async () => { fire?.({ workspaceId: "other", number: 1 }); await flush(); });
    expect(list.mock.calls.length).toBe(before);
    await act(async () => { fire?.({ workspaceId: "w", number: 1 }); await flush(); });
    expect(list.mock.calls.length).toBe(before + 1);
  });
});
