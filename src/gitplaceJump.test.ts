import { describe, expect, it } from "vitest";
import { gitplaceJumpStep, type GitplaceJump } from "./gitplaceJump";

const jump: GitplaceJump = { workspaceId: "w1", path: "src/api.ts", line: 42, headBranch: "feat/x" };

describe("gitplaceJumpStep", () => {
  it("jumps once a chat in the jump's workspace is open", () => {
    expect(gitplaceJumpStep(jump, { view: "workspace", sessionWorkspaceId: "w1" })).toBe("jump");
  });

  it("waits through an unstarted draft in that workspace, so the first message keeps the jump", () => {
    expect(gitplaceJumpStep(jump, { view: "workspace", sessionWorkspaceId: undefined, draftWorkspaceId: "w1" })).toBe("wait");
  });

  it("drops the jump when the reader goes elsewhere", () => {
    expect(gitplaceJumpStep(jump, { view: "workspace", sessionWorkspaceId: "w2" })).toBe("drop");
    expect(gitplaceJumpStep(jump, { view: "workspace", draftWorkspaceId: "w2" })).toBe("drop");
    expect(gitplaceJumpStep(jump, { view: "workspace" })).toBe("drop");
    expect(gitplaceJumpStep(jump, { view: "gitplace", draftWorkspaceId: "w1" })).toBe("drop");
  });
});
