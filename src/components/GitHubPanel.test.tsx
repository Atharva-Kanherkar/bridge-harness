// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import { GitHubPanel } from "./GitHubPanel";

let root: Root | undefined;
let host: HTMLDivElement | undefined;
const flush = async () => { await Promise.resolve(); await Promise.resolve(); };
const status = { availability: { status: "available" as const }, repository: { host: "github.com", owner: "bridge", name: "harness" } };
const prs = { pullRequests: [{ number: 1, title: "Safe GitHub surface", state: "open" as const, isDraft: false, author: { login: "atharva" }, headBranch: "feat/safe", reviewDecision: "approved" as const, mergeability: "mergeable" as const, mergeStateStatus: "CLEAN", checks: { total: 1, queued: 0, inProgress: 0, passed: 1, failed: 0, skipped: 0, cancelled: 0 }, url: "https://example.test/pr/1" }] };

afterEach(async () => {
  await act(async () => { root?.unmount(); });
  host?.remove(); root = undefined; host = undefined; vi.restoreAllMocks();
});

describe("GitHubPanel", () => {
  it("renders fixture PR rollups and opens their read-only detail", async () => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    vi.spyOn(bridgeApi, "githubStatus").mockResolvedValue(status);
    vi.spyOn(bridgeApi, "githubPullRequests").mockResolvedValue(prs);
    vi.spyOn(bridgeApi, "githubPullRequest").mockResolvedValue({ pullRequest: { summary: prs.pullRequests[0], baseBranch: "main", body: "<script>window.pwned = true</script>" }, reviewThreads: [{ id: "t", isResolved: false, isOutdated: false, path: "src/api.ts", line: 1, originalLine: null, comments: [{ id: "c", databaseId: 1, author: { login: "reviewer" }, body: "<b>inert</b>", createdAt: "now", url: "https://example.test", replyToId: null }] }] });
    vi.spyOn(bridgeApi, "githubChecks").mockResolvedValue({ checks: [{ name: "test", status: "completed", conclusion: "success", workflow: "CI", logUrl: "https://example.test/log" }] });
    host = document.createElement("div"); document.body.append(host); root = createRoot(host);
    await act(async () => { root?.render(<GitHubPanel workspaceId="w" />); await flush(); });
    expect(host.textContent).toContain("Safe GitHub surface");
    expect(host.textContent).toContain("passing");
    await act(async () => { (host?.querySelector("button") as HTMLButtonElement).click(); await flush(); });
    expect(host.textContent).toContain("Review threads");
    expect(host.querySelector("script")).toBeNull();
    expect(host.innerHTML).toContain("&lt;script&gt;");
  });

  it("renders explicit unavailable and sign-in states", async () => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    const statusSpy = vi.spyOn(bridgeApi, "githubStatus");
    host = document.createElement("div"); document.body.append(host); root = createRoot(host);
    statusSpy.mockResolvedValue({ availability: { status: "notInstalled" }, repository: null });
    await act(async () => { root?.render(<GitHubPanel workspaceId="w" />); await flush(); });
    expect(host.textContent).toContain("GitHub CLI unavailable");
    statusSpy.mockResolvedValue({ availability: { status: "notAuthenticated", remediation: "gh auth login" }, repository: null });
    await act(async () => { root?.render(<GitHubPanel workspaceId="other" />); await flush(); });
    expect(host.textContent).toContain("gh auth login");
  });
});
