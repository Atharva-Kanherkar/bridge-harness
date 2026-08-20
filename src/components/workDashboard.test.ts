import { describe, expect, it } from "vitest";
import type { WorkBriefRun, WorkSourceCoverage } from "../protocol/generated/protocol";
import { lastRunLine, relativeTime, toolsReadLine } from "./workDashboard";

const NOW = new Date("2026-08-19T12:00:00.000Z");

function source(overrides: Partial<WorkSourceCoverage>): WorkSourceCoverage {
  return { connectorInstanceId: "slack-work", connectorFamily: "slack", status: "succeeded", detail: null, observedAt: null, ...overrides };
}

function run(overrides: Partial<WorkBriefRun>): WorkBriefRun {
  return {
    id: "run-1", trigger: "manual", status: "succeeded", profileReference: "claude/haiku",
    sessionId: null, outputDigest: null, failureCode: null, failureDetail: null, usage: null,
    startedAt: "2026-08-19T11:48:00.000Z", completedAt: "2026-08-19T11:48:41.000Z", ...overrides,
  };
}

describe("toolsReadLine", () => {
  it("says what was read, what failed, and what needs sign-in — separately", () => {
    const line = toolsReadLine([
      source({}),
      source({ connectorInstanceId: "github", connectorFamily: "github" }),
      source({ connectorInstanceId: "notion-1", connectorFamily: "notion", status: "failed" }),
      source({ connectorInstanceId: "gmail-1", connectorFamily: "gmail", status: "auth_required" }),
    ]);
    // The Read clause is pinned exactly: only the succeeded sources may appear
    // in it, so a failed or signed-out family sneaking in fails here.
    expect(line.split(" · ")[0]).toBe("Read Slack, GitHub");
    expect(line).toContain("Notion could not be reached");
    expect(line).toContain("Gmail needs sign-in — reconnect it in your harness");
  });

  it("never claims an unread source was read", () => {
    const line = toolsReadLine([source({ status: "eligible" }), source({ connectorInstanceId: "g", connectorFamily: "github", status: "consulted" })]);
    // Neither the eligible-but-unread source nor the consulted one may appear
    // as read — the whole first clause must be the empty reading.
    expect(line.split(" · ")[0]).toBe("Nothing was read.");
    expect(line).not.toContain("Read Slack");
    expect(line).not.toContain("Read GitHub");
    expect(line).not.toContain("GitHub");
  });

  it("says so plainly when there were no tools at all", () => {
    expect(toolsReadLine([])).toBe("No tools were read.");
  });

  it("falls back to the instance id for a family it cannot name", () => {
    expect(toolsReadLine([source({ connectorInstanceId: "internal-crm", connectorFamily: "unknown" })])).toContain("internal-crm");
  });
});

describe("lastRunLine", () => {
  it("covers every terminal state without provider text", () => {
    expect(lastRunLine(null, NOW)).toBe("No briefing has run yet.");
    expect(lastRunLine(run({ status: "running" }), NOW)).toBe("A briefing is running now.");
    expect(lastRunLine(run({}), NOW)).toBe("Last briefing 11m ago · completed.");
    expect(lastRunLine(run({ status: "cancelled" }), NOW)).toContain("cancelled");
    const failed = lastRunLine(run({ status: "failed", failureCode: "budget_exceeded", failureDetail: "raw provider words" }), NOW);
    expect(failed).toContain("failed (budget_exceeded)");
    expect(failed).toContain("The previous board is untouched");
    expect(failed).not.toContain("raw provider words");
  });
});

describe("relativeTime", () => {
  it("rounds to the unit a human would say", () => {
    expect(relativeTime("2026-08-19T11:59:31.000Z", NOW)).toBe("just now");
    expect(relativeTime("2026-08-19T11:48:00.000Z", NOW)).toBe("12m ago");
    expect(relativeTime("2026-08-19T09:00:00.000Z", NOW)).toBe("3h ago");
    expect(relativeTime("2026-08-17T09:00:00.000Z", NOW)).toBe("2d ago");
    expect(relativeTime("not a time", NOW)).toBe("at an unknown time");
  });
});
