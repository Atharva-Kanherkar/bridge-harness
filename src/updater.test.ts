// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { checkForUpdate, getUpdateChannel, installUpdateAndRestart, setUpdateChannel, UpdateInstallUnavailableError, UpdateCheckSupersededError } from "./updater";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: vi.fn() }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: vi.fn() }));

describe("update channels", () => {
  beforeEach(() => {
    window.localStorage.clear();
    vi.resetAllMocks();
    Object.defineProperty(window, "__TAURI_INTERNALS__", { value: {}, configurable: true });
  });
  afterEach(() => { delete (window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__; });

  it("defaults to stable and persists beta opt in", () => {
    expect(getUpdateChannel()).toBe("stable");
    setUpdateChannel("beta");
    expect(getUpdateChannel()).toBe("beta");
  });

  it("uses the signed plugin's stable check", async () => {
    Object.defineProperty(window, "__TAURI_INTERNALS__", { value: {}, configurable: true });
    const { check } = await import("@tauri-apps/plugin-updater");
    vi.mocked(check).mockResolvedValue(null);
    expect(await checkForUpdate("stable")).toBeNull();
    expect(check).toHaveBeenCalledOnce();
    delete (window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  });

  it("uses the separate native nightly feed", async () => {
    Object.defineProperty(window, "__TAURI_INTERNALS__", { value: {}, configurable: true });
    const { invoke } = await import("@tauri-apps/api/core");
    vi.mocked(invoke).mockResolvedValue({ version: "0.5.10-nightly.20260928", currentVersion: "0.5.9", body: null });
    expect(await checkForUpdate("beta")).toEqual({ version: "0.5.10-nightly.20260928", currentVersion: "0.5.9", body: null, channel: "beta" });
    expect(invoke).toHaveBeenCalledWith("check_nightly_update");
    delete (window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  });

  it("installs the checked nightly version before relaunching", async () => {
    Object.defineProperty(window, "__TAURI_INTERNALS__", { value: {}, configurable: true });
    const { invoke } = await import("@tauri-apps/api/core");
    const { relaunch } = await import("@tauri-apps/plugin-process");
    vi.mocked(invoke).mockResolvedValue({ version: "0.5.10-nightly.20260928", currentVersion: "0.5.9", body: null });
    const update = await checkForUpdate("beta");
    await installUpdateAndRestart(update!);
    expect(invoke).toHaveBeenCalledWith("ensure_update_installable");
    expect(invoke).toHaveBeenCalledWith("install_nightly_update", { version: "0.5.10-nightly.20260928" });
    expect(relaunch).toHaveBeenCalledOnce();
    delete (window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  });

  it("can install the displayed nightly after a later check fails", async () => {
    Object.defineProperty(window, "__TAURI_INTERNALS__", { value: {}, configurable: true });
    const { invoke } = await import("@tauri-apps/api/core");
    const { relaunch } = await import("@tauri-apps/plugin-process");
    vi.mocked(invoke)
      .mockResolvedValueOnce({ version: "0.5.11-nightly.20260928", currentVersion: "0.5.10", body: null })
      .mockRejectedValueOnce("feed temporarily unavailable")
      .mockResolvedValue(undefined);
    const displayed = await checkForUpdate("beta");
    await expect(checkForUpdate("beta")).rejects.toBe("feed temporarily unavailable");
    await installUpdateAndRestart(displayed!);
    expect(invoke).toHaveBeenCalledWith("install_nightly_update", { version: displayed!.version });
    expect(relaunch).toHaveBeenCalledOnce();
    delete (window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  });

  it("refuses to install from an unpackaged development build", async () => {
    const { invoke } = await import("@tauri-apps/api/core");
    const { relaunch } = await import("@tauri-apps/plugin-process");
    vi.mocked(invoke).mockRejectedValueOnce("This development build cannot replace itself safely.");
    await expect(installUpdateAndRestart({ version: "0.5.11-nightly.20260928", currentVersion: "0.5.10", body: null, channel: "beta" }))
      .rejects.toBeInstanceOf(UpdateInstallUnavailableError);
    expect(invoke).toHaveBeenCalledOnce();
    expect(relaunch).not.toHaveBeenCalled();
  });
});


function stableUpdate(version = "0.5.11") {
  return {
    version, currentVersion: "0.5.10", body: "Release notes",
    close: vi.fn().mockResolvedValue(undefined),
    downloadAndInstall: vi.fn().mockResolvedValue(undefined),
  } as unknown as import("@tauri-apps/plugin-updater").Update;
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
}

describe("stable update lifetime and check ordering", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    Object.defineProperty(window, "__TAURI_INTERNALS__", { value: {}, configurable: true });
  });
  afterEach(() => { delete (window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__; });

  it("releases stable discovery resources after copying metadata", async () => {
    const { check } = await import("@tauri-apps/plugin-updater");
    const update = stableUpdate();
    vi.mocked(check).mockResolvedValueOnce(update);
    expect(await checkForUpdate("stable")).toEqual({
      version: "0.5.11", currentVersion: "0.5.10", body: "Release notes", channel: "stable",
    });
    expect(update.close).toHaveBeenCalledOnce();
    expect(update.downloadAndInstall).not.toHaveBeenCalled();
  });

  it.each([null, stableUpdate("0.5.10")])("rejects an older automatic result after a newer manual check (%s)", async oldResult => {
    const { check } = await import("@tauri-apps/plugin-updater");
    const pending = deferred<import("@tauri-apps/plugin-updater").Update | null>();
    const latest = stableUpdate("0.5.12");
    const started = deferred<void>();
    vi.mocked(check).mockImplementationOnce(() => {
      started.resolve();
      return pending.promise;
    }).mockResolvedValueOnce(latest);
    const automatic = checkForUpdate("stable");
    const outcome = automatic.catch(error => error);
    await started.promise;
    expect((await checkForUpdate("stable"))?.version).toBe("0.5.12");
    pending.resolve(oldResult);
    expect(await outcome).toBeInstanceOf(UpdateCheckSupersededError);
    if (oldResult) expect(oldResult.close).toHaveBeenCalledOnce();
    expect(latest.close).toHaveBeenCalledOnce();
  });

  it("releases a changed stable release without installing or restarting", async () => {
    const { check } = await import("@tauri-apps/plugin-updater");
    const { relaunch } = await import("@tauri-apps/plugin-process");
    const current = stableUpdate("0.5.12");
    vi.mocked(check).mockResolvedValueOnce(current);
    await expect(installUpdateAndRestart({ ...current, channel: "stable", version: "0.5.11", body: null }))
      .rejects.toThrow("Update changed");
    expect(current.close).toHaveBeenCalledOnce();
    expect(current.downloadAndInstall).not.toHaveBeenCalled();
    expect(relaunch).not.toHaveBeenCalled();
  });

  it("releases the stable resource when installation fails", async () => {
    const { check } = await import("@tauri-apps/plugin-updater");
    const { relaunch } = await import("@tauri-apps/plugin-process");
    const current = stableUpdate();
    vi.mocked(check).mockResolvedValueOnce(current);
    vi.mocked(current.downloadAndInstall).mockRejectedValueOnce(new Error("download failed"));
    await expect(installUpdateAndRestart({ ...current, body: null, channel: "stable" })).rejects.toThrow("download failed");
    expect(current.close).toHaveBeenCalledOnce();
    expect(relaunch).not.toHaveBeenCalled();
  });

  it("restarts after a successful stable install even if resource cleanup fails", async () => {
    const { check } = await import("@tauri-apps/plugin-updater");
    const { relaunch } = await import("@tauri-apps/plugin-process");
    const current = stableUpdate();
    vi.mocked(check).mockResolvedValueOnce(current);
    vi.mocked(current.close).mockRejectedValueOnce(new Error("resource unavailable"));
    await installUpdateAndRestart({ ...current, body: null, channel: "stable" });
    expect(current.downloadAndInstall).toHaveBeenCalledOnce();
    expect(current.close).toHaveBeenCalledOnce();
    expect(relaunch).toHaveBeenCalledOnce();
  });
});
