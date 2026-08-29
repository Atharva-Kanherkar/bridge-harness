import { describe, expect, it } from "vitest";
import { checksNeedPolling, ciNotificationText, ciToastKey, jumpFallbackHint, rollupState } from "./githubSurface";

const checks = (partial: Partial<{ total: number; queued: number; inProgress: number; passed: number; failed: number }>) => ({
  checks: { total: 0, queued: 0, inProgress: 0, passed: 0, failed: 0, skipped: 0, cancelled: 0, ...partial },
});

describe("rollupState", () => {
  it("classifies failing over running over passing", () => {
    expect(rollupState(checks({ failed: 1, inProgress: 2, passed: 3, total: 6 }))).toBe("failing");
    expect(rollupState(checks({ inProgress: 1, passed: 1, total: 2 }))).toBe("running");
    expect(rollupState(checks({ queued: 1, total: 1 }))).toBe("running");
    expect(rollupState(checks({ passed: 2, total: 2 }))).toBe("passing");
    expect(rollupState(checks({}))).toBe("none");
  });
});

describe("checksNeedPolling", () => {
  it("polls only while at least one check is non-terminal", () => {
    expect(checksNeedPolling([{ name: "build", status: "queued", conclusion: null, logUrl: "", workflow: "CI" }])).toBe(true);
    expect(checksNeedPolling([{ name: "build", status: "inProgress", conclusion: null, logUrl: "", workflow: "CI" }])).toBe(true);
    expect(checksNeedPolling([{ name: "build", status: "completed", conclusion: "success", logUrl: "", workflow: "CI" }])).toBe(false);
    expect(checksNeedPolling([])).toBe(false);
  });
});

describe("ciNotificationText", () => {
  it("renders a failure with its check count and branch", () => {
    const text = ciNotificationText({ workspaceId: "w", number: 341, headBranch: "feat/cursor-sidebar-dev", title: "Sidebar dev", failed: 2, total: 5 });
    expect(text.headline).toBe("CI failed on feat/cursor-sidebar-dev — 2 checks");
    expect(text.detail).toBe("#341 Sidebar dev");
    expect(text.tone).toBe("failure");
  });

  it("singularizes one failed check and renders success without a count", () => {
    expect(ciNotificationText({ workspaceId: "w", number: 1, headBranch: "b", title: "t", failed: 1, total: 3 }).headline).toBe("CI failed on b — 1 check");
    const passed = ciNotificationText({ workspaceId: "w", number: 1, headBranch: "b", title: "t", failed: 0, total: 3 });
    expect(passed.headline).toBe("CI passed on b");
    expect(passed.tone).toBe("success");
  });
});

describe("ciToastKey", () => {
  it("keys per workspace, PR, and terminal outcome so redeliveries collapse", () => {
    const payload = { workspaceId: "w", number: 7, headBranch: "b", title: "t", failed: 1, total: 2 };
    expect(ciToastKey(payload)).toBe(ciToastKey({ ...payload, title: "renamed" }));
    expect(ciToastKey(payload)).not.toBe(ciToastKey({ ...payload, failed: 0 }));
    expect(ciToastKey(payload)).not.toBe(ciToastKey({ ...payload, workspaceId: "other" }));
  });
});

describe("jumpFallbackHint", () => {
  it("is silent when the PR head branch is checked out here", () => {
    expect(jumpFallbackHint("feat/x", "feat/x")).toBeNull();
  });

  it("names both branches when they differ, and copes with a repo-less workspace", () => {
    expect(jumpFallbackHint("main", "feat/x")).toBe("feat/x isn’t checked out here — showing the file on main.");
    expect(jumpFallbackHint(null, "feat/x")).toBe("feat/x isn’t checked out here — showing the file from this workspace.");
  });
});
