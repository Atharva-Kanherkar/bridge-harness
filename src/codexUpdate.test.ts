import { afterEach, describe, expect, it, vi } from "vitest";
import { isCodexVersionError, isOlderCodexVersion, latestCodexVersion, withCodexRefreshDeadline } from "./codexUpdate";

describe("Codex update detection", () => {
  it("compares the installed CLI with the latest stable release", () => {
    expect(isOlderCodexVersion("codex-cli 0.155.0-alpha.9.2", "0.157.0")).toBe(true);
    expect(isOlderCodexVersion("codex-cli 0.157.0-alpha.1", "0.157.0")).toBe(true);
    expect(isOlderCodexVersion("codex-cli 0.157.0", "0.157.0")).toBe(false);
    expect(isOlderCodexVersion("codex-cli 0.158.0", "0.157.0")).toBe(false);
    expect(isOlderCodexVersion("mock", "0.157.0")).toBe(false);
  });

  it("recognizes only the Codex compatibility warning", () => {
    expect(isCodexVersionError("Codex codex-cli 0.155.0-alpha.9.2 is incompatible with Bridge. Upgrade to Codex 0.153.4 or newer.")).toBe(true);
    expect(isCodexVersionError("Claude is incompatible with Bridge")).toBe(false);
  });

  it("reads the stable Codex release and tolerates an unavailable feed", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch");
    fetchMock.mockResolvedValueOnce({ ok: true, json: async () => ({ tag_name: "rust-v0.157.0" }) } as Response);
    expect(await latestCodexVersion()).toBe("0.157.0");
    fetchMock.mockRejectedValueOnce(new Error("offline"));
    expect(await latestCodexVersion()).toBeUndefined();
    fetchMock.mockRestore();
  });
});

describe("Codex update refresh", () => {
  afterEach(() => vi.useRealTimers());

  it("releases the update when a runtime refresh never answers", async () => {
    vi.useFakeTimers();
    const refresh = withCodexRefreshDeadline(new Promise<never>(() => {}));
    const rejected = expect(refresh).rejects.toThrow("Checking the Codex runtime timed out");
    await vi.advanceTimersByTimeAsync(30_000);
    await rejected;
    expect(vi.getTimerCount()).toBe(0);
  });

  it("clears the deadline after a successful or failed refresh", async () => {
    vi.useFakeTimers();
    await expect(withCodexRefreshDeadline(Promise.resolve("updated"))).resolves.toBe("updated");
    await expect(withCodexRefreshDeadline(Promise.reject(new Error("offline")))).rejects.toThrow("offline");
    expect(vi.getTimerCount()).toBe(0);
  });
});
