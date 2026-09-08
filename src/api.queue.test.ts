import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "./api";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

beforeEach(() => {
  invoke.mockReset();
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
});
afterEach(() => vi.unstubAllGlobals());

describe("native invoke scheduling", () => {
  it("serves a burst larger than the daemon's job cap without flooding it", async () => {
    let active = 0;
    let peak = 0;
    invoke.mockImplementation(() => {
      active += 1;
      peak = Math.max(peak, active);
      if (active > 24) return Promise.reject("The Bridge daemon is busy; try again");
      return new Promise(resolve => queueMicrotask(() => {
        active -= 1;
        resolve({ sessions: [] });
      }));
    });

    const reads = Array.from({ length: 60 }, () => bridgeApi.state());
    expect(invoke).toHaveBeenCalledTimes(4);
    const results = await Promise.all(reads);
    expect(results).toHaveLength(60);
    expect(results.every(result => result.sessions.length === 0)).toBe(true);
    expect(peak).toBe(4);
    expect(invoke).toHaveBeenCalledTimes(60);
  });

  it("keeps health responsive when GitHub reads fill their own queue", async () => {
    let releaseGithub!: () => void;
    const github = new Promise<void>(resolve => { releaseGithub = resolve; });
    invoke.mockImplementation((command: string) => command.startsWith("github_") ? github : Promise.resolve({ adapters: [] }));

    const reads = Array.from({ length: 30 }, () => bridgeApi.githubPullRequests("workspace"));
    expect(invoke).toHaveBeenCalledTimes(2);
    await expect(bridgeApi.health()).resolves.toEqual({ adapters: [] });
    expect(invoke).toHaveBeenCalledTimes(3);

    releaseGithub();
    await Promise.all(reads);
    expect(invoke).toHaveBeenCalledTimes(31);
  });

  it("releases a failed slot and submits every queued mutation exactly once", async () => {
    const failure = new Error("save failed");
    invoke.mockRejectedValueOnce(failure).mockResolvedValue({ harnesses: [], agents: [] });
    const policy = { autoApproveProviderPermissions: false, updatedAt: "" };
    const saves = Array.from({ length: 12 }, () => bridgeApi.savePermissionPolicy(policy));

    const results = await Promise.allSettled(saves);
    expect(results[0]).toEqual({ status: "rejected", reason: failure });
    expect(results.slice(1).every(result => result.status === "fulfilled")).toBe(true);
    expect(invoke).toHaveBeenCalledTimes(12);
    for (const [command, params] of invoke.mock.calls) {
      expect(command).toBe("save_permission_policy");
      expect(params).toEqual({ policy });
    }
  });
});
