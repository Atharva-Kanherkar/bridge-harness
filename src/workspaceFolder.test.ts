import { describe, expect, it } from "vitest";
import type { Workspace } from "./types";
import { repoCloneTarget, selectedFolder, workspaceForFolder, workspaceTitleFromFolder } from "./workspaceFolder";

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

describe("repoCloneTarget", () => {
  it("accepts every GitHub URL shape the backend clone validator accepts", () => {
    expect(repoCloneTarget("https://github.com/rimo/bridge-harness")).toBe("https://github.com/rimo/bridge-harness");
    expect(repoCloneTarget("  git@github.com:rimo/bridge-harness.git  ")).toBe("git@github.com:rimo/bridge-harness.git");
    expect(repoCloneTarget("ssh://git@github.com/rimo/bridge-harness.git")).toBe("ssh://git@github.com/rimo/bridge-harness.git");
  });

  it("expands an owner/repo shorthand into a GitHub URL", () => {
    expect(repoCloneTarget("rimo/bridge-harness")).toBe("https://github.com/rimo/bridge-harness");
  });

  it("ignores shorthand that looks like a pasted file path", () => {
    expect(repoCloneTarget("src/App.tsx")).toBeUndefined();
  });

  it("ignores a non-GitHub host, which the backend clone validator would reject anyway", () => {
    expect(repoCloneTarget("https://gitlab.com/rimo/bridge-harness")).toBeUndefined();
  });

  it("ignores normal chat messages, even ones mentioning a URL", () => {
    expect(repoCloneTarget("can you open https://github.com/rimo/bridge-harness for me")).toBeUndefined();
    expect(repoCloneTarget("")).toBeUndefined();
    expect(repoCloneTarget("what does this repo do")).toBeUndefined();
  });
});
