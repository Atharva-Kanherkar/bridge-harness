import { describe, expect, it } from "vitest";
import type { BridgeState, Workspace } from "./types";
import { withWorkspace } from "./workspaceStats";

const workspace = (overrides: Partial<Workspace> = {}): Workspace => ({
  id: "w1", projectId: "p1", city: null, title: "Task", branch: "main", path: "/repo",
  status: "idle", dirtyFiles: 0, additions: 0, deletions: 0, createdAt: "now", ...overrides,
} as Workspace);

const state = (workspaces: Workspace[]): BridgeState => ({ projects: [], workspaces, sessions: [], events: [] } as BridgeState);

describe("withWorkspace", () => {
  it("returns the same state when the poll found nothing new, so nothing re-renders", () => {
    const before = state([workspace()]);
    expect(withWorkspace(before, workspace())).toBe(before);
  });

  it("swaps in only the workspace that moved", () => {
    const other = workspace({ id: "w2" });
    const before = state([workspace(), other]);
    const after = withWorkspace(before, workspace({ dirtyFiles: 3, additions: 10, deletions: 2 }));
    expect(after).not.toBe(before);
    expect(after.workspaces[0]).toMatchObject({ dirtyFiles: 3, additions: 10, deletions: 2 });
    expect(after.workspaces[1]).toBe(other);
    expect(after.sessions).toBe(before.sessions);
  });

  it("ignores a workspace the state no longer has", () => {
    const before = state([workspace()]);
    expect(withWorkspace(before, workspace({ id: "gone", dirtyFiles: 9 }))).toBe(before);
  });
});
