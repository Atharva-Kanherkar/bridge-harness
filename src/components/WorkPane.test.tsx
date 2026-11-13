// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MotionGlobalConfig } from "framer-motion";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PullRequestCheck, SessionPullRequest } from "../protocol/generated/protocol";
import type { BackgroundTaskView } from "../backgroundTasks";
import { WorkPane, type WorkFocus } from "./WorkPane";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const check = (name: string, tone: "passed" | "failed" | "running", logUrl = ""): PullRequestCheck => ({
  name, workflow: "CI", logUrl,
  status: tone === "running" ? "inProgress" : "completed",
  conclusion: tone === "passed" ? "success" : tone === "failed" ? "failure" : null,
});
const pr = (number: number, details: PullRequestCheck[], over: Partial<SessionPullRequest> = {}): SessionPullRequest => ({
  number, title: `Title ${number}`, url: `https://github.com/o/r/pull/${number}`, state: "open", isDraft: false, headBranch: `feat/${number}`, headSha: "abc1234def",
  checks: {
    total: details.length, queued: 0, inProgress: details.filter(c => c.status === "inProgress").length,
    passed: details.filter(c => c.conclusion === "success").length, failed: details.filter(c => c.conclusion === "failure").length, skipped: 0, cancelled: 0,
  },
  checkDetails: details, attribution: "toolCompletion", attachedAt: new Date().toISOString(), fetchedAt: new Date().toISOString(), stale: false, error: null, ...over,
});
const running = pr(412, [check("test (rust)", "running"), check("build", "passed")]);
const failing = pr(413, [check("build", "passed"), check("lint", "failed", "https://ci.test/lint")]);
const merged = pr(409, [check("build", "passed")], { state: "merged" });

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  MotionGlobalConfig.skipAnimations = true;
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 0; });
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  MotionGlobalConfig.skipAnimations = false;
  vi.unstubAllGlobals();
});

type Props = Partial<Parameters<typeof WorkPane>[0]>;
const render = (props: Props = {}) => act(() => root.render(<WorkPane
  prs={[]} tasks={[]} refreshing={false} onAttach={async () => undefined} onRetry={() => undefined} {...props}
/>));
const prRows = () => [...host.querySelectorAll<HTMLElement>("[data-work-pr]")];
const rowButton = (number: number) => host.querySelector<HTMLButtonElement>(`[data-work-pr="${number}"] > button`)!;

describe("WorkPane", () => {
  it("says nothing is running and still offers attach", () => {
    render();
    expect(host.textContent).toContain("Nothing running");
    expect(host.textContent).toContain("Attach a pull request");
  });

  it("lists PRs worst first, one open at a time", () => {
    render({ prs: [merged, running, failing] });
    expect(prRows().map(row => row.dataset.workPr)).toEqual(["413", "412", "409"]);
    expect(rowButton(413).textContent).toContain("feat/413");
    expect(rowButton(413).textContent).toContain("abc1234");
    act(() => rowButton(413).click());
    expect(rowButton(413).getAttribute("aria-expanded")).toBe("true");
    act(() => rowButton(412).click());
    expect(rowButton(413).getAttribute("aria-expanded")).toBe("false");
    expect(host.querySelectorAll("[data-work-pr-detail]").length).toBe(1);
  });

  it("puts the failed check first and offers the fix", () => {
    const onFix = vi.fn();
    render({ prs: [failing], onFix, onOpenGithub: () => undefined });
    act(() => rowButton(413).click());
    const checks = [...host.querySelectorAll("[data-work-pr-detail] li")].map(li => li.textContent);
    expect(checks[0]).toContain("lint");
    expect(host.querySelector('a[aria-label="Logs for lint"]')!.getAttribute("href")).toBe("https://ci.test/lint");
    const fix = [...host.querySelectorAll("button")].find(button => button.textContent === "Fix the failing check")!;
    act(() => fix.click());
    expect(onFix).toHaveBeenCalledWith(failing);
    expect(host.textContent).toContain("Open in GitHub pane");
  });

  it("offers no fix on a PR that is not failing, and no GitHub pane without a repository", () => {
    render({ prs: [running], onFix: () => undefined });
    act(() => rowButton(412).click());
    expect(host.textContent).not.toContain("Fix the failing check");
    expect(host.textContent).not.toContain("Open in GitHub pane");
  });

  it("opens on the PR the shelf asked for, and again on a repeat request", () => {
    render({ prs: [failing, running], focus: { number: 412, nonce: 1 } });
    expect(rowButton(412).getAttribute("aria-expanded")).toBe("true");
    expect(document.activeElement).toBe(rowButton(412));
    act(() => rowButton(412).click());
    expect(rowButton(412).getAttribute("aria-expanded")).toBe("false");
    render({ prs: [failing, running], focus: { number: 412, nonce: 2 } satisfies WorkFocus });
    expect(rowButton(412).getAttribute("aria-expanded")).toBe("true");
  });

  it("labels a stale PR and retries", () => {
    const onRetry = vi.fn();
    render({ prs: [{ ...running, stale: true, error: "gh: offline" }], onRetry });
    act(() => rowButton(412).click());
    expect(host.textContent).toContain("last state Bridge saw");
    act(() => [...host.querySelectorAll("button")].find(button => button.textContent?.includes("Retry"))!.click());
    expect(onRetry).toHaveBeenCalled();
  });

  it("opens a background task onto its output and command", () => {
    const tasks: BackgroundTaskView[] = [
      { key: "a", kind: "running", label: "Install, build and test", command: "bun install > /tmp/a.log", elapsedMs: 21_000, outputTail: [], logFiles: ["a.log"] },
      { key: "b", kind: "watching", label: "Rust tests", command: "cargo test", elapsedMs: 5_000, outputTail: ["test a ... ok"], logFiles: [] },
    ];
    render({ tasks });
    const rows = [...host.querySelectorAll<HTMLButtonElement>("[data-work-task] > button")];
    expect(rows.map(row => row.textContent)).toEqual([expect.stringContaining("Install, build and test"), expect.stringContaining("Watching")]);
    act(() => rows[0].click());
    expect(host.querySelector("[data-work-task-output]")!.textContent).toContain("Output goes to a.log");
    act(() => rows[1].click());
    const second = host.querySelectorAll("[data-work-task]")[1].querySelector("[data-work-task-output]")!;
    expect(second.textContent).toContain("test a ... ok");
    expect(second.textContent).toContain("$ cargo test");
    expect(rows.map(row => row.getAttribute("aria-expanded"))).toEqual(["false", "true"]);
  });

  it("hands focus back on Escape, but not from the attach field", () => {
    const onEscape = vi.fn();
    render({ prs: [running], onEscape });
    act(() => { rowButton(412).dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); });
    expect(onEscape).toHaveBeenCalledTimes(1);
    act(() => [...host.querySelectorAll("button")].find(button => button.textContent?.includes("Attach a pull request"))!.click());
    act(() => { host.querySelector("input")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); });
    expect(onEscape).toHaveBeenCalledTimes(1);
  });
});
