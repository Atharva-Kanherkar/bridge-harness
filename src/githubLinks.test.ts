import { describe, expect, it } from "vitest";
import { describeGithubLink, githubLinkMatchesRepository, parseGithubLink } from "./githubLinks";
import type { GithubRepository } from "./protocol/generated/protocol";

const repository: GithubRepository = { host: "github.com", owner: "bridge", name: "harness" };
const url = (path: string) => `https://github.com${path}`;
const view = (path: string) => parseGithubLink(url(path))?.view;

describe("parseGithubLink", () => {
  it("reads a pull request", () => {
    expect(parseGithubLink(url("/bridge/harness/pull/12"))).toEqual({
      host: "github.com", owner: "bridge", name: "harness",
      view: { kind: "pull", number: 12, tab: "conversation" },
    });
  });

  it("takes the sub-tab from the sub-path the pane has a view for", () => {
    expect(view("/bridge/harness/pull/12/files")).toEqual({ kind: "pull", number: 12, tab: "changes" });
    expect(view("/bridge/harness/pull/12/commits")).toEqual({ kind: "pull", number: 12, tab: "commits" });
    expect(view("/bridge/harness/pull/12/checks")).toEqual({ kind: "pull", number: 12, tab: "checks" });
  });

  it("still lands on the pull request for sub-paths that have no view of their own", () => {
    // One commit's own page: the Commits tab is the nearest thing the pane has.
    expect(view("/bridge/harness/pull/12/commits/9f8e7d6")).toEqual({ kind: "pull", number: 12, tab: "commits" });
    expect(view("/bridge/harness/pull/12/agent-sessions")).toEqual({ kind: "pull", number: 12, tab: "conversation" });
  });

  it("ignores the query and the fragment", () => {
    expect(view("/bridge/harness/pull/12/files?w=1#diff-abc123")).toEqual({ kind: "pull", number: 12, tab: "changes" });
    expect(view("/bridge/harness/pull/12#issuecomment-99")).toEqual({ kind: "pull", number: 12, tab: "conversation" });
  });

  it("reads an issue", () => {
    expect(view("/bridge/harness/issues/204")).toEqual({ kind: "issue", number: 204 });
  });

  it("reads the two lists and the repository overview", () => {
    expect(view("/bridge/harness/pulls")).toEqual({ kind: "pulls" });
    expect(view("/bridge/harness/issues")).toEqual({ kind: "issues" });
    expect(view("/bridge/harness")).toEqual({ kind: "repository" });
    expect(view("/bridge/harness/")).toEqual({ kind: "repository" });
  });

  it("declines every path the pane has no view for", () => {
    for (const path of [
      "/bridge/harness/commit/9f8e7d6",
      "/bridge/harness/blob/main/src/api.ts",
      "/bridge/harness/tree/main/src",
      "/bridge/harness/releases",
      "/bridge/harness/actions/runs/1234",
      "/bridge/harness/discussions/3",
      "/bridge/harness/wiki",
      "/bridge/harness/issues/new",
      "/bridge/harness/compare/main...feat",
      "/bridge",
      "/",
    ]) {
      expect(parseGithubLink(url(path)), path).toBeNull();
    }
  });

  it("declines anything that is not a real issue or pull request number", () => {
    for (const path of ["/bridge/harness/pull/abc", "/bridge/harness/pull/", "/bridge/harness/pull", "/bridge/harness/issues/0", "/bridge/harness/pull/1.5", "/bridge/harness/pull/-1", "/bridge/harness/pull/01"]) {
      expect(parseGithubLink(url(path)), path).toBeNull();
    }
  });

  it("parses only http and https", () => {
    expect(parseGithubLink("javascript:alert(1)")).toBeNull();
    expect(parseGithubLink("file:///bridge/harness/pull/1")).toBeNull();
    expect(parseGithubLink("mailto:a@example.com")).toBeNull();
    expect(parseGithubLink("not a url")).toBeNull();
    expect(parseGithubLink("")).toBeNull();
    expect(parseGithubLink("http://github.com/bridge/harness/pull/1")?.view).toEqual({ kind: "pull", number: 1, tab: "conversation" });
  });

  it("carries the host lowercased, so an Enterprise host parses like any other", () => {
    expect(parseGithubLink("https://GITHUB.COM/bridge/harness/pull/1")?.host).toBe("github.com");
    expect(parseGithubLink("https://ghe.corp.example/bridge/harness/pull/1")).toEqual({
      host: "ghe.corp.example", owner: "bridge", name: "harness",
      view: { kind: "pull", number: 1, tab: "conversation" },
    });
  });

  it("does not keep a trailing .git in the repository name", () => {
    expect(parseGithubLink(url("/bridge/harness.git/pull/1"))?.name).toBe("harness");
  });
});

describe("githubLinkMatchesRepository", () => {
  const link = (path: string) => parseGithubLink(url(path))!;

  it("requires the host, the owner and the name to agree", () => {
    expect(githubLinkMatchesRepository(link("/bridge/harness/pull/1"), repository)).toBe(true);
    expect(githubLinkMatchesRepository(link("/someone/harness/pull/1"), repository)).toBe(false);
    expect(githubLinkMatchesRepository(link("/bridge/other/pull/1"), repository)).toBe(false);
    expect(githubLinkMatchesRepository(parseGithubLink("https://ghe.corp.example/bridge/harness/pull/1")!, repository)).toBe(false);
  });

  it("compares owner and name the way GitHub does, without case", () => {
    expect(githubLinkMatchesRepository(link("/Bridge/Harness/pull/1"), repository)).toBe(true);
    expect(githubLinkMatchesRepository(link("/bridge/harness/pull/1"), { ...repository, owner: "BRIDGE", name: "HARNESS" })).toBe(true);
  });

  it("matches nothing when the workspace resolved no repository", () => {
    expect(githubLinkMatchesRepository(link("/bridge/harness/pull/1"), null)).toBe(false);
    expect(githubLinkMatchesRepository(link("/bridge/harness/pull/1"), undefined)).toBe(false);
  });
});

describe("describeGithubLink", () => {
  const describe_ = (path: string) => describeGithubLink(parseGithubLink(url(path))!);

  it("names what the reader is about to open", () => {
    expect(describe_("/bridge/harness/pull/12")).toBe("Pull request #12");
    expect(describe_("/bridge/harness/pull/12/files")).toBe("Pull request #12 · Changes");
    expect(describe_("/bridge/harness/pull/12/commits")).toBe("Pull request #12 · Commits");
    expect(describe_("/bridge/harness/pull/12/checks")).toBe("Pull request #12 · Checks");
    expect(describe_("/bridge/harness/issues/204")).toBe("Issue #204");
    expect(describe_("/bridge/harness/pulls")).toBe("Pull requests");
    expect(describe_("/bridge/harness/issues")).toBe("Issues");
    expect(describe_("/bridge/harness")).toBe("Repository");
  });
});
