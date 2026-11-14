import { describe, expect, it } from "vitest";
import type { PullRequestCheck, SessionPullRequest } from "./protocol/generated/protocol";
import type { BackgroundTaskView } from "./backgroundTasks";
import { fixPrompt, orderPullRequests, prCounts, shelfLabel, shelfPhrase, sidebarPrChip, taskSummary } from "./workSummary";

const check = (name: string, tone: "passed" | "failed" | "running" | "queued" | "skipped", logUrl = ""): PullRequestCheck => ({
  name, workflow: "CI", logUrl,
  status: tone === "running" ? "inProgress" : tone === "queued" ? "queued" : "completed",
  conclusion: tone === "passed" ? "success" : tone === "failed" ? "failure" : tone === "skipped" ? "skipped" : null,
});

/** Counts follow the details, the way the server builds them. */
const pr = (number: number, details: PullRequestCheck[], over: Partial<SessionPullRequest> = {}): SessionPullRequest => {
  const count = (status: string, conclusion?: string) => details.filter(c => c.status === status && (conclusion === undefined || c.conclusion === conclusion)).length;
  return {
    number, title: `PR ${number}`, url: `https://github.com/o/r/pull/${number}`, state: "open", isDraft: false,
    headBranch: "feat/x", headSha: "abc1234",
    checks: {
      total: details.length, queued: count("queued"), inProgress: count("inProgress"),
      passed: count("completed", "success"), failed: count("completed", "failure"), skipped: count("completed", "skipped"), cancelled: 0,
    },
    checkDetails: details, attribution: "toolCompletion", attachedAt: "2026-11-13T00:00:00Z", fetchedAt: "2026-11-13T00:00:00Z",
    stale: false, error: null, ...over,
  };
};

const running = pr(412, [check("test (rust)", "running"), check("build", "passed"), check("lint", "passed"), check("preview", "skipped")]);
const failing = pr(413, [check("build", "passed"), check("lint", "failed", "https://ci.test/lint")]);
const passing = pr(414, [check("build", "passed"), check("preview", "skipped")]);
const merged = pr(409, [check("build", "passed")], { state: "merged" });
const closed = pr(408, [], { state: "closed" });
const unchecked = pr(407, []);

const task = (key: string, kind: "running" | "watching", elapsedMs?: number): BackgroundTaskView =>
  ({ key, kind, label: `task ${key}`, elapsedMs, outputTail: [], logFiles: [] });

describe("sidebarPrChip", () => {
  it("is absent for a chat with no linked pull request", () => {
    expect(sidebarPrChip(undefined)).toBeNull();
    expect(sidebarPrChip([])).toBeNull();
  });

  it("names each state in a word or two", () => {
    const word = (p: SessionPullRequest) => { const chip = sidebarPrChip([p])!; return [chip.text, chip.tone]; };
    expect(word(merged)).toEqual(["merged", "info"]);
    expect(word(closed)).toEqual(["closed", "faint"]);
    expect(word(failing)).toEqual(["1 failing", "danger"]);
    expect(word(running)).toEqual(["CI running", "neutral"]);
    expect(word(passing)).toEqual(["passing", "success"]);
    expect(word(unchecked)).toEqual(["open", "faint"]);
    expect(word({ ...passing, isDraft: true })).toEqual(["draft", "faint"]);
    expect(word({ ...passing, mergeability: "conflicting" })).toEqual(["conflicts", "danger"]);
  });

  it("shows the most urgent PR, counts the rest, and lists all of them on hover", () => {
    const chip = sidebarPrChip([merged, { ...passing, mergeability: "conflicting" }, failing])!;
    expect(chip.pr.number).toBe(414);
    expect(chip.more).toBe(2);
    expect(chip.title.split("\n")).toEqual([
      "PR #414 · Merge conflicts",
      "PR #413 · 1 check failing",
      "PR #409 · Merged",
    ]);
  });

  it("marks a last-known snapshot in the hover", () => {
    expect(sidebarPrChip([{ ...merged, stale: true }])!.title).toBe("PR #409 · Merged (last known)");
  });
});

describe("orderPullRequests", () => {
  it("leads with what needs the person and keeps arrival order within a phase", () => {
    const second = pr(415, [check("lint", "failed")]);
    expect(orderPullRequests([merged, passing, closed, running, failing, unchecked, second]).map(p => p.number))
      .toEqual([413, 415, 412, 414, 407, 408, 409]);
  });
});

