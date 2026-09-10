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
    commits: [
      { oid: "8f2a1c9d4e5b6a7c8d9e0f1a2b3c4d5e6f708192", abbreviatedOid: "8f2a1c9", messageHeadline: "Keep remote HTML inert", messageBody: "The renderer only emits text nodes.", committedAt: "2026-08-29T10:00:00Z", authors: [{ login: "atharva" }] },
      { oid: "1b0c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e", abbreviatedOid: "1b0c3d4", messageHeadline: "Add the pane skeleton", messageBody: "", committedAt: "not-a-date", authors: [] },
    ],
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
/** React tracks a controlled field's value on the DOM node, so a plain
 * assignment is invisible to it; go through the prototype setter. */
const type = async (field: HTMLInputElement | HTMLTextAreaElement, value: string) => {
  const prototype = field instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(prototype, "value")!.set!.call(field, value);
  await act(async () => { field.dispatchEvent(new Event("input", { bubbles: true })); await flush(); });
};
const buttonByText = (text: string) => [...document.body.querySelectorAll("button")].find(candidate => candidate.textContent?.includes(text)) as HTMLButtonElement;

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

    const tabs = [...host!.querySelectorAll<HTMLButtonElement>('[aria-label="Pull request detail"] [role="tab"]')];
    tabs[1].focus();
    await act(async () => { tabs[1].dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true })); });
    expect(tabs[2].getAttribute("aria-selected")).toBe("true");
    expect(document.activeElement).toBe(tabs[2]);
    expect(tabs.filter(tab => tab.tabIndex === 0)).toHaveLength(1);
    expect(host!.querySelector('[role="tabpanel"]')?.getAttribute("aria-labelledby")).toBe(tabs[2].id);
    await act(async () => { tabs[2].dispatchEvent(new KeyboardEvent("keydown", { key: "Home", bubbles: true })); });
    expect(tabs[0].getAttribute("aria-selected")).toBe("true");

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

  it("keeps header actions in one even toolbar", async () => {
    mockReads();
    vi.spyOn(bridgeApi, "githubIssues").mockResolvedValue({ issues: [] });
    vi.spyOn(bridgeApi, "githubRepository").mockResolvedValue({
      nameWithOwner: "bridge/harness", description: "Repository overview", visibility: "PRIVATE",
      defaultBranch: "main", primaryLanguage: "Rust", url: "https://example.test", openIssues: 1, openPullRequests: 2, labels: [],
    });
    await mount();
    const toolbar = host!.querySelector('[role="toolbar"][aria-label="GitHub repository actions"]')!;
    expect([...toolbar.children].map(node => node.getAttribute("aria-label"))).toEqual([
      "Pull requests",
      "Issues",
      "Repository",
      "Open bridge/harness on GitHub",
      "Refresh GitHub",
    ]);
    expect([...toolbar.children].every(node => node.className.includes("size-7"))).toBe(true);
    const copy = host!.querySelector('button[aria-label="Copy bridge/harness"]')!;
    expect(copy.className).toMatch(/\bmin-w-0\b/);
    expect(copy.className).toMatch(/\bshrink\b/);
    expect(copy.className).not.toMatch(/\bshrink-0\b/);
    expect(host!.querySelector("header")!.className).toContain("p-1.5");
    expect(host!.querySelector("header")!.className).not.toContain("u-glass");
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
    expect(document.body.textContent).toContain('add label "enhancement" to PR #1');
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
    expect(document.body.textContent).toContain("check out PR #1 (feat/safe) into a task worktree");
    await click(buttonByText("Cancel"));
    expect(checkout).not.toHaveBeenCalled();

    await click(buttonByText("Check out"));
    await click([...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button')].filter(candidate => candidate.textContent === "Check out").at(-1) as HTMLButtonElement);
    expect(checkout).toHaveBeenCalledWith("w", 1);
    expect(host!.textContent).toContain("Checked out into a task worktree on ");
    expect(host!.textContent).toContain("/w/github/pr-1-feat-safe");

    checkout.mockResolvedValue({ workspaceId: "ws2", path: "/w/github/pr-1-feat-safe", branch: "feat/safe", reused: true });
    await click(buttonByText("Check out"));
    await click([...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button')].filter(candidate => candidate.textContent === "Check out").at(-1) as HTMLButtonElement);
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

  it("asks Cursor Bugbot by posting the GitHub trigger comment", async () => {
    mockReads();
    const review = vi.spyOn(bridgeApi, "githubReview").mockResolvedValue({
      status: "launched", sessionId: null, message: "Asked Cursor Bugbot to review PR #1. It will comment on the pull request.",
    });
    await mount({ sessionId: "sess-1" });
    await click(buttonByText("Safe GitHub surface"));
    await click(buttonByText("Review"));
    expect(host!.textContent).toContain("Cursor Bugbot");
    await click(buttonByText("Cursor Bugbot"));
    expect(review).toHaveBeenCalledWith("w", 1, "bugbot", "sess-1");
    expect(host!.textContent).toContain("Asked Cursor Bugbot to review PR #1");
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

  it("lists the commits on the pull request with copyable SHAs", async () => {
    mockReads();
    await mount();
    await click(buttonByText("Safe GitHub surface"));

    const tabs = [...host!.querySelectorAll<HTMLButtonElement>('[aria-label="Pull request detail"] [role="tab"]')];
    expect(tabs.map(tab => tab.textContent)).toEqual(["conversation", "changes 2", "commits 2", "checks 2"]);
    await click(tabs[2]);

    const commits = host!.querySelector('[aria-label="Commits"]')!;
    expect(commits.textContent).toContain("Keep remote HTML inert");
    expect(commits.textContent).toContain("8f2a1c9");
    expect(commits.textContent).toContain("The renderer only emits text nodes.");
    // A junk commit date is omitted rather than printed raw, like every other
    // timestamp on this surface.
    expect(commits.querySelectorAll("time")).toHaveLength(1);

    const copy = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { value: { writeText: copy }, configurable: true });
    await click(commits.querySelector('button[aria-label="Copy the SHA 8f2a1c9"]') as HTMLButtonElement);
    expect(copy).toHaveBeenCalledWith("8f2a1c9d4e5b6a7c8d9e0f1a2b3c4d5e6f708192");
  });

  it("copies the pull request link and branch name out of the pane", async () => {
    mockReads();
    const copy = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { value: { writeText: copy }, configurable: true });
    await mount();

    // From the list row, without opening the pull request.
    await click(host!.querySelector('button[aria-label="Copy the link to #1"]') as HTMLButtonElement);
    expect(copy).toHaveBeenLastCalledWith("https://example.test/pr/1");

    await click(buttonByText("Safe GitHub surface"));
    await click(host!.querySelector('button[aria-label="Copy the branch name feat/safe"]') as HTMLButtonElement);
    expect(copy).toHaveBeenLastCalledWith("feat/safe");
    await click(host!.querySelector('button[aria-label="Copy bridge/harness"]') as HTMLButtonElement);
    expect(copy).toHaveBeenLastCalledWith("bridge/harness");
  });

  it("filters the pull request list by facet and by query", async () => {
    vi.spyOn(bridgeApi, "githubStatus").mockResolvedValue(status);
    vi.spyOn(bridgeApi, "githubPullRequests").mockResolvedValue({ pullRequests: [
      summary,
      { ...summary, number: 2, title: "Draft groundwork", isDraft: true, headBranch: "feat/draft", reviewDecision: "none" as const },
    ] });
    await mount();
    expect(host!.textContent).toContain("Draft groundwork");

    await click(buttonByText("Draft"));
    expect(host!.textContent).not.toContain("Safe GitHub surface");
    expect(host!.textContent).toContain("Draft groundwork");

    await click(buttonByText("All"));
    const search = host!.querySelector('input[aria-label="Filter pull requests"]') as HTMLInputElement;
    await type(search, "#1");
    expect(host!.textContent).toContain("Safe GitHub surface");
    expect(host!.textContent).not.toContain("Draft groundwork");

    await type(search, "nothing matches this");
    expect(host!.textContent).toContain("Nothing matches this filter");
    await click(buttonByText("Clear the filter"));
    expect(host!.textContent).toContain("Safe GitHub surface");
  });

  it("renders GitHub-flavoured markdown instead of printing its markup", async () => {
    mockReads();
    vi.spyOn(bridgeApi, "githubRepository").mockResolvedValue({ nameWithOwner: "bridge/harness", description: "", visibility: "PUBLIC", defaultBranch: "main", primaryLanguage: "Rust", url: "https://example.test/repo", openIssues: 0, openPullRequests: 1, labels: [] });
    vi.spyOn(bridgeApi, "githubPullRequest").mockResolvedValue({
      ...detail,
      pullRequest: {
        ...detail.pullRequest,
        body: [
          "<!-- template instructions -->",
          "Closes #412 for @atharva.",
          "",
          "- [x] done",
          "- [ ] pending",
          "",
          "<details><summary>CI log</summary>",
          "",
          "the collapsed log",
          "",
          "</details>",
        ].join("\n"),
      },
    });
    await mount();
    await click(buttonByText("Safe GitHub surface"));
    const description = host!.querySelector('[aria-label="Description"]')!;

    // Template comments are invisible on GitHub, so they are invisible here.
    expect(description.textContent).not.toContain("template instructions");
    // A cross-reference and a handle become real links — both on the
    // repository's own host, so an Enterprise mention does not land on a
    // stranger's github.com account.
    const links = [...description.querySelectorAll("a")].map(link => link.getAttribute("href"));
    expect(links).toContain("https://example.test/repo/issues/412");
    expect(links).toContain("https://example.test/atharva");
    expect(links.join(" ")).not.toContain("github.com");
    // Task lists read as statuses, not as literal brackets.
    expect(description.textContent).toContain("☑ done");
    expect(description.textContent).toContain("☐ pending");
    expect(description.textContent).not.toContain("[x]");
    // A <details> section is a real disclosure, not printed markup.
    const disclosure = description.querySelector("details");
    expect(disclosure?.querySelector("summary")?.textContent).toContain("CI log");
    expect(disclosure?.textContent).toContain("the collapsed log");
    expect(description.textContent).not.toContain("<summary>");
  });

  it("never turns a remote body into a link the webview would run in-app", async () => {
    mockReads();
    vi.spyOn(bridgeApi, "githubPullRequest").mockResolvedValue({
      ...detail,
      pullRequest: {
        ...detail.pullRequest,
        body: [
          '<a href="javascript:void%200">totally safe</a>',
          "",
          "[or this one](javascript:void%200)",
          "",
          '<img src="data:text/html,pwned">',
          "",
          "[real link](https://example.test/ok)",
        ].join("\n"),
      },
    });
    await mount();
    await click(buttonByText("Safe GitHub surface"));
    const description = host!.querySelector('[aria-label="Description"]')!;

    // Only the http(s) target — the one `externalLinks` would hand to the OS
    // browser — is an anchor at all.
    const hrefs = [...description.querySelectorAll("a")].map(link => link.getAttribute("href"));
    expect(hrefs).toEqual(["https://example.test/ok"]);
    expect(description.innerHTML).not.toContain("javascript:");
    expect(description.innerHTML).not.toContain("data:text/html");
    // The labels survive as text, so nothing silently disappears.
    expect(description.textContent).toContain("totally safe");
    expect(description.textContent).toContain("or this one");
  });

  it("groups checks by workflow and never spins a running check", async () => {
    mockReads();
    vi.spyOn(bridgeApi, "githubChecks").mockResolvedValue({ checks: [
      { name: "build", status: "completed", conclusion: "success", workflow: "CI", logUrl: "https://example.test/log" },
      { name: "bundle size", status: "inProgress", conclusion: null, workflow: "Size", logUrl: "" },
    ] });
    await mount();
    await click(buttonByText("Safe GitHub surface"));
    await click(host!.querySelector('[aria-label="Pull request detail"] [role="tab"]:nth-of-type(4)') as HTMLButtonElement);

    const checks = host!.querySelector('[aria-label="Checks"]')!;
    expect(checks.textContent).toContain("CI");
    expect(checks.textContent).toContain("SIZE");
    expect(checks.textContent).toContain("1/2 passed");
    // The old surface spun a loader on every unfinished row; the live hint is
    // now a breathe, and rotation is reserved for the explicit refresh.
    expect(checks.querySelector(".animate-spin")).toBeNull();
    expect(checks.querySelector(".github-check-live")).not.toBeNull();
  });

  it("posts a comment through the same confirmation gate as every other write", async () => {
    mockReads();
    const action = vi.spyOn(bridgeApi, "githubAct").mockResolvedValue({ executed: true, message: "Commented on PR #1." });
    await mount();
    await click(buttonByText("Safe GitHub surface"));

    const composer = host!.querySelector('textarea[aria-label="New comment"]') as HTMLTextAreaElement;
    await type(composer, "This reads well now.");
    await click(buttonByText("Comment"));

    // The draft is shown back, not re-requested, and nothing has run yet.
    expect(document.body.textContent).toContain("post a comment on PR #1");
    expect(document.body.textContent).toContain("This reads well now.");
    expect(action).not.toHaveBeenCalled();
    await click(buttonByText("Confirm"));
    expect(action).toHaveBeenCalledWith("w", { kind: "comment", target: "pullRequest", number: 1, body: "This reads well now." }, true);
  });

  it("closes a pull request and marks a draft ready behind a confirmation", async () => {
    mockReads();
    const action = vi.spyOn(bridgeApi, "githubAct").mockResolvedValue({ executed: true, message: "Closed PR #1." });
    await mount();
    await click(buttonByText("Safe GitHub surface"));
    await click(buttonByText("Close"));
    expect(document.body.textContent).toContain("close PR #1 on bridge/harness");
    await click(buttonByText("Confirm"));
    expect(action).toHaveBeenCalledWith("w", { kind: "setState", target: "pullRequest", number: 1, operation: "close" }, true);

    // A draft offers the undraft transition instead.
    await act(async () => { root?.unmount(); }); host?.remove();
    vi.spyOn(bridgeApi, "githubPullRequest").mockResolvedValue({
      ...detail,
      pullRequest: { ...detail.pullRequest, summary: { ...summary, isDraft: true } },
    });
    await mount();
    await click(buttonByText("Safe GitHub surface"));
    await click(buttonByText("Ready for review"));
    await click(buttonByText("Confirm"));
    expect(action).toHaveBeenLastCalledWith("w", { kind: "ready", number: 1 }, true);
  });

  it("closes and reopens an issue, and comments on it", async () => {
    mockReads();
    const issueSummary = {
      number: 17, title: "Stay inside Bridge", state: "open" as const, author: { login: "atharva" },
      labels: [], createdAt: "now", updatedAt: "now", url: "https://example.test/issues/17",
    };
    vi.spyOn(bridgeApi, "githubIssues").mockResolvedValue({ issues: [issueSummary] });
    const issueRead = vi.spyOn(bridgeApi, "githubIssue").mockResolvedValue({
      issue: { summary: issueSummary, body: "Body", comments: [] },
    });
    const action = vi.spyOn(bridgeApi, "githubAct").mockResolvedValue({ executed: true, message: "Closed issue #17." });
    await mount();
    await click(host!.querySelector('button[aria-label="Issues"]') as HTMLButtonElement);
    await click(buttonByText("Stay inside Bridge"));

    await click(buttonByText("Close"));
    expect(document.body.textContent).toContain("close issue #17 on bridge/harness");
    await click(buttonByText("Confirm"));
    expect(action).toHaveBeenCalledWith("w", { kind: "setState", target: "issue", number: 17, operation: "close" }, true);

    const composer = host!.querySelector('textarea[aria-label="New comment"]') as HTMLTextAreaElement;
    await type(composer, "On it.");
    await click(buttonByText("Comment"));
    // The drafted text is shown back before it posts, exactly as on a PR.
    expect(document.body.textContent).toContain("post a comment on issue #17");
    expect(document.querySelector('[role="dialog"] [aria-label="Comment body"]')?.textContent).toBe("On it.");
    await click(buttonByText("Confirm"));
    expect(action).toHaveBeenLastCalledWith("w", { kind: "comment", target: "issue", number: 17, body: "On it." }, true);

    // A closed issue offers Reopen rather than Close.
    issueRead.mockResolvedValue({ issue: { summary: { ...issueSummary, state: "closed" }, body: "Body", comments: [] } });
    await click(buttonByText("All issues"));
    await click(buttonByText("Stay inside Bridge"));
    expect(buttonByText("Reopen")).toBeDefined();
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
