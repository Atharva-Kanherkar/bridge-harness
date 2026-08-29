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
  pullRequest: { summary, baseBranch: "main", body: "<script>window.pwned = true</script>" },
  reviewThreads: [{
    id: "t", isResolved: false, isOutdated: false, path: "src/api.ts", line: 42, originalLine: null,
    comments: [{ id: "c", databaseId: 9, author: { login: "reviewer" }, body: "<b>inert</b>", createdAt: "now", url: "https://example.test", replyToId: null }],
  }],
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
    expect(host!.textContent).toContain("feat/safe → main");
    expect(host!.querySelector("script")).toBeNull();
    expect(host!.innerHTML).toContain("&lt;script&gt;");

    // Back returns to the list.
    await click(buttonByText("All pull requests"));
    expect(host!.textContent).toContain("Safe GitHub surface");
    expect(host!.textContent).not.toContain("REVIEW THREADS");
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
