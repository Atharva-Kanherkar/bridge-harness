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
const prs = { pullRequests: [
  { number: 1, title: "Safe GitHub surface", state: "open" as const, isDraft: false, author: { login: "atharva" }, headBranch: "feat/safe", reviewDecision: "approved" as const, mergeability: "mergeable" as const, mergeStateStatus: "CLEAN", checks: { total: 1, queued: 0, inProgress: 0, passed: 1, failed: 0, skipped: 0, cancelled: 0 }, url: "https://example.test/pr/1" },
  { number: 2, title: "Redder CI", state: "open" as const, isDraft: false, author: { login: "bridge" }, headBranch: "feat/red", reviewDecision: "none" as const, mergeability: "mergeable" as const, mergeStateStatus: "CLEAN", checks: { total: 2, queued: 0, inProgress: 0, passed: 1, failed: 1, skipped: 0, cancelled: 0 }, url: "https://example.test/pr/2" },
] };

afterEach(async () => {
  await act(async () => { root?.unmount(); });
  host?.remove(); root = undefined; host = undefined; vi.restoreAllMocks();
});

describe("GitHubPanel", () => {
  it("renders compact rollup rows and deep-links a row into the pane", async () => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    vi.spyOn(bridgeApi, "githubStatus").mockResolvedValue(status);
    vi.spyOn(bridgeApi, "githubPullRequests").mockResolvedValue(prs);
    const onOpen = vi.fn();
    host = document.createElement("div"); document.body.append(host); root = createRoot(host);
    await act(async () => { root?.render(<GitHubPanel workspaceId="w" onOpen={onOpen} />); await flush(); });
    expect(host.textContent).toContain("PULL REQUESTS");
    expect(host.textContent).toContain("Safe GitHub surface");
    expect(host.textContent).toContain("Redder CI");
    // The glance carries no detail overlay any more — the pane owns that.
    expect(host.textContent).not.toContain("Review threads");
    const row = [...host.querySelectorAll("button")].find(candidate => candidate.textContent?.includes("Redder CI"))!;
    await act(async () => { row.click(); });
    expect(onOpen).toHaveBeenCalledWith(2);
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

  it("stays silent when GitHub errors rather than wedging the rail", async () => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    vi.spyOn(bridgeApi, "githubStatus").mockRejectedValue(new Error("no network"));
    host = document.createElement("div"); document.body.append(host); root = createRoot(host);
    await act(async () => { root?.render(<GitHubPanel workspaceId="w" />); await flush(); });
    expect(host.textContent).toBe("");
  });
});
