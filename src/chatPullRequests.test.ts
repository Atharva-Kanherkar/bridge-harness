import { describe, expect, it } from "vitest";
import type { SessionPullRequest } from "./protocol/generated/protocol";
import { announcement, chatPrStatus, checkTone, freshness, orderChecks, refreshInterval } from "./chatPullRequests";

const rollup = (over: Partial<SessionPullRequest["checks"]> = {}): SessionPullRequest["checks"] => ({
  total: 0, queued: 0, inProgress: 0, passed: 0, failed: 0, skipped: 0, cancelled: 0, ...over,
});

const pr = (over: Partial<SessionPullRequest> = {}): SessionPullRequest => ({
  number: 12, title: "Fix linking", url: "https://github.com/o/r/pull/12", state: "open", isDraft: false,
  headBranch: "feat/x", headSha: "abc1234", checks: rollup(), checkDetails: [], attribution: "toolCompletion",
  attachedAt: "2026-09-29T10:00:00Z", fetchedAt: "2026-09-29T10:00:00Z", stale: false, error: null, ...over,
});

describe("chatPrStatus", () => {
  it("keeps every state the card names distinct", () => {
    expect(chatPrStatus(pr({ checks: rollup({ total: 3, queued: 3 }) })).headline).toBe("3 checks queued");
    expect(chatPrStatus(pr({ checks: rollup({ total: 3, inProgress: 1, passed: 2 }) })).headline).toBe("Checks running · 2 of 3 done");
    expect(chatPrStatus(pr({ checks: rollup({ total: 3, passed: 3 }) })).phase).toBe("passing");
    expect(chatPrStatus(pr({ checks: rollup({ total: 3, failed: 1, passed: 2 }) })).headline).toBe("1 check failing");
    expect(chatPrStatus(pr({ checks: rollup({ total: 3, failed: 2, inProgress: 1 }) })).headline).toBe("2 checks failing · 1 still running");
    expect(chatPrStatus(pr()).headline).toBe("No checks reported");
    expect(chatPrStatus(pr({ checks: rollup({ total: 2, skipped: 2 }) })).phase).toBe("none");
    expect(chatPrStatus(pr({ state: "merged", checks: rollup({ total: 1, failed: 1 }) })).phase).toBe("merged");
    expect(chatPrStatus(pr({ state: "closed" })).headline).toBe("Closed without merging");
  });

  it("zero checks and queued checks are not the same state", () => {
    expect(chatPrStatus(pr()).live).toBe(false);
    expect(chatPrStatus(pr({ checks: rollup({ total: 1, queued: 1 }) })).live).toBe(true);
  });

  it("never shows green while anything failed", () => {
    const status = chatPrStatus(pr({ checks: rollup({ total: 10, passed: 9, failed: 1 }) }));
    expect(status.tone).toBe("danger");
  });

  it("marks drafts in the strip summary", () => {
    expect(chatPrStatus(pr({ isDraft: true, checks: rollup({ total: 1, passed: 1 }) })).summary).toBe("Draft · Checks passed");
  });
});

describe("checks", () => {
  it("classifies and orders failures first", () => {
    const checks = [
      { name: "a", status: "completed", conclusion: "success", logUrl: "", workflow: "" },
      { name: "b", status: "inProgress", conclusion: null, logUrl: "", workflow: "" },
      { name: "c", status: "completed", conclusion: "timedOut", logUrl: "", workflow: "" },
      { name: "d", status: "completed", conclusion: "skipped", logUrl: "", workflow: "" },
    ] as SessionPullRequest["checkDetails"];
    expect(checks.map(checkTone)).toEqual(["passed", "running", "failed", "skipped"]);
    expect(orderChecks(checks).map(check => check.name)).toEqual(["c", "b", "a", "d"]);
  });
});

describe("freshness and cadence", () => {
  const at = Date.parse("2026-09-29T10:00:00Z");
  it("reads relative time", () => {
    expect(freshness("2026-09-29T10:00:00Z", at + 3_000)).toBe("just now");
    expect(freshness("2026-09-29T10:00:00Z", at + 42_000)).toBe("42s ago");
    expect(freshness("2026-09-29T10:00:00Z", at + 5 * 60_000)).toBe("5m ago");
    expect(freshness(null, at)).toBe("never refreshed");
  });

  it("stops refreshing once every PR is terminal", () => {
    expect(refreshInterval([])).toBeNull();
    expect(refreshInterval([pr({ state: "merged" }), pr({ state: "closed" })])).toBeNull();
    expect(refreshInterval([pr({ checks: rollup({ total: 1, inProgress: 1 }) })])).toBe(20_000);
    expect(refreshInterval([pr({ checks: rollup({ total: 1, passed: 1 }) })])).toBe(90_000);
    expect(refreshInterval([pr({ stale: true })])).toBe(30_000);
  });
});

describe("announcement", () => {
  it("speaks on phase transitions only", () => {
    const running = pr({ checks: rollup({ total: 2, inProgress: 2 }) });
    expect(announcement(undefined, running)).toBeNull();
    expect(announcement("running", running)).toBeNull();
    expect(announcement("running", pr({ checks: rollup({ total: 2, passed: 2 }) }))).toBe("Pull request #12: All checks passed.");
  });
});
