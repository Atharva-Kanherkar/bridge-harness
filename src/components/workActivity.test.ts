import { describe, expect, it } from "vitest";
import type { WorkTask } from "../protocol/generated/protocol";
import { recentIntegrationActivity } from "./workActivity";

const now = new Date("2026-09-10T12:00:00Z");
export const activity = (overrides: Partial<WorkTask> = {}): WorkTask => ({
  id: "slack-1", connectorInstanceId: "slack-work", sourceKind: "slack.message",
  title: "Release discussion", why: "The team shared a release update.", rank: 1,
  confidenceBps: 8000, state: "active", pinned: false, missCount: 0,
  sourceActivityAt: "2026-09-10T11:00:00Z", createdAt: now.toISOString(), updatedAt: now.toISOString(),
  evidenceTarget: { kind: "externalLink", url: "https://app.slack.com/archives/C1/p1", host: "app.slack.com" },
  ...overrides,
});

describe("recentIntegrationActivity", () => {
  it("includes both window boundaries and sorts by source activity, not model rank or pins", () => {
    const items = [activity({ id: "oldest", sourceActivityAt: "2026-09-09T12:00:00Z", pinned: true }), activity({ id: "latest", sourceActivityAt: now.toISOString(), rank: 12 }), activity()];
    expect(recentIntegrationActivity(items, now).map(item => item.id)).toEqual(["latest", "slack-1", "oldest"]);
  });
  it.each([undefined, null, "invalid", "2026-09-09T11:59:59.999Z", "2026-09-10T12:00:00.001Z"])("excludes invalid or out-of-window source dates: %s", date => {
    expect(recentIntegrationActivity([activity({ sourceActivityAt: date, pinned: true, evidenceObservedAt: now.toISOString() })], now)).toEqual([]);
  });
  it("rejects local sources, unknown integrations, missing identities and hidden states", () => {
    const tasks = [activity({ sourceKind: "bridge.check" }), activity({ sourceKind: "unknown" }), activity({ connectorInstanceId: "" }), ...(["stale", "done", "dismissed", "snoozed"] as const).map(state => activity({ state, pinned: true }))];
    expect(recentIntegrationActivity(tasks, now)).toEqual([]);
  });
  it.each(["slack.message", "github.item", "gmail.thread", "linear.issue", "notion.page"])("accepts dated %s activity", sourceKind => {
    expect(recentIntegrationActivity([activity({ sourceKind })], now)).toHaveLength(1);
  });
});
