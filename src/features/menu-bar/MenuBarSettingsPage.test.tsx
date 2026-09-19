// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { bridgeApi } from "../../api";
import type { MenuBarSettings } from "../../protocol/generated/protocol";
import { MenuBarSettingsPage } from "./MenuBarSettingsPage";

const settings: MenuBarSettings = { schemaVersion: 1, enabled: true, codexEnabled: true, claudeEnabled: false, cursorEnabled: false, opencodeEnabled: false, selectedProvider: "codex", opencodeWorkspace: null,
  displayMode: "remaining", quotaWindow: "session", showAccount: true, showTokens: true, showCost: true, refreshSeconds: 300 };
let root: Root;
let container: HTMLDivElement;
beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
  vi.spyOn(bridgeApi, "getMenuBarSettings").mockResolvedValue(structuredClone(settings));
  vi.spyOn(bridgeApi, "getProviderUsageOverviews").mockResolvedValue(null);
  vi.spyOn(bridgeApi, "onProviderUsageOverviews").mockResolvedValue(() => undefined);
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
  vi.mocked(bridgeApi.getProviderUsageOverviews).mockRejectedValue(new Error("Usage unavailable"));
  await act(async () => root.render(<MenuBarSettingsPage />));
  expect(toggle().getAttribute("aria-checked")).toBe("true");
  expect(toggle().disabled).toBe(false);
  expect(container.querySelector('[role="alert"]')?.textContent).toContain("Usage unavailable");
});

it("cleans up a subscription that resolves after the settings page closes", async () => {
  const unlisten = vi.fn();
  let finish: (fn: () => void) => void = () => undefined;
  vi.mocked(bridgeApi.onProviderUsageOverviews).mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  await act(async () => root.render(<MenuBarSettingsPage />));
  await act(async () => root.render(null));
  await act(async () => finish(unlisten));
  expect(unlisten).toHaveBeenCalledOnce();
});

it("enables Claude without changing the other provider preferences", async () => {
  const save = vi.spyOn(bridgeApi, "saveMenuBarSettings").mockImplementation(async value => value);
  await act(async () => root.render(<MenuBarSettingsPage />));
  for (const provider of ["Codex", "Claude", "Cursor", "OpenCode"]) {
    expect(container.querySelector(`[aria-label="Read ${provider} usage"]`)).not.toBeNull();
  }
  await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="Read Claude usage"]')!.click());
  expect(save).toHaveBeenCalledWith({ ...settings, claudeEnabled: true });
});

it("refreshes the provider group and opens the explicit OpenCode connection flow", async () => {
  const refresh = vi.spyOn(bridgeApi, "refreshProviderUsageOverviews").mockResolvedValue(null);
  const connect = vi.spyOn(bridgeApi, "connectMenuBarOpenCode").mockResolvedValue();
  await act(async () => root.render(<MenuBarSettingsPage />));
  await act(async () => [...container.querySelectorAll("button")].find(b => b.textContent === "Refresh usage")!.click());
  expect(refresh).toHaveBeenCalledOnce();
  await act(async () => [...container.querySelectorAll("button")].find(b => b.textContent === "Connect OpenCode")!.click());
  expect(connect).toHaveBeenCalledOnce();
});

it("shows the result from the native default-browser dispatch", async () => {
  let publish: (message: string) => void = () => undefined;
  vi.spyOn(bridgeApi, "onMenuBarConnection").mockImplementation(async handler => {
    publish = handler;
    return () => undefined;
  });
  await act(async () => root.render(<MenuBarSettingsPage />));
  await act(async () => publish("OpenCode sign-in opened in your default browser. Copy the Go API key, then paste it here."));
  expect(container.textContent).toContain("OpenCode sign-in opened in your default browser");
});

