// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MotionGlobalConfig } from "framer-motion";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PullRequestCheck, SessionPullRequest } from "../protocol/generated/protocol";
import { createBackgroundWorkStore, type BackgroundOutcome, type BackgroundTaskView, type BackgroundWorkStore } from "../backgroundTasks";
import { reduceConversation, type ConversationItem } from "../conversation";
import { asWireKind } from "../transcript/wire";
import type { AgentEvent } from "../types";
import { WorkShelf } from "./WorkShelf";
import { NOTICE_MS, useBackgroundTasks } from "./useBackgroundTasks";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const check = (name: string, tone: "passed" | "failed" | "running", logUrl = ""): PullRequestCheck => ({
  name, workflow: "CI", logUrl,
  status: tone === "running" ? "inProgress" : "completed",
  conclusion: tone === "passed" ? "success" : tone === "failed" ? "failure" : null,
});
const pr = (number: number, details: PullRequestCheck[], over: Partial<SessionPullRequest> = {}): SessionPullRequest => ({
  number, title: `Title ${number}`, url: `https://github.com/o/r/pull/${number}`, state: "open", isDraft: false, headBranch: "feat/x", headSha: "abc1234",
  checks: {
    total: details.length, queued: 0, inProgress: details.filter(c => c.status === "inProgress").length,
    passed: details.filter(c => c.conclusion === "success").length, failed: details.filter(c => c.conclusion === "failure").length, skipped: 0, cancelled: 0,
  },
  checkDetails: details, attribution: "toolCompletion", attachedAt: "2026-11-13T00:00:00Z", fetchedAt: "2026-11-13T00:00:00Z", stale: false, error: null, ...over,
});
const running = pr(412, [check("test (rust)", "running"), check("build", "passed")]);
const failing = pr(413, [check("build", "passed"), check("lint", "failed")]);
const passing = pr(414, [check("build", "passed")]);
const merged = pr(409, [check("build", "passed")], { state: "merged" });
const view = (key: string, kind: "running" | "watching" = "running", elapsedMs = 21_000): BackgroundTaskView =>
  ({ key, kind, label: `task ${key}`, elapsedMs, outputTail: [], logFiles: [] });

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  MotionGlobalConfig.skipAnimations = true;
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  MotionGlobalConfig.skipAnimations = false;
  vi.useRealTimers();
});

const shelf = () => host.querySelector<HTMLElement>("[data-work-shelf]");
const phrase = () => host.querySelector<HTMLElement>("[data-shelf-phrase]");
const render = (prs: SessionPullRequest[], tasks: BackgroundTaskView[] = [], handlers: { onOpen?: (n?: number) => void; onFix?: (p: SessionPullRequest) => void } = {}) =>
  act(() => root.render(<WorkShelf prs={prs} tasks={tasks} notice={null} harness="claude" onOpen={handlers.onOpen ?? (() => undefined)} onFix={handlers.onFix ?? (() => undefined)} />));

