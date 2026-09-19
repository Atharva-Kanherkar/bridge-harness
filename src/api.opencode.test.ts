import { afterEach, expect, it, vi } from "vitest";
import { bridgeApi } from "./api";
const { emit } = vi.hoisted(() => ({ emit: vi.fn().mockResolvedValue(undefined) }));
vi.mock("@tauri-apps/api/event", () => ({ emit, listen: vi.fn() }));
afterEach(() => { vi.unstubAllGlobals(); vi.clearAllMocks(); });

it("asks the native menu host to open OpenCode login in the OS default browser", async () => {
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  await bridgeApi.connectMenuBarOpenCode();
  expect(emit).toHaveBeenCalledWith("bridge-menu-bar-connect-opencode");
});
