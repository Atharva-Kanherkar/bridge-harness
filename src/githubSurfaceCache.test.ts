import { describe, expect, it, beforeEach } from "vitest";
import { forgetSurfaces, rememberSurface, surfaceSnapshot } from "./githubSurfaceCache";
import type { GithubStatusResult } from "./protocol/generated/protocol";

const status: GithubStatusResult = { availability: { status: "available" }, repository: { host: "github.com", owner: "bridge", name: "harness" } };
const signedOut: GithubStatusResult = { availability: { status: "notAuthenticated", remediation: "gh auth login" }, repository: null };

beforeEach(() => forgetSurfaces());

describe("rememberSurface", () => {
  it("merges over the previous snapshot instead of replacing it", () => {
    rememberSurface("w", { status, prs: [{ number: 1 } as never], issues: [], repositoryOverview: { url: "https://github.test" } as never });
    rememberSurface("w", { status });
    expect(surfaceSnapshot("w")).toEqual({
      status,
      prs: [{ number: 1 }],
      issues: [],
      repositoryOverview: { url: "https://github.test" },
    });
  });

  it("drops cached resources when the repository or availability changes", () => {
    const other = { ...status, repository: { ...status.repository!, name: "other" } };
    rememberSurface("w", { status, prs: [{ number: 1 } as never], issues: [] });
    rememberSurface("w", { status: other });
    expect(surfaceSnapshot("w")).toEqual({ status: other });
    rememberSurface("w", { status: other, prs: [{ number: 2 } as never] });
    rememberSurface("w", { status: signedOut });
    expect(surfaceSnapshot("w")).toEqual({ status: signedOut });
  });

  it("keeps the least-recently-used workspaces within the cap", () => {
    for (let index = 0; index < 10; index++) rememberSurface(`w${index}`, { status });
    expect(surfaceSnapshot("w0")).toBeUndefined();
    expect(surfaceSnapshot("w1")).toBeUndefined();
    expect(surfaceSnapshot("w8")).toBeDefined();
    // Re-reading an old workspace renews its claim on the cap.
    rememberSurface("w2", { status });
    rememberSurface("w10", { status });
    expect(surfaceSnapshot("w2")).toBeDefined();
    expect(surfaceSnapshot("w3")).toBeUndefined();
    expect(surfaceSnapshot("w10")).toBeDefined();
  });
});
