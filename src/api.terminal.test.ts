import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { bridgeApi } from "./api";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
beforeEach(() => {
  invoke.mockReset().mockResolvedValue({});
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
});
afterEach(() => vi.unstubAllGlobals());

it("supplies the protocol restart default to the embedded Tauri command", async () => {
  await bridgeApi.createTerminal({ workspaceId: "w", terminalId: "pane" });
  expect(invoke).toHaveBeenCalledWith("create_terminal", { workspaceId: "w", terminalId: "pane", restart: false });
});

it("preserves explicit restart and structured agent launch parameters", async () => {
  const params = { workspaceId: "w", terminalId: "pane", restart: true, agentId: "codex", cwd: "/tmp/checkout" };
  await bridgeApi.createTerminal(params);
  expect(invoke).toHaveBeenCalledWith("create_terminal", params);
});
