// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { checkForUpdate, getUpdateChannel, installUpdateAndRestart, setUpdateChannel } from "./updater";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: vi.fn() }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: vi.fn() }));

describe("update channels", () => {
  beforeEach(() => { window.localStorage.clear(); vi.clearAllMocks(); });

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
    expect((await checkForUpdate("beta"))?.version).toBe("0.5.10-nightly.20260928");
    expect(invoke).toHaveBeenCalledWith("check_nightly_update");
    delete (window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  });

  it("installs the checked nightly version before relaunching", async () => {
    Object.defineProperty(window, "__TAURI_INTERNALS__", { value: {}, configurable: true });
    const { invoke } = await import("@tauri-apps/api/core");
    const { relaunch } = await import("@tauri-apps/plugin-process");
    vi.mocked(invoke).mockResolvedValue({ version: "0.5.10-nightly.20260928", currentVersion: "0.5.9", body: null });
    await checkForUpdate("beta");
    await installUpdateAndRestart();
    expect(invoke).toHaveBeenCalledWith("install_nightly_update", { version: "0.5.10-nightly.20260928" });
    expect(relaunch).toHaveBeenCalledOnce();
    delete (window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  });
});
