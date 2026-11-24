// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import { forgetSurfaces, rememberSurface, surfaceSnapshot } from "../githubSurfaceCache";
import { GitHubPane } from "./GitHubPane";
import type { GithubPullRequestResult, GithubStatusResult } from "../protocol/generated/protocol";
import type { GithubLinkView } from "../githubLinks";

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

type Thread = GithubPullRequestResult["reviewThreads"][number];
const thread = (over: Partial<Thread> = {}): Thread => ({ ...detail.reviewThreads[0], viewerCanReply: true, ...over });
const withThreads = (reviewThreads: Thread[]): GithubPullRequestResult => ({ ...detail, reviewThreads });

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
const exactButton = (text: string) => [...document.body.querySelectorAll("button")].find(candidate => candidate.textContent?.trim() === text) as HTMLButtonElement;
/** The sheet's verb button (the one write the sheet gates). */
const confirmButton = () => document.querySelector<HTMLButtonElement>("button[data-confirm]")!;
const buttonByText = (text: string) => [...document.body.querySelectorAll("button")].find(candidate => candidate.textContent?.includes(text)) as HTMLButtonElement;

afterEach(async () => {
  await act(async () => { root?.unmount(); });
  host?.remove(); root = undefined; host = undefined; vi.restoreAllMocks();
  forgetSurfaces();
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
    expect(host!.textContent).toContain("Review threads");
    expect(host!.textContent).toContain("Main conversation comment");
    expect(host!.textContent).toContain("wants to merge 2 commits into main from feat/safe");
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
    expect(document.body.textContent).toContain("Add label “enhancement”");
    expect(document.body.textContent).toContain('gh pr edit 1 -R bridge/harness --add-label "enhancement"');
    expect(act).not.toHaveBeenCalled();
    await click(confirmButton());
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
    expect(host!.querySelector('[aria-label="File patch"]')).toBeNull();

    await click(buttonByText("src/file-0.ts"));
    const patch = host!.querySelector('[aria-label="File patch"]');
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
    expect(surfaceSnapshot("w")?.prs).toEqual([summary]);
  });

  it("shows the connecting skeleton while the status resolves, not a lone spinner", async () => {
    vi.spyOn(bridgeApi, "githubStatus").mockReturnValue(new Promise(() => undefined));
    await mount();
    const state = host!.querySelector('[role="status"][aria-label="Connecting to GitHub"]');
    expect(state).not.toBeNull();
    expect(state!.textContent).toContain("Resolving the CLI and repository.");
    // Structure, not a blank surface: placeholder rows ride beneath the notice.
    expect(state!.querySelectorAll("[data-skeleton-row]").length).toBeGreaterThan(0);
  });

  it("includes remembered rows in the initial render before effects run", () => {
    rememberSurface("w", { status, prs: [summary] });
    const html = renderToStaticMarkup(<GitHubPane workspaceId="w" workspaceBranch="main" onJumpToFile={() => {}} />);
    expect(html).toContain("Safe GitHub surface");
    expect(html).not.toContain('aria-label="Connecting to GitHub"');
  });

  it("paints remembered content immediately and revalidates in the background", async () => {
    rememberSurface("w", { status, prs: [summary] });
    // Neither read resolves: the first paint must come from the snapshot.
    vi.spyOn(bridgeApi, "githubStatus").mockReturnValue(new Promise(() => undefined));
    vi.spyOn(bridgeApi, "githubPullRequests").mockReturnValue(new Promise(() => undefined));
    await mount();
    expect(host!.textContent).toContain("Safe GitHub surface");
    expect(host!.querySelector('[aria-label="Connecting to GitHub"]')).toBeNull();
  });

  it("applies revalidated lists over remembered content", async () => {
    rememberSurface("w", { status, prs: [summary] });
    const fresh = { ...summary, number: 2, title: "Freshly revalidated" };
    vi.spyOn(bridgeApi, "githubStatus").mockResolvedValue(status);
    vi.spyOn(bridgeApi, "githubPullRequests").mockResolvedValue({ pullRequests: [fresh] });
    await mount();
    expect(host!.textContent).toContain("Freshly revalidated");
    expect(host!.textContent).not.toContain("Safe GitHub surface");
  });

  it("remembers real reads as each tab loads and reopens all tabs without fetching", async () => {
    mockReads();
    const issue = { number: 17, title: "Remembered issue", state: "open" as const, author: { login: "atharva" }, labels: [], createdAt: "now", updatedAt: "now", url: "https://example.test/issues/17" };
    const overview = { nameWithOwner: "bridge/harness", description: "Remembered overview", visibility: "PUBLIC", defaultBranch: "main", primaryLanguage: "Rust", url: "https://example.test/repo", openIssues: 1, openPullRequests: 1, labels: [] };
    let finishIssues!: (value: { issues: typeof issue[] }) => void;
    vi.spyOn(bridgeApi, "githubIssues").mockReturnValue(new Promise(resolve => { finishIssues = resolve; }));
    vi.spyOn(bridgeApi, "githubRepository").mockResolvedValue(overview);
    await mount();
    // One pending tab must not prevent the completed tabs from being cached.
    expect(surfaceSnapshot("w")?.prs).toEqual([summary]);
    expect(surfaceSnapshot("w")?.repositoryOverview).toEqual(overview);
    await act(async () => { finishIssues({ issues: [issue] }); await flush(); });
    await act(async () => { root?.unmount(); }); host?.remove();
    vi.mocked(bridgeApi.githubStatus).mockReturnValue(new Promise(() => {}));
    await mount();
    expect(host!.textContent).toContain("Safe GitHub surface");
    await click(host!.querySelector('button[aria-label="Issues"]') as HTMLButtonElement);
    expect(host!.textContent).toContain("Remembered issue");
    await click(host!.querySelector('button[aria-label="Repository"]') as HTMLButtonElement);
    expect(host!.textContent).toContain("Remembered overview");
  });

  it("clears old repository data when revalidation resolves a different repository", async () => {
    rememberSurface("w", { status, prs: [summary], issues: [], repositoryOverview: { url: "https://old.example.test" } as never });
    vi.spyOn(bridgeApi, "githubStatus").mockResolvedValue({ ...status, repository: { host: "github.com", owner: "bridge", name: "other" } });
    vi.spyOn(bridgeApi, "githubPullRequests").mockRejectedValue(new Error("new repository unavailable"));
    vi.spyOn(bridgeApi, "githubIssues").mockReturnValue(new Promise(() => {}));
    vi.spyOn(bridgeApi, "githubRepository").mockReturnValue(new Promise(() => {}));
    await mount();
    expect(host!.textContent).toContain("bridge/other");
    expect(host!.textContent).not.toContain("Safe GitHub surface");
    expect(surfaceSnapshot("w")?.prs).toBeUndefined();
    expect(surfaceSnapshot("w")?.repositoryOverview).toBeUndefined();
    expect(host!.querySelector('[aria-label="Open bridge/other on GitHub"]')?.getAttribute("href")).toBe("https://github.com/bridge/other");
  });

  it("forgets lists after sign-out even when the next sign-in list fails", async () => {
    rememberSurface("w", { status, prs: [summary] });
    const statusRead = vi.spyOn(bridgeApi, "githubStatus").mockResolvedValue({ availability: { status: "notAuthenticated", remediation: "gh auth login" }, repository: null });
    await mount();
    expect(surfaceSnapshot("w")?.prs).toBeUndefined();
    await act(async () => { root?.unmount(); }); host?.remove();
    statusRead.mockResolvedValue(status);
    vi.spyOn(bridgeApi, "githubPullRequests").mockRejectedValue(new Error("permission denied"));
    await mount();
    expect(host!.textContent).not.toContain("Safe GitHub surface");
  });

  it("ignores list reads from a load superseded by a signed-out refresh", async () => {
    const statusRead = vi.spyOn(bridgeApi, "githubStatus").mockResolvedValue(status);
    let finishPrs!: (value: { pullRequests: typeof summary[] }) => void;
    vi.spyOn(bridgeApi, "githubPullRequests").mockReturnValue(new Promise(resolve => { finishPrs = resolve; }));
    vi.spyOn(bridgeApi, "githubIssues").mockReturnValue(new Promise(() => {}));
    vi.spyOn(bridgeApi, "githubRepository").mockReturnValue(new Promise(() => {}));
    await mount();
    const signedOut: GithubStatusResult = { availability: { status: "notAuthenticated", remediation: "gh auth login" }, repository: null };
    statusRead.mockResolvedValue(signedOut);
    await click(host!.querySelector('button[aria-label="Refresh GitHub"]') as HTMLButtonElement);
    await act(async () => { finishPrs({ pullRequests: [summary] }); await flush(); });
    expect(surfaceSnapshot("w")).toEqual({ status: signedOut });
    expect(host!.textContent).toContain("Sign in to GitHub");
    await act(async () => { root?.unmount(); }); host?.remove();
    statusRead.mockReturnValue(new Promise(() => {}));
    await mount();
    expect(host!.textContent).toContain("Sign in to GitHub");
    expect(host!.textContent).not.toContain("Safe GitHub surface");
  });

  it("does not leak another workspace's remembered content", async () => {
    rememberSurface("other", { status, prs: [summary] });
    vi.spyOn(bridgeApi, "githubStatus").mockReturnValue(new Promise(() => undefined));
    await mount({ workspaceId: "w" });
    expect(host!.textContent).not.toContain("Safe GitHub surface");
    expect(host!.querySelector('[aria-label="Connecting to GitHub"]')).not.toBeNull();
  });

  it("renders explicit unavailable, sign-in, empty, and error states", async () => {
    const statusSpy = vi.spyOn(bridgeApi, "githubStatus");
    statusSpy.mockResolvedValue({ availability: { status: "notInstalled" }, repository: null });
    await mount();
    expect(host!.textContent).toContain("GitHub CLI is not installed");
    await act(async () => { root?.unmount(); }); host?.remove();

    statusSpy.mockResolvedValue({ availability: { status: "notAuthenticated", remediation: "gh auth login" }, repository: null });
    const login = vi.spyOn(bridgeApi, "startProviderLogin").mockResolvedValue({ workspaceId: "provider-login", terminalId: "github" });
    await mount();
    expect(host!.textContent).toContain("Sign in to GitHub");
    expect(host!.textContent).not.toContain("in a terminal");
    const signIn = [...host!.querySelectorAll("button")].find(button => button.textContent === "Sign in to GitHub") as HTMLButtonElement;
    await click(signIn);
    await act(async () => { await Promise.resolve(); });
    expect(login).toHaveBeenCalledWith("github");
    expect(host!.querySelector('[aria-label="GitHub sign-in"]')).not.toBeNull();
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

    // Check out rides the box's overflow menu now.
    await click(host!.querySelector('button[aria-label="More pull request actions"]') as HTMLButtonElement);
    await click(buttonByText("Check out"));
    expect(document.body.textContent).toContain("Check out #1 (feat/safe)");
    expect(document.body.textContent).toContain("your current checkout is untouched");
    await click(buttonByText("Cancel"));
    expect(checkout).not.toHaveBeenCalled();

    await click(host!.querySelector('button[aria-label="More pull request actions"]') as HTMLButtonElement);
    await click(buttonByText("Check out"));
    await click(confirmButton());
    expect(checkout).toHaveBeenCalledWith("w", 1);
    expect(host!.textContent).toContain("Checked out into a task worktree on ");
    expect(host!.textContent).toContain("/w/github/pr-1-feat-safe");

    checkout.mockResolvedValue({ workspaceId: "ws2", path: "/w/github/pr-1-feat-safe", branch: "feat/safe", reused: true });
    await click(host!.querySelector('button[aria-label="More pull request actions"]') as HTMLButtonElement);
    await click(buttonByText("Check out"));
    await click(confirmButton());
    expect(host!.textContent).toContain("Reusing the task worktree already on ");
  });

  it("starts a review job with the picked harness and shows it in Reviews", async () => {
    mockReads();
    vi.spyOn(bridgeApi, "getReviewJobSettings").mockResolvedValue({ secondOpinion: false });
    const legacy = vi.spyOn(bridgeApi, "githubReview");
    const start = vi.spyOn(bridgeApi, "startReview").mockResolvedValue({ job: {
      id: "job-1", workspaceId: "w", number: 1, headSha: "abc", status: "queued", trigger: "manual", postMode: "draft",
      harnesses: ["codex"], findingCounts: { total: 0, critical: 0, major: 0, minor: 0, nit: 0 }, createdAt: new Date().toISOString(),
    } });
    await mount({ sessionId: "sess-1" });
    await click(buttonByText("Safe GitHub surface"));

    // The button is present for an open PR; opening it reveals the harness picker.
    await click(buttonByText("Review"));
    expect(host!.textContent).toContain("RUN REVIEW WITH");
    expect(start).not.toHaveBeenCalled();
    // No harness needs an approval or a worktree of its own any more.
    expect(host!.textContent).not.toContain("isolated worktree");

    await click(buttonByText("Codex"));
    expect(start).toHaveBeenCalledWith({ workspaceId: "w", number: 1, harnesses: ["codex"] });
    expect(legacy).not.toHaveBeenCalled();
    const reviews = host!.querySelector('section[aria-label="Reviews"]')!;
    expect(reviews.textContent).toContain("Codex");
    expect(reviews.textContent).toContain("Queued");
    expect(host!.textContent).not.toContain("RUN REVIEW WITH");
  });

  it("with Second opinion on, starts one job with the two picked harnesses", async () => {
    mockReads();
    vi.spyOn(bridgeApi, "getReviewJobSettings").mockResolvedValue({ secondOpinion: true });
    const start = vi.spyOn(bridgeApi, "startReview").mockResolvedValue({ job: {
      id: "job-2", workspaceId: "w", number: 1, headSha: "abc", status: "queued", trigger: "manual", postMode: "draft",
      harnesses: ["claude", "codex"], findingCounts: { total: 0, critical: 0, major: 0, minor: 0, nit: 0 }, createdAt: new Date().toISOString(),
    } });
    await mount();
    await click(buttonByText("Safe GitHub surface"));
    await click(buttonByText("Review"));
    expect(host!.textContent).toContain("PICK TWO FOR A SECOND OPINION");
    const option = (label: string) => [...host!.querySelectorAll<HTMLButtonElement>('[role="menuitemcheckbox"]')].find(item => item.textContent?.includes(label))!;
    await click(option("Claude"));
    await click(option("Codex"));
    expect(option("Claude").getAttribute("aria-checked")).toBe("true");
    expect(option("OpenCode").disabled).toBe(true);
    expect(start).not.toHaveBeenCalled();
    await click(buttonByText("Review with Claude and Codex"));
    expect(start).toHaveBeenCalledWith({ workspaceId: "w", number: 1, harnesses: ["claude", "codex"] });
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

  it("surfaces a refused review start as an error notice", async () => {
    mockReads();
    vi.spyOn(bridgeApi, "getReviewJobSettings").mockResolvedValue({});
    vi.spyOn(bridgeApi, "startReview").mockRejectedValue(new Error("No harness is available to review with"));
    await mount({ sessionId: "sess-1" });
    await click(buttonByText("Safe GitHub surface"));
    await click(buttonByText("Review"));
    await click(buttonByText("Claude"));
    expect(host!.querySelector('[role="status"]')!.textContent).toContain("No harness is available to review with");
  });

  it("hides the checkout affordance when the PR head is already checked out here", async () => {
    mockReads();
    await mount({ workspaceBranch: "feat/safe" });
    await click(buttonByText("Safe GitHub surface"));
    expect(host!.textContent).toContain("Checked out here");
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
    expect(checks.textContent).toContain("Size");
    expect(checks.textContent).toContain("1/2 passed");
    // The old surface spun a loader on every unfinished row; the live hint is
    // now a breathe, and rotation is reserved for the explicit refresh.
    expect(checks.querySelector(".animate-spin")).toBeNull();
    expect(checks.querySelector(".github-check-live")).not.toBeNull();
  });

  it("opens the merge box confirm in place and merges through the typed action", async () => {
    mockReads();
    vi.spyOn(bridgeApi, "githubMergeConfig").mockResolvedValue({
      defaultStrategy: "squash" as const,
      strategies: { merge: true, squash: true, rebase: false },
    });
    const action = vi.spyOn(bridgeApi, "githubAct").mockResolvedValue({ executed: true, message: "Merge requested for PR #1 (squash)." });
    await mount();
    await click(buttonByText("Safe GitHub surface"));

    // The split button reads the repo default from the config read.
    expect(buttonByText("Squash and merge")).toBeTruthy();
    // Confirm expands in place — no dialog anywhere in the DOM.
    await click(buttonByText("Squash and merge"));
    expect(document.body.querySelector("[data-merge-go]")).toBeTruthy();
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    expect(document.body.textContent).toContain("Squashes 2 commits into one commit on main");

    // The commit subject is editable; the branch cleanup defaults to off.
    const subject = document.querySelector<HTMLInputElement>('input[aria-label="Commit message"]')!;
    expect(subject.value).toBe("Safe GitHub surface (#1)");

    await click(document.querySelector("button[data-merge-go]") as HTMLButtonElement);
    expect(action).toHaveBeenCalledTimes(1);
    expect(action).toHaveBeenCalledWith("w", {
      kind: "merge", number: 1, strategy: "squash", subject: "Safe GitHub surface (#1)", body: undefined, deleteBranch: undefined,
    }, true);
    // A second click during the flight cannot submit again.
    await click(document.querySelector("button[data-merge-go]") as HTMLButtonElement);
    expect(action).toHaveBeenCalledTimes(1);
  });

  it("does not merge when the chord confirms an action sheet over an open merge step", async () => {
    mockReads();
    vi.spyOn(bridgeApi, "githubMergeConfig").mockResolvedValue({
      defaultStrategy: "squash" as const,
      strategies: { merge: true, squash: true, rebase: false },
    });
    const action = vi.spyOn(bridgeApi, "githubAct").mockResolvedValue({ executed: true, message: "Done." });
    await mount();
    await click(buttonByText("Safe GitHub surface"));
    await click(buttonByText("Squash and merge"));
    expect(document.body.querySelector("[data-merge-go]")).toBeTruthy();
    await click(buttonByText("Manage labels"));
    await click(buttonByText("enhancement"));
    const confirm = confirmButton();
    await act(async () => {
      confirm.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", metaKey: true, bubbles: true, cancelable: true }));
    });
    expect(action).toHaveBeenCalledTimes(1);
    expect(action.mock.calls[0][1]).toMatchObject({ kind: "label" });
  });

  it("picking a strategy from the menu writes that strategy's commit subject", async () => {
    mockReads();
    vi.spyOn(bridgeApi, "githubMergeConfig").mockResolvedValue({
      defaultStrategy: "squash" as const,
      strategies: { merge: true, squash: true, rebase: false },
    });
    await mount();
    await click(buttonByText("Safe GitHub surface"));
    await click(document.querySelector('button[aria-label="Choose a merge strategy"]') as HTMLButtonElement);
    const item = [...document.querySelectorAll<HTMLButtonElement>('[role="menu"][aria-label="Merge strategy"] button, [role="menu"][aria-label="Merge strategy"] [role="menuitemradio"], [role="menu"][aria-label="Merge strategy"] [role="menuitem"]')]
      .find(button => button.textContent?.includes("Create a merge commit"))!;
    await click(item);
    const subject = document.querySelector<HTMLInputElement>('input[aria-label="Commit message"]')!;
    expect(subject.value).toMatch(/^Merge pull request #1 from /);
  });

  it("shows a refused merge verbatim with Try again and keeps the form", async () => {
    mockReads();
    vi.spyOn(bridgeApi, "githubMergeConfig").mockResolvedValue({
      defaultStrategy: "squash" as const,
      strategies: { merge: true, squash: true, rebase: false },
    });
    const action = vi.spyOn(bridgeApi, "githubAct").mockResolvedValue({ executed: false, message: 'Required status check "build" is expected.' });
    await mount();
    await click(buttonByText("Safe GitHub surface"));
    await click(buttonByText("Squash and merge"));
    await click(document.querySelector("button[data-merge-go]") as HTMLButtonElement);
    expect(document.body.textContent).toContain('Required status check "build" is expected.');
    expect(document.body.textContent).toContain("Try again");
    expect((document.querySelector<HTMLInputElement>('input[aria-label="Commit message"]')!).value).toBe("Safe GitHub surface (#1)");
    // The refusal tints the box and shows the reason inline, not a dialog.
    expect(document.querySelector('[role="dialog"]')).toBeNull();
  });

  it("offers only the strategies the repository allows", async () => {
    mockReads();
    vi.spyOn(bridgeApi, "githubMergeConfig").mockResolvedValue({
      defaultStrategy: "merge" as const,
      strategies: { merge: true, squash: false, rebase: false },
    });
    await mount();
    await click(buttonByText("Safe GitHub surface"));
    await click(document.querySelector('button[aria-label="Choose a merge strategy"]') as HTMLButtonElement);
    const menu = document.querySelector('[role="menu"][aria-label="Merge strategy"]')!;
    expect(menu.textContent).toContain("Create a merge commit");
    expect(menu.textContent).not.toContain("Squash and merge");
    expect(menu.textContent).not.toContain("Rebase and merge");
  });

  it("renders the merged receipt in its final state under reduced motion", async () => {
    mockReads();
    vi.spyOn(bridgeApi, "githubMergeConfig").mockResolvedValue({
      defaultStrategy: "squash" as const,
      strategies: { merge: true, squash: true, rebase: false },
    });
    const matchMedia = window.matchMedia;
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: (query: string) => ({
        matches: query.includes("prefers-reduced-motion"),
        media: query,
        onchange: null,
        addListener: () => undefined,
        removeListener: () => undefined,
        addEventListener: () => undefined,
        removeEventListener: () => undefined,
        dispatchEvent: () => false,
      }),
    });
    vi.spyOn(bridgeApi, "githubAct").mockResolvedValue({ executed: true, message: "Merge requested for PR #1 (squash)." });
    try {
      await mount();
      await click(buttonByText("Safe GitHub surface"));
      await click(buttonByText("Squash and merge"));
      await click(document.querySelector("button[data-merge-go]") as HTMLButtonElement);
      // Reduced motion: no choreography, the settled frame right away.
      expect(document.querySelector(".merge-graph-final")).toBeTruthy();
      expect(document.querySelector(".merge-graph-play")).toBeNull();
      expect(host!.textContent).toContain("Merged into main");
    } finally {
      Object.defineProperty(window, "matchMedia", { configurable: true, value: matchMedia });
    }
  });

  it("restores a deleted branch behind the confirmation gate", async () => {
    mockReads();
    vi.spyOn(bridgeApi, "githubMergeConfig").mockResolvedValue({
      defaultStrategy: "squash" as const,
      strategies: { merge: true, squash: true, rebase: false },
    });
    const action = vi.spyOn(bridgeApi, "githubAct")
      .mockResolvedValueOnce({ executed: true, message: "Merge requested for PR #1 (squash)." })
      .mockResolvedValueOnce({ executed: true, message: "Restored branch feat/safe." });
    await mount();
    await click(buttonByText("Safe GitHub surface"));
    await click(buttonByText("Squash and merge"));
    await click(document.querySelector("button[data-merge-go]") as HTMLButtonElement);
    // The receipt rises in; Restore branch rides the sheet like every write.
    expect(host!.textContent).toContain("Merged into main");
    await click(buttonByText("Restore branch"));
    expect(document.body.textContent).toContain("Recreates the deleted head branch");
    await click(confirmButton());
    expect(action).toHaveBeenLastCalledWith("w", { kind: "restoreBranch", number: 1, branch: "feat/safe" }, true);
  });

  it("posts a comment on its own button press and keeps the draft when GitHub refuses", async () => {
    mockReads();
    const action = vi.spyOn(bridgeApi, "githubAct").mockResolvedValueOnce({ executed: false, message: "GitHub said no." });
    await mount();
    await click(buttonByText("Safe GitHub surface"));

    const composer = () => host!.querySelector('textarea[aria-label="New comment"]') as HTMLTextAreaElement;
    await type(composer(), "This reads well now.");
    await click(exactButton("Comment"));
    // One press, one typed write, no second sheet.
    expect(action).toHaveBeenCalledWith("w", { kind: "comment", target: "pullRequest", number: 1, body: "This reads well now." }, true);
    expect(document.querySelector("button[data-confirm]")).toBeNull();
    // A refusal is shown where it was asked, and the words are still there.
    expect(host!.querySelector('[role="alert"]')?.textContent).toBe("GitHub said no.");
    expect(composer().value).toBe("This reads well now.");

    action.mockResolvedValueOnce({ executed: true, message: "Commented on PR #1." });
    await click(exactButton("Comment"));
    expect(action).toHaveBeenCalledTimes(2);
    expect(composer().value).toBe("");
  });

  it("resolves and reopens a conversation with one button, and undoes a refused resolve", async () => {
    mockReads();
    const read = vi.spyOn(bridgeApi, "githubPullRequest").mockResolvedValue(withThreads([thread({ viewerCanResolve: true })]));
    const action = vi.spyOn(bridgeApi, "githubAct").mockResolvedValueOnce({ executed: false, message: "Resource not accessible." });
    await mount();
    await click(buttonByText("Safe GitHub surface"));
    const card = () => host!.querySelector('[data-thread="t"]') as HTMLElement;

    await click(exactButton("Resolve"));
    expect(action).toHaveBeenCalledWith("w", { kind: "resolveThread", number: 1, threadId: "t", resolved: true }, true);
    // Refused: the card is back to open and says why.
    expect(card().hasAttribute("data-resolved")).toBe(false);
    expect(card().textContent).toContain("Resource not accessible.");

    action.mockResolvedValue({ executed: true, message: "Resolved a conversation on PR #1." });
    read.mockResolvedValue(withThreads([thread({ isResolved: true, viewerCanUnresolve: true })]));
    await click(exactButton("Resolve"));
    expect(card().hasAttribute("data-resolved")).toBe(true);
    expect(card().textContent).toContain("Resolved");
    // A resolved conversation folds to one line that still names its author.
    expect(card().querySelector('[aria-expanded="false"]')).not.toBeNull();
    expect(card().textContent).toContain("reviewer");

    read.mockResolvedValue(withThreads([thread({ viewerCanResolve: true })]));
    await click(exactButton("Unresolve"));
    expect(action).toHaveBeenLastCalledWith("w", { kind: "resolveThread", number: 1, threadId: "t", resolved: false }, true);
    expect(card().hasAttribute("data-resolved")).toBe(false);
  });

  it("offers Resolve only when GitHub says the viewer may", async () => {
    mockReads();
    vi.spyOn(bridgeApi, "githubPullRequest").mockResolvedValue(withThreads([thread({ viewerCanResolve: false })]));
    await mount();
    await click(buttonByText("Safe GitHub surface"));
    expect(exactButton("Resolve")).toBeUndefined();
  });

  it("replies inside a conversation without leaving it", async () => {
    mockReads();
    const action = vi.spyOn(bridgeApi, "githubAct").mockResolvedValue({ executed: true, message: "Replied on PR #1." });
    await mount();
    await click(buttonByText("Safe GitHub surface"));
    await click(exactButton("Reply…"));
    await type(host!.querySelector('textarea[aria-label="Reply"]') as HTMLTextAreaElement, "Fixed in the next push.");
    await click(exactButton("Reply"));
    expect(action).toHaveBeenCalledWith("w", { kind: "reply", number: 1, commentId: 9, body: "Fixed in the next push." }, true);
    // The box folds back to one line once the reply is in.
    expect(host!.querySelector('textarea[aria-label="Reply"]')).toBeNull();
  });

  it("filters conversations by open and resolved", async () => {
    mockReads();
    vi.spyOn(bridgeApi, "githubPullRequest").mockResolvedValue(withThreads([
      thread({ id: "open-one" }),
      thread({ id: "done-one", isResolved: true }),
    ]));
    await mount();
    await click(buttonByText("Safe GitHub surface"));
    const radio = (name: string) => [...host!.querySelectorAll<HTMLButtonElement>('[role="radio"]')].find(button => button.textContent?.startsWith(name))!;
    expect(radio("Open").textContent).toContain("1");
    await click(radio("Resolved"));
    expect(host!.querySelector('[data-thread="done-one"]')).not.toBeNull();
    expect(radio("Resolved").getAttribute("aria-checked")).toBe("true");
    await click(radio("Open"));
    expect(host!.querySelector('[data-thread="open-one"]')).not.toBeNull();
  });

  it("shows the lines a conversation is about", async () => {
    mockReads();
    vi.spyOn(bridgeApi, "githubPullRequest").mockResolvedValue(withThreads([thread({ diffHunk: "@@ -1,2 +1,3 @@\n fn main() {\n-    old();\n+    run();" })]));
    await mount();
    await click(buttonByText("Safe GitHub surface"));
    const hunk = host!.querySelector('[aria-label="Commented lines"]')!;
    expect(hunk.textContent).toContain("+    run();");
    expect(hunk.textContent).not.toContain("@@");
  });

  it("comments on a changed line with the file line and side GitHub expects", async () => {
    mockReads();
    vi.spyOn(bridgeApi, "githubPullRequest").mockResolvedValue({
      ...detail,
      files: [{ path: "src/api.ts", previousPath: null, status: "modified", additions: 1, deletions: 1, patch: "@@ -10,3 +10,3 @@\n keep\n-old\n+new\n tail" }],
    });
    const action = vi.spyOn(bridgeApi, "githubAct").mockResolvedValue({ executed: true, message: "Commented." });
    await mount();
    await click(buttonByText("Safe GitHub surface"));
    await click(host!.querySelector('button[role="tab"]:nth-of-type(2)') as HTMLButtonElement);

    // A hunk header is not a line, so it offers nothing.
    expect(host!.querySelector('button[aria-label="Comment on line 10"]')).not.toBeNull();
    expect(host!.querySelectorAll('button[aria-label^="Comment on line"]').length).toBe(4);

    // The added line is line 11 of the new file.
    const added = [...host!.querySelectorAll<HTMLButtonElement>('button[aria-label="Comment on line 11"]')];
    await click(added[1]);
    await type(host!.querySelector('textarea[aria-label="Comment on line 11"]') as HTMLTextAreaElement, "Why this?");
    await click(exactButton("Add comment"));
    expect(action).toHaveBeenCalledWith("w", { kind: "reviewComment", number: 1, path: "src/api.ts", line: 11, side: "right", body: "Why this?" }, true);
    expect(host!.querySelector('textarea[aria-label="Comment on line 11"]')).toBeNull();

    // The deleted line is line 11 of the old file.
    await click(added[0]);
    await type(host!.querySelector('textarea[aria-label="Comment on line 11"]') as HTMLTextAreaElement, "Keep it?");
    await click(exactButton("Add comment"));
    expect(action).toHaveBeenLastCalledWith("w", { kind: "reviewComment", number: 1, path: "src/api.ts", line: 11, side: "left", body: "Keep it?" }, true);
  });

  it("does not offer line comments on a pull request that is not open", async () => {
    mockReads();
    vi.spyOn(bridgeApi, "githubPullRequest").mockResolvedValue({ ...detail, pullRequest: { ...detail.pullRequest, summary: { ...summary, state: "merged" } } });
    await mount();
    await click(buttonByText("Safe GitHub surface"));
    await click(host!.querySelector('button[role="tab"]:nth-of-type(2)') as HTMLButtonElement);
    expect(host!.querySelector('[aria-label="File patch"]')).not.toBeNull();
    expect(host!.querySelector('button[aria-label^="Comment on line"]')).toBeNull();
  });

  it("shows images and videos from GitHub inline and holds third-party media behind a click", async () => {
    mockReads();
    vi.spyOn(bridgeApi, "githubPullRequest").mockResolvedValue({
      ...detail,
      pullRequest: {
        ...detail.pullRequest,
        body: [
          "Before and after:",
          "",
          "| Dark | Light |",
          "|---|---|",
          "| ![dark shot](https://raw.githubusercontent.com/o/r/a/dark.png) | ![light shot](https://raw.githubusercontent.com/o/r/a/light.png) |",
          "",
          "https://github.com/user-attachments/assets/11111111-2222-3333-4444-555555555555",
          "",
          '<img width="400" alt="tracker" src="https://tracker.example.test/pixel.png">',
          "",
          "```text",
          '<img src="https://github.com/in-a-fence.png">',
          "```",
        ].join("\n"),
      },
    });
    await mount();
    await click(buttonByText("Safe GitHub surface"));
    const description = host!.querySelector('[aria-label="Description"]')!;

    const images = [...description.querySelectorAll("img")];
    expect(images.map(image => image.getAttribute("src"))).toEqual([
      "https://raw.githubusercontent.com/o/r/a/dark.png",
      "https://raw.githubusercontent.com/o/r/a/light.png",
    ]);
    expect(images[0].getAttribute("alt")).toBe("dark shot");
    expect(images.every(image => image.getAttribute("referrerpolicy") === "no-referrer")).toBe(true);
    // The table's headings caption the pair.
    expect([...description.querySelectorAll("figcaption")].map(caption => caption.textContent)).toEqual(["Dark", "Light"]);
    // An uploaded recording is a player, not a bare address.
    expect(description.querySelector("video")?.getAttribute("src")).toBe("https://github.com/user-attachments/assets/11111111-2222-3333-4444-555555555555");
    // Someone else's server is not contacted until the reader asks.
    expect(description.innerHTML).not.toContain('src="https://tracker.example.test');
    await click(buttonByText("Load from tracker.example.test"));
    expect(description.querySelector('img[alt="tracker"]')?.getAttribute("src")).toBe("https://tracker.example.test/pixel.png");
    // Code stays code.
    expect(description.textContent).toContain('<img src="https://github.com/in-a-fence.png">');
  });

  it("falls back to the signed-in fetch when the webview cannot load GitHub media", async () => {
    mockReads();
    const created = vi.fn(() => "blob:bridge/1");
    vi.stubGlobal("URL", Object.assign(URL, { createObjectURL: created }));
    const media = vi.spyOn(bridgeApi, "githubMedia").mockResolvedValue({ mimeType: "image/png", data: btoa("png-bytes") });
    vi.spyOn(bridgeApi, "githubPullRequest").mockResolvedValue({
      ...detail,
      pullRequest: { ...detail.pullRequest, body: "![private shot](https://github.com/user-attachments/assets/aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee)" },
    });
    await mount();
    await click(buttonByText("Safe GitHub surface"));
    const image = () => host!.querySelector('[aria-label="Description"] img') as HTMLImageElement;
    await act(async () => { image().dispatchEvent(new Event("error")); await flush(); });
    expect(media).toHaveBeenCalledWith("w", "https://github.com/user-attachments/assets/aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee");
    expect(image().getAttribute("src")).toBe("blob:bridge/1");

    // The backend's copy failing too is the end of the road: say so, offer the original.
    await act(async () => { image().dispatchEvent(new Event("error")); await flush(); });
    expect(host!.textContent).toContain("Couldn’t load private shot");
    expect(media).toHaveBeenCalledTimes(1);
  });

  it("closes a pull request and marks a draft ready behind a confirmation", async () => {
    mockReads();
    const action = vi.spyOn(bridgeApi, "githubAct").mockResolvedValue({ executed: true, message: "Closed PR #1." });
    await mount();
    await click(buttonByText("Safe GitHub surface"));
    await click(host!.querySelector('button[aria-label="More pull request actions"]') as HTMLButtonElement);
    await click(buttonByText("Close"));
    expect(document.body.textContent).toContain("Close #1 without merging");
    await click(confirmButton());
    expect(action).toHaveBeenCalledWith("w", { kind: "setState", target: "pullRequest", number: 1, operation: "close" }, true);

    // A draft offers the undraft transition instead.
    await act(async () => { root?.unmount(); }); host?.remove();
    vi.spyOn(bridgeApi, "githubPullRequest").mockResolvedValue({
      ...detail,
      pullRequest: { ...detail.pullRequest, summary: { ...summary, isDraft: true } },
    });
    await mount();
    await click(buttonByText("Safe GitHub surface"));
    await click(host!.querySelector('button[aria-label="More pull request actions"]') as HTMLButtonElement);
    await click(buttonByText("Ready for review"));
    expect(document.body.textContent).toContain("Mark #1 ready for review");
    await click(confirmButton());
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
    expect(document.body.textContent).toContain("Close #17");
    expect(document.body.textContent).toContain("gh issue close 17 -R bridge/harness");
    await click(confirmButton());
    expect(action).toHaveBeenCalledWith("w", { kind: "setState", target: "issue", number: 17, operation: "close" }, true);

    const composer = host!.querySelector('textarea[aria-label="New comment"]') as HTMLTextAreaElement;
    await type(composer, "On it.");
    await click(exactButton("Comment"));
    // A comment posts on its own button, exactly as on a PR.
    expect(action).toHaveBeenLastCalledWith("w", { kind: "comment", target: "issue", number: 17, body: "On it." }, true);

    // A closed issue offers Reopen rather than Close.
    issueRead.mockResolvedValue({ issue: { summary: { ...issueSummary, state: "closed" }, body: "Body", comments: [] } });
    await click(buttonByText("All issues"));
    await click(buttonByText("Stay inside Bridge"));
    expect(buttonByText("Reopen")).toBeDefined();
  });

  // The bug this replaces: a folder with no GitHub remote failed all three
  // reads and the pane printed the raw resolution error three ways.
  it("offers Connect instead of a raw resolution error when no repository resolves", async () => {
    vi.spyOn(bridgeApi, "githubStatus").mockResolvedValue({ availability: { status: "available" }, repository: null });
    const list = vi.spyOn(bridgeApi, "githubPullRequests").mockRejectedValue(new Error("could not resolve a GitHub repository for /Users/a/animevocab: fatal: not a git repository"));
    const issues = vi.spyOn(bridgeApi, "githubIssues").mockRejectedValue(new Error("could not resolve a GitHub repository"));
    await mount();

    expect(host!.textContent).toContain("No GitHub repository connected");
    expect(host!.textContent).not.toContain("fatal: not a git repository");
    expect(host!.textContent).not.toContain("could not resolve");
    // The reads that can only fail are never made.
    expect(list).not.toHaveBeenCalled();
    expect(issues).not.toHaveBeenCalled();
  });

  // Daemon-host mode throws the JSON envelope, not an Error. The pane used to
  // print it verbatim — `{"code":1000,"kind":"invalid","message":"…"}`.
  it("shows the human message from a daemon-host error envelope, not the envelope", async () => {
    vi.spyOn(bridgeApi, "githubStatus").mockRejectedValue(
      '{"code":1000,"kind":"invalid","message":"GitHub CLI is unavailable"}',
    );
    await mount();

    expect(host!.textContent).toContain("GitHub CLI is unavailable");
    expect(host!.textContent).not.toContain('"code"');
    expect(host!.textContent).not.toContain("kind");
  });

  it("connects the workspace to a pasted repository URL and reloads the surface", async () => {
    const statusRead = vi.spyOn(bridgeApi, "githubStatus").mockResolvedValue({ availability: { status: "available" }, repository: null });
    vi.spyOn(bridgeApi, "githubPullRequests").mockResolvedValue({ pullRequests: [summary] });
    vi.spyOn(bridgeApi, "githubIssues").mockResolvedValue({ issues: [] });
    const connect = vi.spyOn(bridgeApi, "githubConnect").mockResolvedValue({
      repository: { host: "github.com", owner: "bridge", name: "harness" }, initialized: true, replacedRemote: false,
    });
    await mount();

    await click(buttonByText("Connect a repository"));
    const field = document.body.querySelector<HTMLInputElement>('input[aria-label="Repository search or URL"]')!;
    await type(field, "https://github.com/bridge/harness");
    statusRead.mockResolvedValue(status);
    await click(exactButton("Connect"));

    expect(connect).toHaveBeenCalledWith("w", "https://github.com/bridge/harness");
    expect(host!.textContent).toContain("Safe GitHub surface");
  });

  it("reports a failed repository search instead of claiming no matches", async () => {
    vi.useFakeTimers();
    vi.spyOn(bridgeApi, "githubStatus").mockResolvedValue({ availability: { status: "available" }, repository: null });
    const search = vi.spyOn(bridgeApi, "searchGithubRepos").mockRejectedValue(new Error("GitHub CLI unavailable: gh is not installed"));
    await mount();

    await click(buttonByText("Connect a repository"));
    const field = document.body.querySelector<HTMLInputElement>('input[aria-label="Repository search or URL"]')!;
    await type(field, "harness");
    await act(async () => { vi.advanceTimersByTime(300); await flush(); });

    expect(search).toHaveBeenCalledWith("harness");
    expect(document.body.textContent).toContain("GitHub CLI unavailable: gh is not installed");
    // "No repository matches that" would send the user hunting for a repo that exists.
    expect(document.body.textContent).not.toContain("No repository matches that");
  });

  it("keeps the connect failure on the dialog rather than dropping the user back to an error pane", async () => {
    vi.spyOn(bridgeApi, "githubStatus").mockResolvedValue({ availability: { status: "available" }, repository: null });
    vi.spyOn(bridgeApi, "githubConnect").mockRejectedValue(new Error("Enter a GitHub repository HTTPS or SSH URL"));
    await mount();

    await click(buttonByText("Connect a repository"));
    const field = document.body.querySelector<HTMLInputElement>('input[aria-label="Repository search or URL"]')!;
    await type(field, "https://github.com/bridge/harness");
    await click(exactButton("Connect"));

    expect(document.body.textContent).toContain("Enter a GitHub repository HTTPS or SSH URL");
    expect(document.body.querySelector('input[aria-label="Repository search or URL"]')).not.toBeNull();
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

describe("GitHubPane deep links", () => {
  const issues = () => {
    vi.spyOn(bridgeApi, "githubIssues").mockResolvedValue({ issues: [{
      number: 17, title: "Stay inside Bridge", state: "open", author: { login: "atharva" }, labels: [],
      createdAt: "now", updatedAt: "now", url: "https://example.test/issues/17",
    }] });
    return vi.spyOn(bridgeApi, "githubIssue").mockResolvedValue({ issue: {
      summary: { number: 17, title: "Stay inside Bridge", state: "open", author: { login: "atharva" }, labels: [], createdAt: "now", updatedAt: "now", url: "https://example.test/issues/17" },
      body: "Issue body", comments: [{ id: "i1", author: { login: "reviewer" }, body: "Issue comment", createdAt: "now", url: "https://example.test" }],
    } });
  };
  const overview = () => vi.spyOn(bridgeApi, "githubRepository").mockResolvedValue({ nameWithOwner: "bridge/harness", description: "Repository overview", visibility: "PRIVATE", defaultBranch: "main", primaryLanguage: "Rust", url: "https://example.test", openIssues: 1, openPullRequests: 2, labels: [] });
  const detailTabs = () => [...document.body.querySelectorAll<HTMLButtonElement>('[aria-label="Pull request detail"] button[role="tab"]')];
  const selectedDetailTab = () => detailTabs().find(tab => tab.getAttribute("aria-selected") === "true")?.textContent;
  const detailTab = (name: string) => detailTabs().find(tab => tab.textContent?.startsWith(name))!;

  it("opens the pull request a pull intent names", async () => {
    mockReads();
    await mount({ intent: { view: { kind: "pull", number: 1, tab: "conversation" }, nonce: 1 } });

    expect(host!.textContent).toContain("Main conversation comment");
    expect(selectedDetailTab()).toContain("conversation");
  });

  it("selects the sub-tab a pull intent names", async () => {
    mockReads();
    await mount({ intent: { view: { kind: "pull", number: 1, tab: "changes" }, nonce: 1 } });
    expect(selectedDetailTab()).toContain("changes");
    expect(host!.querySelector('section[aria-label="Changed files"]')).not.toBeNull();

    await act(async () => { root?.render(<GitHubPane workspaceId="w" workspaceBranch="main" onJumpToFile={() => undefined} intent={{ view: { kind: "pull", number: 1, tab: "checks" }, nonce: 2 }} />); await flush(); });
    expect(selectedDetailTab()).toContain("checks");
  });

  it("opens the issue an issue intent names", async () => {
    mockReads(); issues(); overview();
    await mount({ intent: { view: { kind: "issue", number: 17 }, nonce: 1 } });

    expect(host!.querySelector('button[aria-label="Issues"]')?.getAttribute("aria-pressed")).toBe("true");
    expect(host!.textContent).toContain("Issue comment");
  });

  it("shows a list or overview intent with nothing selected", async () => {
    mockReads(); issues(); overview();
    await mount({ intent: { view: { kind: "pull", number: 1, tab: "conversation" }, nonce: 1 } });
    expect(host!.textContent).toContain("Main conversation comment");

    const render = async (view: GithubLinkView, nonce: number) => {
      await act(async () => { root?.render(<GitHubPane workspaceId="w" workspaceBranch="main" onJumpToFile={() => undefined} intent={{ view, nonce }} />); await flush(); });
    };

    await render({ kind: "issues" }, 2);
    expect(host!.querySelector('button[aria-label="Issues"]')?.getAttribute("aria-pressed")).toBe("true");
    expect(host!.textContent).toContain("Stay inside Bridge");
    expect(host!.textContent).not.toContain("Issue comment");

    await render({ kind: "repository" }, 3);
    expect(host!.textContent).toContain("Repository overview");

    await render({ kind: "pulls" }, 4);
    expect(host!.querySelector('button[aria-label="Pull requests"]')?.getAttribute("aria-pressed")).toBe("true");
    // The pushed-out detail is still mid-exit when the flush resolves; let
    // the transition's frames run out under act before asserting it's gone.
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 450)); await flush(); });
    expect(host!.textContent).not.toContain("Main conversation comment");
  });

  it("re-fires on a new nonce and stays put on a re-render", async () => {
    mockReads();
    const detailRead = vi.spyOn(bridgeApi, "githubPullRequest");
    await mount({ intent: { view: { kind: "pull", number: 1, tab: "conversation" }, nonce: 1 } });
    const opened = detailRead.mock.calls.length;

    const render = async (nonce: number) => {
      await act(async () => { root?.render(<GitHubPane workspaceId="w" workspaceBranch="main" onJumpToFile={() => undefined} intent={{ view: { kind: "pull", number: 1, tab: "conversation" }, nonce }} />); await flush(); });
    };

    await render(1);
    expect(detailRead.mock.calls.length).toBe(opened);
    await render(2);
    expect(detailRead.mock.calls.length).toBe(opened + 1);
  });

  it("marks the affordances whose whole point is to leave the app", async () => {
    mockReads();
    // A patch past the pane's line cap is what puts the full-diff escape on
    // screen; the diff it points at is the one the pane just refused to show.
    vi.spyOn(bridgeApi, "githubPullRequest").mockResolvedValue({
      ...detail,
      files: [{ path: "src/api.ts", previousPath: null, status: "modified", additions: 700, deletions: 0, patch: ["@@ -1 +1 @@", ...Array.from({ length: 700 }, (_, line) => `+line ${line}`)].join("\n") }],
    });
    await mount();
    await click(buttonByText("Safe GitHub surface"));

    const marked = (selector: string) => host!.querySelector(selector)?.hasAttribute("data-system-browser");
    // Every "open on GitHub" affordance goes through one component, so the
    // rule is asserted over all of them rather than anchor by anchor.
    const allOpenOnGithubMarked = () => {
      const anchors = [...host!.querySelectorAll('a[aria-label$="on GitHub"]')];
      expect(anchors.length, "no open-on-GitHub affordance rendered").toBeGreaterThan(0);
      return anchors.every(anchor => anchor.hasAttribute("data-system-browser"));
    };
    expect(marked('a[aria-label="Open #1 on GitHub"]')).toBe(true);
    expect(allOpenOnGithubMarked()).toBe(true);

    await click(detailTab("changes"));
    expect(host!.textContent).toContain("view the full diff on GitHub");
    expect(marked('a[href="https://example.test/pr/1/files"]')).toBe(true);

    await click(detailTab("checks"));
    // A log URL is often shaped `/pull/<n>/checks`, which the pane would
    // otherwise swallow into a checks list holding no logs.
    expect(marked('a[aria-label="Open logs for build on GitHub"]')).toBe(true);
    expect(allOpenOnGithubMarked()).toBe(true);
  });
});