describe("WorkShelf", () => {
  it("is absent when nothing is going on", () => {
    render([]);
    expect(host.innerHTML).toBe("");
  });

  it("says one PR's live state in one line", () => {
    render([running]);
    expect(shelf()!.textContent).toContain("#412");
    expect(shelf()!.textContent).toContain("Title 412");
    expect(phrase()!.textContent).toBe("1 of 2 · test (rust) running");
    expect(phrase()!.className).toContain("text-muted-foreground");
    expect(host.querySelector("[data-work-shelf-fix]")).toBeNull();
  });

  it("names the failed check and offers a fix only then", () => {
    const onFix = vi.fn();
    render([failing], [], { onFix });
    expect(phrase()!.textContent).toBe("lint failed");
    expect(phrase()!.className).toContain("text-destructive");
    act(() => host.querySelector<HTMLButtonElement>("[data-work-shelf-fix]")!.click());
    expect(onFix).toHaveBeenCalledWith(failing);
  });

  it("stays quiet when green and says merged in info ink", () => {
    render([passing]);
    expect(phrase()!.textContent).toBe("Checks passed");
    expect(phrase()!.className).toContain("text-success");
    render([merged]);
    expect(phrase()!.textContent).toBe("Merged");
    expect(phrase()!.className).toContain("text-info");
  });

  it("collapses several PRs into counts, worst first", () => {
    render([merged, running, failing]);
    expect(shelf()!.textContent).toContain("3 pull requests");
    expect(phrase()!.textContent).toBe("1 failing · 1 running · 1 merged");
    expect(host.querySelectorAll("[data-pr-glyph], .lucide-circle-x, .lucide-git-merge").length).toBe(3);
    expect(host.querySelector("[data-work-shelf-fix]")).toBeNull();
  });

  it("names one background task and counts several", () => {
    render([], [view("a")]);
    expect(host.querySelector("[data-shelf-tasks]")!.textContent).toBe("task a21s");
    render([], [view("a", "running", 5_000), view("b", "watching", 134_000)]);
    expect(host.querySelector("[data-shelf-tasks]")!.textContent).toBe("1 running · 1 watching2m 14s");
  });

  it("puts PRs and tasks on the same fixed-height line", () => {
    render([failing], [view("a")]);
    expect(shelf()!.className).toContain("h-9");
    expect(phrase()).not.toBeNull();
    expect(host.querySelector("[data-shelf-tasks]")).not.toBeNull();
  });

  it("opens the Work pane on the lone PR, or on the list", () => {
    const onOpen = vi.fn();
    render([failing], [], { onOpen });
    act(() => host.querySelector<HTMLButtonElement>("[data-work-shelf-open]")!.click());
    expect(onOpen).toHaveBeenLastCalledWith(413);
    render([failing, running], [], { onOpen });
    act(() => host.querySelector<HTMLButtonElement>("[data-work-shelf-open]")!.click());
    expect(onOpen).toHaveBeenLastCalledWith(undefined);
    expect(host.querySelector("[data-work-shelf-open]")!.getAttribute("aria-label")).toBe("2 pull requests: 1 failing, 1 running. Open the Work pane.");
  });

  it("announces phase transitions, not repeat polls", () => {
    render([running]);
    const live = () => host.querySelector("[aria-live]")!.textContent;
    expect(live()).toBe("");
    render([{ ...running, fetchedAt: "2026-11-13T00:01:00Z" }]);
    expect(live()).toBe("");
    render([pr(412, [check("test (rust)", "failed"), check("build", "passed")])]);
    expect(live()).toBe("Pull request #412: 1 check failing.");
  });

  it("never floats: nothing sticky, fixed or absolute", () => {
    render([failing, running], [view("a")]);
    expect(host.innerHTML).not.toMatch(/\b(sticky|fixed|absolute)\b/);
  });
});

// ── the hook and the shelf together ─────────────────────────────────────────

const event = (id: number, kind: string, overrides: Partial<AgentEvent> = {}): AgentEvent => ({
  id, sessionId: "s", sequence: id, protocolVersion: 1, kind: asWireKind(kind), itemId: `i-${id}`,
  role: null, status: "completed", title: null, text: null, data: {}, providerMeta: {}, createdAt: "2026-11-12T09:55:46Z", ...overrides,
});
const taskItem = (itemId: string, title: string): ConversationItem => {
  const items = reduceConversation([event(1, "tool.started", { itemId, title, status: "inProgress", data: { type: "backgroundTask", backgroundTask: true, background: true, command: "cargo test" } })]);
  return items[items.length - 1];
};

function Harness({ scope, store }: { scope: string; store: BackgroundWorkStore }) {
  const { tasks, notice } = useBackgroundTasks(scope, store);
  return <WorkShelf prs={[]} tasks={tasks} notice={notice} onOpen={() => undefined} />;
}

describe("background work through the store", () => {
  const publish = (store: BackgroundWorkStore, scope: string, items: ConversationItem[], outcome?: BackgroundOutcome) =>
    act(() => store.publish({ scope, items, signature: items.map(item => item.identity).join(","), outcomeOf: () => outcome }));

  it("shows the open chat's tasks only", () => {
    const store = createBackgroundWorkStore();
    act(() => root.render(<Harness scope="mine" store={store} />));
    publish(store, "other", [taskItem("t", "Rust tests")]);
    expect(host.innerHTML).toBe("");
    publish(store, "mine", [taskItem("t", "Rust tests")]);
    expect(host.querySelector("[data-shelf-tasks]")!.textContent).toContain("Rust tests");
  });

  it("names a finished task for a moment, then goes", () => {
    vi.useFakeTimers();
    const store = createBackgroundWorkStore();
    act(() => root.render(<Harness scope="s" store={store} />));
    publish(store, "s", [taskItem("t", "Rust tests")]);
    publish(store, "s", [], "done");
    expect(host.querySelector("[data-shelf-notice]")!.textContent).toBe("Rust testsDone");
    act(() => { vi.advanceTimersByTime(NOTICE_MS); });
    expect(host.innerHTML).toBe("");
  });

  it("announces nothing when a task leaves without a reported outcome, or the chat changes", () => {
    const store = createBackgroundWorkStore();
    act(() => root.render(<Harness scope="s" store={store} />));
    publish(store, "s", [taskItem("t", "Rust tests")]);
    publish(store, "s", [], undefined);
    expect(host.innerHTML).toBe("");
    publish(store, "s", [taskItem("u", "Build")]);
    act(() => root.render(<Harness scope="other" store={store} />));
    publish(store, "other", [], "done");
    expect(host.innerHTML).toBe("");
  });
});
