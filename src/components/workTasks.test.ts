import { describe, expect, it } from "vitest";
import type { WorkTask, WorkTaskState } from "../protocol/generated/protocol";
import {
  ACTIONS_BY_STATE,
  confidenceLabel,
  evidenceLabel,
  hasOpenableEvidence,
  isVisibleTask,
  orderTasks,
  sourceLabel,
  taskAnnouncement,
  taskRoute,
} from "./workTasks";

function task(overrides: Partial<WorkTask> = {}): WorkTask {
  return {
    id: "task-v1:abc",
    fingerprint: "v1:abc",
    connectorInstanceId: "slack-work",
    canonicalResourceId: "slack:slack-work:1.1",
    sourceKind: "slack.message",
    title: "Reply to Priya",
    why: "She asked twice and nobody answered.",
    rank: 1,
    confidenceBps: 8_200,
    state: "active",
    pinned: false,
    snoozedUntil: null,
    evidenceDigest: "d".repeat(64),
    evidenceTarget: { kind: "externalLink", url: "https://app.slack.com/archives/C1/p1", host: "app.slack.com" },
    evidenceObservedAt: "2026-08-19T12:00:00Z",
    missCount: 0,
    workspaceId: null,
    createdAt: "2026-08-19T12:00:00Z",
    updatedAt: "2026-08-19T12:00:00Z",
    ...overrides,
  };
}

describe("visibility", () => {
  it("hides everything a human has put away, and stale unless pinned", () => {
    expect(isVisibleTask(task({ state: "active" }))).toBe(true);
    expect(isVisibleTask(task({ state: "stale", pinned: false }))).toBe(false);
    expect(isVisibleTask(task({ state: "stale", pinned: true }))).toBe(true);
    for (const state of ["snoozed", "done", "dismissed"] as WorkTaskState[]) {
      expect(isVisibleTask(task({ state, pinned: true }))).toBe(false);
    }
  });

  it("puts pinned tasks first and otherwise keeps the model's ranking", () => {
    // The one place the client reorders, and it reorders by a decision the user made rather
    // than second-guessing the ranking.
    const ordered = orderTasks([
      task({ fingerprint: "a", rank: 1 }),
      task({ fingerprint: "b", rank: 2 }),
      task({ fingerprint: "c", rank: 3, pinned: true }),
    ]);
    expect(ordered.map(item => item.fingerprint)).toEqual(["c", "a", "b"]);
  });

  it("breaks a rank tie on durable id so the order is total", () => {
    const ordered = orderTasks([
      task({ id: "z", fingerprint: "z", rank: 1 }),
      task({ id: "a", fingerprint: "a", rank: 1 }),
    ]);
    expect(ordered.map(item => item.fingerprint)).toEqual(["a", "z"]);
  });

  it("drops what it must not show while ordering", () => {
    const ordered = orderTasks([task({ fingerprint: "a" }), task({ fingerprint: "b", state: "dismissed" })]);
    expect(ordered.map(item => item.fingerprint)).toEqual(["a"]);
  });
});

describe("actions offered", () => {
  it("never offers a button the backend would refuse", () => {
    // The Rust state machine refuses re-snoozing, re-completing, and restoring what was
    // never put away. A row that showed those buttons would be promising something.
    expect(ACTIONS_BY_STATE.done).toEqual([]);
    expect(ACTIONS_BY_STATE.dismissed).toEqual(["restore"]);
    expect(ACTIONS_BY_STATE.snoozed).not.toContain("snooze");
    expect(ACTIONS_BY_STATE.active).not.toContain("restore");
    expect(ACTIONS_BY_STATE.stale).not.toContain("restore");
  });

  it("offers start on every state a task can still be worked from", () => {
    expect(ACTIONS_BY_STATE.active).toContain("start");
    expect(ACTIONS_BY_STATE.stale).toContain("start");
    // Not from something put away: starting work on a dismissed task means restoring it
    // first, which is a decision the user should make explicitly.
    expect(ACTIONS_BY_STATE.dismissed).not.toContain("start");
    expect(ACTIONS_BY_STATE.snoozed).not.toContain("start");
  });

  it("covers every state", () => {
    for (const state of ["active", "snoozed", "done", "dismissed", "stale"] as WorkTaskState[]) {
      expect(ACTIONS_BY_STATE[state]).toBeDefined();
    }
  });
});

