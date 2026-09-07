import { describe, expect, it } from "vitest";
import { ancestorPaths, buildFileTree, collapseChains, rankPaths } from "./fileTree";

describe("buildFileTree", () => {
  it("nests paths and sorts directories before files", () => {
    const tree = buildFileTree(["README.md", "src/App.tsx", "src/components/ui/button.tsx", "package.json"]);
    expect(tree.map(node => node.name)).toEqual(["src", "package.json", "README.md"]);
    const src = tree[0];
    expect(src.children?.map(node => node.name)).toEqual(["components", "App.tsx"]);
  });

  it("sorts case-insensitively", () => {
    const tree = buildFileTree(["zebra.ts", "Apple.ts", "banana.ts"]);
    expect(tree.map(node => node.name)).toEqual(["Apple.ts", "banana.ts", "zebra.ts"]);
  });

  it("keeps a file and a directory of the same name apart", () => {
    const tree = buildFileTree(["src/theme", "src/theme/index.ts"]);
    expect(tree[0].children?.map(node => [node.name, Boolean(node.children)])).toEqual([
      ["theme", true],
      ["theme", false],
    ]);
  });

  it("ignores empty and trailing-slash paths", () => {
    expect(buildFileTree(["", "a//b"])).toEqual([
      { name: "a", path: "a", children: [{ name: "b", path: "a/b" }] },
    ]);
  });
});

describe("collapseChains", () => {
  it("folds single-child directory chains into one row", () => {
    const tree = collapseChains(buildFileTree(["src/components/ui/button.tsx"]));
    expect(tree[0].name).toBe("src/components/ui");
    expect(tree[0].path).toBe("src/components/ui");
    expect(tree[0].children?.map(node => node.name)).toEqual(["button.tsx"]);
  });

  it("stops folding where a directory branches", () => {
    const tree = collapseChains(buildFileTree(["src/a/one.ts", "src/b/two.ts"]));
    expect(tree[0].name).toBe("src");
    expect(tree[0].children?.map(node => node.name)).toEqual(["a", "b"]);
  });

  it("does not fold a directory whose only child is a file", () => {
    const tree = collapseChains(buildFileTree(["src/App.tsx"]));
    expect(tree[0].name).toBe("src");
  });
});

describe("ancestorPaths", () => {
  it("lists the directories on the way to a file", () => {
    expect(ancestorPaths("src/components/ui/button.tsx")).toEqual(["src", "src/components", "src/components/ui"]);
  });

  it("is empty for a root file", () => {
    expect(ancestorPaths("README.md")).toEqual([]);
  });
});

describe("rankPaths", () => {
  const paths = ["src/App.tsx", "src/components/AgentConversation.tsx", "docs/app-notes.md", "src/api.ts"];

  it("returns everything for an empty query", () => {
    expect(rankPaths(paths, "  ")).toEqual(paths);
  });

  it("prefers a basename prefix over a match deeper in the path", () => {
    expect(rankPaths(paths, "app")[0]).toBe("src/App.tsx");
  });

  it("matches a subsequence across the basename", () => {
    expect(rankPaths(paths, "agtcnv")[0]).toBe("src/components/AgentConversation.tsx");
  });

  it("drops paths the query cannot match", () => {
    expect(rankPaths(paths, "zzzz")).toEqual([]);
  });

  it("honours the limit", () => {
    expect(rankPaths(paths, "", 2)).toHaveLength(2);
  });
});
