// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { bridgeApi } from "../../api";
import type { AdapterDescriptor } from "../../types";
import type { LimitRecoverySettings } from "../../protocol/generated/protocol";
import { LimitRecoverySection } from "./LimitRecoverySection";

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(() => { act(() => root.unmount()); host.remove(); vi.restoreAllMocks(); });

const adapters = [
  { id: "claude", label: "Claude", available: true, defaultModel: "opus", models: [
    { id: "opus", label: "Opus", supportedEffortLevels: ["low", "high", "max"] },
    { id: "sonnet", label: "Sonnet", supportedEffortLevels: [] },
  ] },
  { id: "codex", label: "Codex", available: true, models: [{ id: "gpt-5.5", label: "GPT-5.5", supportedEffortLevels: ["medium", "xhigh"] }] },
  { id: "opencode", label: "OpenCode", available: false, models: [{ id: "kimi", label: "Kimi" }] },
] as unknown as AdapterDescriptor[];

async function press(label: string, text: string) {
  const button = [...host.querySelectorAll<HTMLButtonElement>(`[aria-label="${label}"] button`)].find(node => node.textContent === text)!;
  expect(button, `${label}: ${text}`).toBeTruthy();
  await act(async () => { button.click(); });
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 0)); });
}
async function choose(label: string, text: string) {
  await act(async () => host.querySelector<HTMLButtonElement>(`[aria-label="${label}"]`)!.click());
  const option = [...document.querySelectorAll<HTMLElement>(`[role="listbox"][aria-label="${label}"] [role="option"]`)].find(item => item.textContent === text);
  expect(option, `${label}: ${text}`).toBeTruthy();
  await act(async () => { option!.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true })); option!.click(); });
}

async function options(label: string) {
  await act(async () => host.querySelector<HTMLButtonElement>(`[aria-label="${label}"]`)!.click());
  const values = [...document.querySelectorAll<HTMLElement>(`[role="listbox"][aria-label="${label}"] [role="option"]`)].map(item => item.textContent);
  await act(async () => document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
  return values;
}

async function mount(settings: LimitRecoverySettings) {
  vi.spyOn(bridgeApi, "limitRecoverySettings").mockResolvedValue(settings);
  await act(async () => { root.render(<LimitRecoverySection adapters={adapters} />); });
}

it("is off by default and saves resume in one choice", async () => {
  await mount({ mode: "off", backup: null });
  const save = vi.spyOn(bridgeApi, "saveLimitRecoverySettings").mockImplementation(async settings => settings);
  expect(host.querySelector('[aria-label="When a usage limit stops a chat"] [aria-pressed="true"]')?.textContent).toBe("Off");
  expect(host.querySelector('[aria-label="Backup model"]')).toBeNull();
  await press("When a usage limit stops a chat", "Resume");
  expect(save).toHaveBeenCalledWith({ mode: "resume", backup: null });
  expect(host.textContent).toContain("waits for the reset time");
});

it("holds backup mode as a draft until a backup is chosen, then saves model and effort", async () => {
  await mount({ mode: "off", backup: null });
  const save = vi.spyOn(bridgeApi, "saveLimitRecoverySettings").mockImplementation(async settings => settings);
  await press("When a usage limit stops a chat", "Backup model");
  expect(save).not.toHaveBeenCalled();
  expect(host.textContent).toContain("Choose a backup model to turn this on.");
  // Unavailable harnesses are not offered.
  expect(await options("Backup model")).toEqual(["Claude · Opus", "Claude · Sonnet", "Codex · GPT-5.5"]);
  await choose("Backup model", "Codex · GPT-5.5");
  expect(save).toHaveBeenLastCalledWith({ mode: "backup", backup: { harness: "codex", model: "gpt-5.5", effort: null } });
  expect(await options("Backup effort")).toEqual(["Model default", "medium", "xhigh"]);
  await choose("Backup effort", "xhigh");
  expect(save).toHaveBeenLastCalledWith({ mode: "backup", backup: { harness: "codex", model: "gpt-5.5", effort: "xhigh" } });
  // The effort row confirms its own save, like every other control here.
  const effortRow = host.querySelector<HTMLButtonElement>('[aria-label="Backup effort"]')!.closest("[class*='min-h-11']")!;
  expect(effortRow.textContent).toContain("Saved");
});

it("offers only the efforts the setting can store and disables effort for a model without one", async () => {
  await mount({ mode: "backup", backup: { harness: "claude", model: "opus", effort: "high" } });
  expect(await options("Backup effort")).toEqual(["Model default", "low", "high"]);
  vi.spyOn(bridgeApi, "saveLimitRecoverySettings").mockImplementation(async settings => settings);
  await choose("Backup model", "Claude · Sonnet");
  expect(host.querySelector<HTMLButtonElement>('[aria-label="Backup effort"]')!.disabled).toBe(true);
  expect(host.textContent).toContain("This model has no thinking control.");
});

it("shows a refused save and keeps the stored backup visible when the catalog drops it", async () => {
  await mount({ mode: "backup", backup: { harness: "codex", model: "gpt-4-retired", effort: null } });
  expect(host.querySelector('[aria-label="Backup model"]')?.textContent).toContain("gpt-4-retired");
  vi.spyOn(bridgeApi, "saveLimitRecoverySettings").mockRejectedValue(new Error("gpt-4-retired is not a model codex lists; refresh models"));
  await press("When a usage limit stops a chat", "Resume");
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("refresh models");
});