it("composes a two-line icon layout without saving until Apply", async () => {
  const save = vi.spyOn(bridgeApi, "saveMenuBarSettings").mockImplementation(async value => value);
  await act(async () => root.render(<MenuBarSettingsPage />));
  await act(async () => [...container.querySelectorAll("button")].find(b => b.textContent === "Two limits")!.click());
  expect(save).not.toHaveBeenCalled();
  expect(container.querySelector("pre")?.textContent).toBe("▥ 5h 42%\n7d 74%");
  await act(async () => [...container.querySelectorAll("button")].find(b => b.textContent === "Apply layout")!.click());
  expect(save).toHaveBeenCalledWith({ ...settings, statusLayout: [["icon", "space", "fiveHourUsed"], ["weeklyUsed"]] });
});

it("defaults Overview on while preserving the separate status display choice", async () => {
  const save = vi.spyOn(bridgeApi, "saveMenuBarSettings").mockImplementation(async value => value);
  await act(async () => root.render(<MenuBarSettingsPage />));
  const overview = container.querySelector<HTMLButtonElement>('[aria-label="Open to Overview"]')!;
  expect(overview.getAttribute("aria-checked")).toBe("true");
  await act(async () => overview.click());
  expect(save).toHaveBeenCalledWith({ ...settings, openToOverview: false });
});

it("shows the active custom layout and lets Today's spend replace a saved used-quota layout", async () => {
  const custom: MenuBarSettings = { ...settings, displayMode: "cost", statusLayout: [["icon", "space", "used"]] };
  vi.mocked(bridgeApi.getMenuBarSettings).mockResolvedValue(custom);
  const save = vi.spyOn(bridgeApi, "saveMenuBarSettings").mockImplementation(async value => value);
  await act(async () => root.render(<MenuBarSettingsPage />));
  const display = () => container.querySelector<HTMLButtonElement>('[aria-label="Beside the icon"]')!;
  expect(display().textContent).toBe("Custom layout");
  await act(async () => display().click());
  const cost = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(option => option.textContent === "Today's spend")!;
  await act(async () => {
    cost.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
    cost.click();
  });
  expect(save).toHaveBeenCalledWith({ ...custom, displayMode: "cost", statusLayout: [] });
  expect(display().textContent).toBe("Today's spend");
  expect(container.textContent).toContain("Standard display is active.");
});

it("switches to standard display immediately and retains the custom layout if saving fails", async () => {
  const custom: MenuBarSettings = { ...settings, statusLayout: [["icon", "space", "used"]] };
  vi.mocked(bridgeApi.getMenuBarSettings).mockResolvedValue(custom);
  const save = vi.spyOn(bridgeApi, "saveMenuBarSettings").mockRejectedValue(new Error("Save failed"));
  await act(async () => root.render(<MenuBarSettingsPage />));
  await act(async () => [...container.querySelectorAll("button")].find(button => button.textContent === "Use standard display")!.click());
  expect(save).toHaveBeenCalledWith({ ...custom, statusLayout: [] });
  expect(container.querySelector('[aria-label="Beside the icon"]')?.textContent).toBe("Custom layout");
  expect(container.querySelector('[role="alert"]')?.textContent).toContain("Save failed");
});

it("serializes rapid provider changes against the last confirmed settings", async () => {
  const finishes: ((value: MenuBarSettings) => void)[] = [];
  const save = vi.spyOn(bridgeApi, "saveMenuBarSettings").mockImplementation(() => new Promise(resolve => finishes.push(resolve)));
  await act(async () => root.render(<MenuBarSettingsPage />));
  await act(async () => {
    container.querySelector<HTMLButtonElement>('[aria-label="Read Claude usage"]')!.click();
    container.querySelector<HTMLButtonElement>('[aria-label="Read Cursor usage"]')!.click();
  });
  expect(save).toHaveBeenCalledTimes(1);
  await act(async () => finishes[0]({ ...settings, claudeEnabled: true }));
  expect(save).toHaveBeenCalledTimes(2);
  expect(save).toHaveBeenLastCalledWith({ ...settings, claudeEnabled: true, cursorEnabled: true });
  await act(async () => finishes[1]({ ...settings, claudeEnabled: true, cursorEnabled: true }));
  expect(container.querySelector('[aria-label="Read Claude usage"]')!.getAttribute("aria-checked")).toBe("true");
  expect(container.querySelector('[aria-label="Read Cursor usage"]')!.getAttribute("aria-checked")).toBe("true");
});

