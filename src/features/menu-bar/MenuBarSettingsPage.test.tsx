// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { bridgeApi } from "../../api";
import type { MenuBarSettings } from "../../protocol/generated/protocol";
import { MenuBarSettingsPage } from "./MenuBarSettingsPage";

const settings: MenuBarSettings = { schemaVersion: 1, enabled: true, codexEnabled: true,
  displayMode: "remaining", quotaWindow: "session", showAccount: true, showTokens: true, showCost: true, refreshSeconds: 300 };
let root: Root;
let container: HTMLDivElement;
beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
  vi.spyOn(bridgeApi, "getMenuBarSettings").mockResolvedValue(structuredClone(settings));
  vi.spyOn(bridgeApi, "getUsageOverview").mockResolvedValue(null);
  vi.spyOn(bridgeApi, "onUsageOverview").mockResolvedValue(() => undefined);
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.restoreAllMocks(); });
const toggle = () => container.querySelector<HTMLButtonElement>('[aria-label="Show in menu bar"]')!;

it("keeps the saved setting visible until the backend confirms a change", async () => {
  let finish: (value: MenuBarSettings) => void = () => undefined;
  const save = vi.spyOn(bridgeApi, "saveMenuBarSettings").mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  await act(async () => root.render(<MenuBarSettingsPage />));
  await act(async () => toggle().click());
  expect(save).toHaveBeenCalledWith({ ...settings, enabled: false });
  expect(toggle().getAttribute("aria-checked")).toBe("true");
  expect(toggle().disabled).toBe(true);
  await act(async () => finish({ ...settings, enabled: false }));
  expect(toggle().getAttribute("aria-checked")).toBe("false");
  expect(container.querySelector('[role="status"]')?.textContent).toContain("saved");
});

it("retains stored settings and reports a failed save", async () => {
  vi.spyOn(bridgeApi, "saveMenuBarSettings").mockRejectedValue(new Error("Backend unavailable"));
  await act(async () => root.render(<MenuBarSettingsPage />));
  await act(async () => toggle().click());
  expect(toggle().getAttribute("aria-checked")).toBe("true");
  expect(toggle().disabled).toBe(false);
  expect(container.querySelector('[role="alert"]')?.textContent).toContain("Backend unavailable");
});

it("keeps menu controls usable when the usage snapshot fails", async () => {
  vi.mocked(bridgeApi.getUsageOverview).mockRejectedValue(new Error("Usage unavailable"));
  await act(async () => root.render(<MenuBarSettingsPage />));
  expect(toggle().getAttribute("aria-checked")).toBe("true");
  expect(toggle().disabled).toBe(false);
  expect(container.querySelector('[role="alert"]')?.textContent).toContain("Usage unavailable");
});

it("cleans up a subscription that resolves after the settings page closes", async () => {
  const unlisten = vi.fn();
  let finish: (fn: () => void) => void = () => undefined;
  vi.mocked(bridgeApi.onUsageOverview).mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  await act(async () => root.render(<MenuBarSettingsPage />));
  await act(async () => root.render(null));
  await act(async () => finish(unlisten));
  expect(unlisten).toHaveBeenCalledOnce();
});
