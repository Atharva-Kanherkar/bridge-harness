import { describe, expect, it } from "vitest";
import { pullsJumpStep, type PullsJump } from "./pullsJump";

const jump: PullsJump = { workspaceId: "w1", path: "src/api.ts", line: 42, headBranch: "feat/x" };

describe("pullsJumpStep", () => {
  it("jumps once a chat in the jump's workspace is open", () => {
    expect(pullsJumpStep(jump, { view: "workspace", sessionWorkspaceId: "w1" })).toBe("jump");
  });

  it("waits through an unstarted draft in that workspace, so the first message keeps the jump", () => {
    expect(pullsJumpStep(jump, { view: "workspace", sessionWorkspaceId: undefined, draftWorkspaceId: "w1" })).toBe("wait");
  });

  it("drops the jump when the reader goes elsewhere", () => {
    expect(pullsJumpStep(jump, { view: "workspace", sessionWorkspaceId: "w2" })).toBe("drop");
    expect(pullsJumpStep(jump, { view: "workspace", draftWorkspaceId: "w2" })).toBe("drop");
    expect(pullsJumpStep(jump, { view: "workspace" })).toBe("drop");
    expect(pullsJumpStep(jump, { view: "pulls", draftWorkspaceId: "w1" })).toBe("drop");
  });
});