it("defaults to Codex, Claude and Cursor favorites without connecting their accounts", async () => {
  const save = vi.spyOn(bridgeApi, "saveMenuBarSettings").mockImplementation(async value => value);
  await act(async () => root.render(<MenuBarSettingsPage />));
  for (const [index, name] of ["Codex", "Claude", "Cursor"].entries()) {
    expect(container.querySelector(`[aria-label="Favorite provider ${index + 1}"]`)?.textContent).toBe(name);
  }
  expect(container.querySelector('[aria-label="Read Cursor usage"]')!.getAttribute("aria-checked")).toBe("false");
  expect(save).not.toHaveBeenCalled();
});

it("saves a favorite replacement without changing enabled accounts", async () => {
  const save = vi.spyOn(bridgeApi, "saveMenuBarSettings").mockImplementation(async value => value);
  await act(async () => root.render(<MenuBarSettingsPage />));
  await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="Favorite provider 3"]')!.click());
  const option = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(option => option.textContent === "OpenCode")!;
  expect(option).toBeDefined();
  await act(async () => {
    option.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
    option.click();
  });
  expect(save).toHaveBeenCalledWith({ ...settings, pinnedProviders: ["codex", "claude", "opencode"] });
  expect(container.querySelector('[aria-label="Read OpenCode usage"]')!.getAttribute("aria-checked")).toBe("false");
});

it("shows the first favorite beside the icon independently of the selected detail tab", async () => {
  vi.mocked(bridgeApi.getMenuBarSettings).mockResolvedValue({ ...settings, selectedProvider: "cursor", pinnedProviders: ["claude", "codex", "cursor"] });
  await act(async () => root.render(<MenuBarSettingsPage />));
  const statusProvider = container.querySelector('[aria-label="Provider beside the icon"]')!;
  expect(statusProvider.textContent).toBe("Claude");
  expect(statusProvider.tagName).toBe("SPAN");
  expect(container.textContent).toContain("Switching tabs only changes the open menu");
});

it("persists overview summary visibility and separate icons independently", async () => {
  const save = vi.spyOn(bridgeApi, "saveMenuBarSettings").mockImplementation(async value => value);
  await act(async () => root.render(<MenuBarSettingsPage />));
  const summary = () => container.querySelector<HTMLButtonElement>('[aria-label="Overview usage & spend"]')!;
  const icons = () => container.querySelector<HTMLButtonElement>('[aria-label="Separate provider icons"]')!;
  expect(summary().getAttribute("aria-checked")).toBe("true");
  expect(icons().getAttribute("aria-checked")).toBe("false");
  await act(async () => summary().click());
  expect(save).toHaveBeenLastCalledWith({ ...settings, showOverviewSummary: false });
  await act(async () => icons().click());
  expect(save).toHaveBeenLastCalledWith({ ...settings, showOverviewSummary: false, separateProviderIcons: true });
  expect(summary().getAttribute("aria-checked")).toBe("false");
  expect(icons().getAttribute("aria-checked")).toBe("true");
});

it("limits separate icon owners to enabled favorites as favorites are added and removed", async () => {
  vi.mocked(bridgeApi.getMenuBarSettings).mockResolvedValue({ ...settings, claudeEnabled: true, cursorEnabled: true, opencodeEnabled: true,
    pinnedProviders: ["cursor", "codex", "claude"], separateProviderIcons: true });
  const save = vi.spyOn(bridgeApi, "saveMenuBarSettings").mockImplementation(async value => value);
  await act(async () => root.render(<MenuBarSettingsPage />));
  const owners = () => container.querySelector('[aria-label="Provider beside the icon"]')?.textContent;
  expect(owners()).toBe("Cursor, Codex, Claude");
  await act(async () => [...container.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "Add favorite")!.click());
  expect(owners()).toBe("Cursor, Codex, Claude, OpenCode");
  await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="Favorite provider 4"]')!.click());
  const remove = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(option => option.textContent === "Remove favorite")!;
  await act(async () => {
    remove.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
    remove.click();
  });
  expect(owners()).toBe("Cursor, Codex, Claude");
  expect(save.mock.lastCall?.[0].opencodeEnabled).toBe(true);
  await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="Read Claude usage"]')!.click());
  expect(owners()).toBe("Cursor, Codex");
});

