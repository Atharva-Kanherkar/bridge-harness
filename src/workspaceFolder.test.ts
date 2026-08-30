import { describe, expect, it } from "vitest";
import type { Workspace } from "./types";
import { selectedFolder, workspaceForFolder, workspaceTitleFromFolder } from "./workspaceFolder";

const workspace = (path: string | null, id = "workspace"): Workspace => ({ id, path } as Workspace);

describe("selectedFolder", () => {
  it("only accepts the single-folder result from the picker", () => {
    expect(selectedFolder("/Users/you/Projects/bridge")).toBe("/Users/you/Projects/bridge");
    expect(selectedFolder(null)).toBeNull();
    expect(selectedFolder(["/Users/you/Projects/bridge"])).toBeNull();
  });
});

describe("workspaceTitleFromFolder", () => {
  it("uses the selected directory's own name", () => {
    expect(workspaceTitleFromFolder("/Users/you/Projects/bridge/")).toBe("bridge");
    expect(workspaceTitleFromFolder("/")).toBe("Folder");
  });
});

describe("workspaceForFolder", () => {
  it("finds an already-connected project and ignores pathless rows", () => {
    const existing = workspace("/Users/you/Projects/bridge", "existing");
    expect(workspaceForFolder([workspace(null, "pathless"), existing], "/Users/you/Projects/bridge")).toBe(existing);
  });
});