describe("shelfPhrase", () => {
  it("names the one check still running", () => {
    expect(shelfPhrase(running)).toEqual({ text: "3 of 4 · test (rust) running", tone: "neutral" });
  });

  it("counts when several checks run, and says queued when nothing has started", () => {
    expect(shelfPhrase(pr(1, [check("a", "running"), check("b", "running"), check("c", "passed")])).text).toBe("1 of 3 · running");
    expect(shelfPhrase(pr(1, [check("a", "queued"), check("b", "queued")]))).toEqual({ text: "2 queued", tone: "neutral" });
  });

  it("names a lone failure and counts several", () => {
    expect(shelfPhrase(failing)).toEqual({ text: "lint failed", tone: "danger" });
    expect(shelfPhrase(pr(1, [check("a", "failed"), check("b", "failed")])).text).toBe("2 checks failed");
  });

  it("words the settled states", () => {
    expect(shelfPhrase(passing)).toEqual({ text: "Checks passed", tone: "success" });
    expect(shelfPhrase(merged)).toEqual({ text: "Merged", tone: "info" });
    expect(shelfPhrase(closed)).toEqual({ text: "Closed", tone: "faint" });
    expect(shelfPhrase(unchecked)).toEqual({ text: "No checks", tone: "faint" });
    expect(shelfPhrase(pr(1, [check("a", "skipped")])).text).toBe("Checks skipped");
  });

  it("marks an open draft", () => {
    expect(shelfPhrase({ ...failing, isDraft: true }).text).toBe("Draft · lint failed");
    expect(shelfPhrase({ ...merged, isDraft: true }).text).toBe("Merged");
  });
});

describe("prCounts", () => {
  it("lists failing, running, passed, merged, closed and drops zeros", () => {
    expect(prCounts([merged, running, failing, closed, passing, unchecked])).toEqual([
      { count: 1, label: "failing", tone: "danger" },
      { count: 1, label: "running", tone: "neutral" },
      { count: 1, label: "passed", tone: "success" },
      { count: 1, label: "merged", tone: "info" },
      { count: 1, label: "closed", tone: "faint" },
    ]);
    expect(prCounts([running, running])).toEqual([{ count: 2, label: "running", tone: "neutral" }]);
  });
});

describe("taskSummary", () => {
  it("shows one task by name and several by count", () => {
    expect(taskSummary([])).toEqual({ kind: "none" });
    expect(taskSummary([task("a", "running", 21_000)])).toMatchObject({ kind: "one", task: { key: "a" } });
    expect(taskSummary([task("a", "running", 21_000), task("b", "running", 5_000)])).toMatchObject({ kind: "many", text: "2 running", oldestMs: 21_000 });
    expect(taskSummary([task("a", "running"), task("b", "watching", 134_000)])).toMatchObject({ kind: "many", text: "1 running · 1 watching", oldestMs: 134_000 });
    expect(taskSummary([task("a", "watching"), task("b", "watching")])).toMatchObject({ kind: "many", text: "2 watching", oldestMs: undefined });
  });
});

describe("fixPrompt", () => {
  it("names the PR and every failing check, with logs only when GitHub gave one", () => {
    expect(fixPrompt(failing)).toBe("This check is failing on #413:\n- lint (log: https://ci.test/lint)\nRead the failing log, find the cause, and push a fix.");
    const two = pr(9, [check("lint", "failed"), check("test", "failed", "https://ci.test/t")]);
    expect(fixPrompt(two)).toBe("These checks are failing on #9:\n- lint\n- test (log: https://ci.test/t)\nRead the failing log, find the cause, and push a fix.");
  });
});

describe("shelfLabel", () => {
  it("reads the whole line", () => {
    expect(shelfLabel([failing], { kind: "none" })).toBe("Pull request #413: lint failed. Open the Work pane.");
    expect(shelfLabel([failing, running], taskSummary([task("a", "running"), task("b", "watching")])))
      .toBe("2 pull requests: 1 failing, 1 running. In the background: 1 running · 1 watching. Open the Work pane.");
    expect(shelfLabel([{ ...running, stale: true }], taskSummary([task("a", "running")])))
      .toBe("Pull request #412: 3 of 4 · test (rust) running, last known state. In the background: task a. Open the Work pane.");
  });
});
