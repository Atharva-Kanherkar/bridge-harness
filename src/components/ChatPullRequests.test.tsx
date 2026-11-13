// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import type { SessionPullRequest } from "../protocol/generated/protocol";
import { PullRequestGlyph, useChatPullRequests } from "./ChatPullRequests";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const pr = (over: Partial<SessionPullRequest> = {}): SessionPullRequest => ({
  number: 12, title: "Fix linking", url: "https://github.com/o/r/pull/12", state: "open", isDraft: false,
  headBranch: "feat/x", headSha: "abc1234",
  checks: { total: 2, queued: 0, inProgress: 1, passed: 1, failed: 0, skipped: 0, cancelled: 0 },
  checkDetails: [
    { name: "unit", status: "inProgress", conclusion: null, logUrl: "", workflow: "CI" },
    { name: "lint", status: "completed", conclusion: "success", logUrl: "https://example.test/log", workflow: "CI" },
  ],
  attribution: "toolCompletion", attachedAt: new Date().toISOString(), fetchedAt: new Date().toISOString(), stale: false, error: null, ...over,
});

let root: Root;
let host: HTMLElement;
beforeEach(() => { host = document.createElement("div"); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); vi.restoreAllMocks(); });

const flush = () => act(async () => { await Promise.resolve(); await Promise.resolve(); });

describe("PullRequestGlyph", () => {
  const draw = (value: SessionPullRequest) => act(() => root.render(<PullRequestGlyph pr={value} />));

  it("draws an open PR as a ring of its checks, the running arc breathing", () => {
    draw(pr());
    const svg = host.querySelector("svg[data-pr-glyph]")!;
    expect(svg.getAttribute("data-pr-glyph")).toBe("running");
    expect(svg.querySelectorAll("circle").length).toBe(3);
    expect(svg.querySelector(".github-check-live")).not.toBeNull();
  });

  it("draws a dashed track before any check reports", () => {
    draw(pr({ checks: { total: 0, queued: 0, inProgress: 0, passed: 0, failed: 0, skipped: 0, cancelled: 0 }, checkDetails: [] }));
    expect(host.querySelectorAll("circle").length).toBe(1);
    expect(host.querySelector("circle")!.getAttribute("class")).toContain("stroke-dasharray");
  });

  it("turns a failing PR into a red circle-x and settled PRs into their GitHub marks", () => {
    draw(pr({ checks: { total: 2, queued: 0, inProgress: 0, passed: 1, failed: 1, skipped: 0, cancelled: 0 } }));
    expect(host.querySelector(".lucide-circle-x")!.getAttribute("class")).toContain("text-destructive");
    draw(pr({ state: "merged" }));
    expect(host.querySelector(".lucide-git-merge")!.getAttribute("class")).toContain("text-info");
    draw(pr({ state: "closed" }));
    expect(host.querySelector(".lucide-git-pull-request-closed")).not.toBeNull();
  });
});

describe("useChatPullRequests", () => {
  function Probe({ sessionId }: { sessionId: string }) {
    const { prs } = useChatPullRequests(sessionId, "w1");
    return <p>{prs.map(value => value.number).join(",")}</p>;
  }

  it("reads per session and keeps chats isolated", async () => {
    const read = vi.spyOn(bridgeApi, "githubSessionPrs").mockImplementation(async sessionId => ({ pullRequests: sessionId === "a" ? [pr()] : [] }));
    act(() => root.render(<Probe sessionId="a" />));
    await flush();
    expect(host.textContent).toBe("12");
    expect(read).toHaveBeenCalledWith("a", true);
    act(() => root.render(<Probe sessionId="b" />));
    await flush();
    expect(host.textContent).toBe("");
  });

  it("refetches on a checks event without any transcript message", async () => {
    let listener: ((payload: { workspaceId: string; number: number }) => void) | undefined;
    vi.spyOn(bridgeApi, "onGithubChecksChanged").mockImplementation(async handler => { listener = handler; return () => undefined; });
    const read = vi.spyOn(bridgeApi, "githubSessionPrs")
      .mockResolvedValueOnce({ pullRequests: [pr()] })
      .mockResolvedValue({ pullRequests: [pr({ state: "merged" })] });
    act(() => root.render(<Probe sessionId="a" />));
    await flush();
    listener?.({ workspaceId: "other", number: 12 });
    await flush();
    expect(read).toHaveBeenCalledTimes(1);
    listener?.({ workspaceId: "w1", number: 12 });
    await flush();
    expect(read).toHaveBeenCalledTimes(2);
  });

  it("retains forced refresh when it queues behind an in-flight read", async () => {
    let resolveFirst!: (value: { pullRequests: SessionPullRequest[] }) => void;
    const read = vi.spyOn(bridgeApi, "githubSessionPrs")
      .mockImplementationOnce(() => new Promise(resolve => { resolveFirst = resolve; }))
      .mockResolvedValue({ pullRequests: [pr()] });
    let reload!: (refresh: boolean) => Promise<void>;
    function Holder() {
      const state = useChatPullRequests("a", "w1");
      reload = state.reload;
      return <p>{state.prs.length}</p>;
    }
    act(() => root.render(<Holder />));
    await act(async () => { await reload(true); await reload(false); });
    await act(async () => { resolveFirst({ pullRequests: [] }); });
    await flush();
    expect(read.mock.calls).toEqual([["a", true], ["a", true]]);
    expect(host.textContent).toBe("1");
  });

  it("forces discovery on focus even when the initial list was empty", async () => {
    const read = vi.spyOn(bridgeApi, "githubSessionPrs")
      .mockResolvedValueOnce({ pullRequests: [] })
      .mockResolvedValue({ pullRequests: [pr()] });
    act(() => root.render(<Probe sessionId="a" />));
    await flush();
    await act(async () => { window.dispatchEvent(new Event("focus")); });
    await flush();
    expect(read.mock.calls).toEqual([["a", true], ["a", true]]);
    expect(host.textContent).toBe("12");
  });

  it("keeps the last cards through a failed read", async () => {
    vi.spyOn(bridgeApi, "githubSessionPrs").mockResolvedValueOnce({ pullRequests: [pr()] }).mockRejectedValue(new Error("offline"));
    let reload: ((refresh: boolean) => Promise<void>) | undefined;
    function Holder() {
      const state = useChatPullRequests("a", "w1");
      reload = state.reload;
      return <p>{state.prs.map(value => value.number).join(",")}</p>;
    }
    act(() => root.render(<Holder />));
    await flush();
    await act(async () => { await reload?.(true); });
    expect(host.textContent).toBe("12");
  });
});