describe("what a row says", () => {
  it("bands confidence instead of printing a false precision", () => {
    // A model's 82% is not a measurement, and printing it to the percent invites a
    // precision it does not have.
    expect(confidenceLabel(8_200)).toBe("high confidence");
    expect(confidenceLabel(7_500)).toBe("high confidence");
    expect(confidenceLabel(5_000)).toBe("medium confidence");
    expect(confidenceLabel(4_000)).toBe("medium confidence");
    expect(confidenceLabel(3_999)).toBe("low confidence");
    expect(confidenceLabel(0)).toBe("low confidence");
    for (const bps of [0, 4_000, 8_200, 10_000]) {
      expect(confidenceLabel(bps)).not.toMatch(/\d/);
    }
  });

  it("names the source and the account it came from", () => {
    expect(sourceLabel(task())).toBe("Slack · slack-work");
    expect(sourceLabel(task({ sourceKind: "gmail.thread", connectorInstanceId: "gmail-personal" })))
      .toBe("Gmail · gmail-personal");
    // Two accounts must read differently, or a task from the wrong one looks the same.
    expect(sourceLabel(task({ connectorInstanceId: "slack-personal" })))
      .not.toBe(sourceLabel(task({ connectorInstanceId: "slack-work" })));
  });

  it("carries source, state and confidence as words for a screen reader", () => {
    const announcement = taskAnnouncement(task());
    expect(announcement).toContain("Suggested from Slack · slack-work");
    expect(announcement).toContain("Reply to Priya");
    expect(announcement).toContain("high confidence");
    expect(taskAnnouncement(task({ state: "stale", pinned: true }))).toContain("stale");
    expect(taskAnnouncement(task({ pinned: true }))).toContain("pinned");
  });
});

describe("evidence affordance", () => {
  it("offers to open only a shape the backend produced", () => {
    expect(hasOpenableEvidence(task())).toBe(true);
    expect(evidenceLabel(task())).toBe("Open on app.slack.com");
    expect(hasOpenableEvidence(task({ evidenceTarget: { kind: "session", sessionId: "s-1" } }))).toBe(true);
    expect(evidenceLabel(task({ evidenceTarget: { kind: "session", sessionId: "s-1" } }))).toBe("Open session");
  });

  it("draws no affordance for anything that would be refused on open", () => {
    // Drawing a button the Rust check would refuse is worse than drawing none.
    for (const target of [
      null,
      { kind: "externalLink", url: "http://app.slack.com/x", host: "app.slack.com" },
      { kind: "externalLink", url: "javascript:alert(1)", host: "app.slack.com" },
      { kind: "session", sessionId: "" },
    ] as WorkTask["evidenceTarget"][]) {
      expect(hasOpenableEvidence(task({ evidenceTarget: target }))).toBe(false);
    }
  });

  it("names the host it would send you to, so the destination is visible before the click", () => {
    expect(evidenceLabel(task({
      evidenceTarget: { kind: "externalLink", url: "https://github.com/o/r/pull/1", host: "github.com" },
    }))).toBe("Open on github.com");
  });
});

describe("taskRoute", () => {
  it("routes a workspace-bound task to Code and everything else to Work", () => {
    expect(taskRoute({ workspaceId: "w-1" })).toEqual({ kind: "code", workspaceId: "w-1" });
    expect(taskRoute({ workspaceId: null })).toEqual({ kind: "work" });
  });
});
