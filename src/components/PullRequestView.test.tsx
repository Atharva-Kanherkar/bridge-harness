// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import { PullRequestView } from "./PullRequestView";
import type { GithubPullRequestResult } from "../protocol/generated/protocol";

let root: Root | undefined;
let host: HTMLDivElement | undefined;
const flush = async () => { await Promise.resolve(); await Promise.resolve(); };

const summary = { number: 5, title: "Slice 4", state: "open" as const, isDraft: false, author: { login: "atharva" }, headBranch: "feat/act", reviewDecision: "reviewRequired" as const, mergeability: "mergeable" as const, mergeStateStatus: "CLEAN", checks: { total: 1, queued: 0, inProgress: 0, passed: 0, failed: 1, skipped: 0, cancelled: 0 }, url: "https://example.test/pr/5" };
const result: GithubPullRequestResult = { pullRequest: { summary, body: "body", baseBranch: "main" }, reviewThreads: [{ id: "t1", isResolved: false, isOutdated: false, path: "src/api.ts", line: 3, originalLine: null, comments: [{ id: "c1", databaseId: 55, author: { login: "reviewer" }, body: "please fix", createdAt: "now", url: "https://example.test", replyToId: null }] }] };
const checks = { checks: [{ name: "test", status: "completed" as const, conclusion: "failure" as const, workflow: "CI", logUrl: "https://example.test/log" }] };
const repository = { host: "github.com", owner: "o", name: "r" };

const mount = async (onActed = () => {}) => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await act(async () => { root?.render(<PullRequestView workspaceId="w" repository={repository} result={result} checks={checks} onActed={onActed} onClose={() => {}} />); await flush(); });
};
const buttons = (scope: ParentNode) => [...scope.querySelectorAll("button")] as HTMLButtonElement[];
const clickText = async (scope: ParentNode, text: string) => {
  const button = buttons(scope).find(candidate => candidate.textContent === text);
  if (!button) throw new Error(`no button "${text}"`);
  await act(async () => { button.click(); await flush(); });
};
const dialog = () => host?.querySelector('[role="dialog"]') as HTMLElement;

afterEach(async () => {
  await act(async () => { root?.unmount(); });
  host?.remove(); root = undefined; host = undefined; vi.restoreAllMocks();
});

describe("PullRequestView actions", () => {
  it("merge dialog offers only allowed strategies, preselects the default, and sends the chosen one", async () => {
    vi.spyOn(bridgeApi, "githubMergeConfig").mockResolvedValue({ strategies: { merge: true, squash: true, rebase: false }, defaultStrategy: "squash" });
    const act$ = vi.spyOn(bridgeApi, "githubAct").mockResolvedValue({ executed: true, message: "ok" });
    await mount();
    await clickText(host!, "Merge");
    // Rebase is disallowed and absent; squash is the preselected default.
    expect(dialog().textContent).toContain("Squash and merge");
    expect(dialog().textContent).toContain("Merge commit");
    expect(dialog().textContent).not.toContain("Rebase and merge");
    expect((dialog().querySelector('input[value="squash"]') as HTMLInputElement).checked).toBe(true);
    // Pick a different strategy, then confirm.
    await act(async () => { (dialog().querySelector('input[value="merge"]') as HTMLInputElement).click(); await flush(); });
    await clickText(dialog(), "Merge");
    expect(act$).toHaveBeenCalledWith("w", { kind: "merge", number: 5, strategy: "merge" }, true);
  });

  it("cancel executes nothing", async () => {
    vi.spyOn(bridgeApi, "githubMergeConfig").mockResolvedValue({ strategies: { merge: true, squash: false, rebase: false }, defaultStrategy: "merge" });
    const act$ = vi.spyOn(bridgeApi, "githubAct").mockResolvedValue({ executed: true, message: "ok" });
    await mount();
    await clickText(host!, "Merge");
    await clickText(dialog(), "Cancel");
    expect(act$).not.toHaveBeenCalled();
    expect(host?.querySelector('[role="dialog"]')).toBeNull();
  });

  it("renders a gh refusal verbatim and does not report success", async () => {
    vi.spyOn(bridgeApi, "githubMergeConfig").mockResolvedValue({ strategies: { merge: true, squash: true, rebase: true }, defaultStrategy: "squash" });
    const refusal = "GitHub action failed: GraphQL: Branch protections: at least 1 approving review is required (mergePullRequest)";
    vi.spyOn(bridgeApi, "githubAct").mockRejectedValue(new Error(refusal));
    const onActed = vi.fn();
    await mount(onActed);
    await clickText(host!, "Merge");
    await clickText(dialog(), "Merge");
    expect(host?.textContent).toContain(refusal);
    expect(onActed).not.toHaveBeenCalled();
  });

  it("reply composer submits a reply action for the thread's comment", async () => {
    const act$ = vi.spyOn(bridgeApi, "githubAct").mockResolvedValue({ executed: true, message: "ok" });
    await mount();
    await clickText(host!, "Reply");
    const textarea = host?.querySelector("textarea") as HTMLTextAreaElement;
    const setValue = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, "value")!.set!;
    await act(async () => { setValue.call(textarea, "looks good"); textarea.dispatchEvent(new Event("input", { bubbles: true })); await flush(); });
    await clickText(dialog(), "Confirm");
    expect(act$).toHaveBeenCalledWith("w", { kind: "reply", number: 5, commentId: 55, body: "looks good" }, true);
  });

  it("approve submits a review action with an empty body", async () => {
    const act$ = vi.spyOn(bridgeApi, "githubAct").mockResolvedValue({ executed: true, message: "ok" });
    await mount();
    await clickText(host!, "Approve");
    // Approve requires no body — Confirm is immediately actionable.
    await clickText(dialog(), "Confirm");
    expect(act$).toHaveBeenCalledWith("w", { kind: "review", number: 5, event: "approve", body: "" }, true);
  });
});
