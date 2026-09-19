import { afterEach, expect, it, vi } from "vitest";
import { bridgeApi } from "./api";
const { open, emit } = vi.hoisted(() => ({ open: vi.fn().mockResolvedValue(undefined), emit: vi.fn() }));
vi.mock("@tauri-apps/plugin-shell", () => ({ open }));
vi.mock("@tauri-apps/api/event", () => ({ emit, listen: vi.fn() }));
afterEach(() => { vi.unstubAllGlobals(); vi.clearAllMocks(); });

it("opens OpenCode login in the OS default browser without an embedded login event", async () => {
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  await bridgeApi.connectMenuBarOpenCode();
  expect(open).toHaveBeenCalledWith("https://opencode.ai/auth");
  expect(emit).not.toHaveBeenCalled();
});