it("adds a fourth favorite without connecting the provider", async () => {
  const save = vi.spyOn(bridgeApi, "saveMenuBarSettings").mockImplementation(async value => value);
  await act(async () => root.render(<MenuBarSettingsPage />));
  expect(container.querySelector('[aria-label="Favorite provider 4"]')).toBeNull();
  const add = [...container.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "Add favorite")!;
  await act(async () => add.click());
  expect(save).toHaveBeenLastCalledWith({ ...settings, pinnedProviders: ["codex", "claude", "cursor", "opencode"] });
  expect(container.querySelector('[aria-label="Favorite provider 4"]')?.textContent).toBe("OpenCode");
  expect(container.querySelector('[aria-label="Read OpenCode usage"]')?.getAttribute("aria-checked")).toBe("false");
  expect(add.disabled).toBe(true);
});

async function pasteOpenCodeKey(value: string) {
  const input = container.querySelector<HTMLInputElement>('[aria-label="OpenCode Go API key"]')!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  return input;
}

it("saves Go through OpenCode's auth API, clears the secret and refreshes enabled usage", async () => {
  vi.mocked(bridgeApi.getMenuBarSettings).mockResolvedValue({ ...settings, opencodeEnabled: true });
  const connect = vi.spyOn(bridgeApi, "setOpenCodeProviderApiKey").mockResolvedValue({ executablePath: "opencode", version: "1.18.3", providers: [] });
  const refresh = vi.spyOn(bridgeApi, "refreshProviderUsageOverviews").mockResolvedValue(null);
  const preferences = vi.spyOn(bridgeApi, "saveMenuBarSettings");
  await act(async () => root.render(<MenuBarSettingsPage />));
  const input = await pasteOpenCodeKey(" test-go-key ");
  expect(input.type).toBe("password");
  await act(async () => input.form!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  expect(connect).toHaveBeenCalledWith("opencode-go", "test-go-key");
  expect(input.value).toBe("");
  expect(refresh).toHaveBeenCalledOnce();
  expect(preferences).not.toHaveBeenCalled();
  expect(container.textContent).toContain("OpenCode Go API key saved");
});

it("does not retain or display API keys when OpenCode connection fails", async () => {
  vi.spyOn(bridgeApi, "setOpenCodeProviderApiKey").mockRejectedValue(new Error("bad request secret-key"));
  const refresh = vi.spyOn(bridgeApi, "refreshProviderUsageOverviews");
  await act(async () => root.render(<MenuBarSettingsPage />));
  const input = await pasteOpenCodeKey("secret-key");
  await act(async () => input.form!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  expect(input.value).toBe("");
  expect(container.querySelector('[role="alert"]')?.textContent).toContain("Could not save the OpenCode Go API key");
  expect(container.textContent).not.toContain("secret-key");
  expect(refresh).not.toHaveBeenCalled();
});

it("explains environment-managed OpenCode credentials without leaking the error payload", async () => {
  vi.spyOn(bridgeApi, "setOpenCodeProviderApiKey").mockRejectedValue(new Error("OPENCODE_AUTH_CONTENT private-details"));
  await act(async () => root.render(<MenuBarSettingsPage />));
  const input = await pasteOpenCodeKey("new-key");
  await act(async () => input.form!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  expect(container.querySelector('[role="alert"]')?.textContent).toContain("Update that environment setting instead");
  expect(container.textContent).not.toContain("private-details");
  expect(input.value).toBe("");
});
