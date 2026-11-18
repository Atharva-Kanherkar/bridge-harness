// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { bridgeApi } from "../../api";
import { OpencodeAppsSetting } from "./OpencodeAppsSetting";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
let container: HTMLDivElement, root: Root;
const prefs = (allow?: boolean) => ({ revision: 1, values: allow === undefined ? {} : { allowOpencodeAppCourier: allow } });
beforeEach(() => { container = document.createElement("div"); document.body.append(container); root = createRoot(container); });
afterEach(() => { act(() => root.unmount()); container.remove(); vi.restoreAllMocks(); });
const settle = () => act(async () => { await new Promise(resolve => setTimeout(resolve, 0)); });
const button = (text: string) => [...container.querySelectorAll("button")].find(node => (node.textContent ?? "").includes(text) || node.getAttribute("aria-label") === text);
const toggle = () => container.querySelector<HTMLButtonElement>('[role="switch"], button[aria-label="Allow personal apps through OpenCode"]')!;

it("is off by default, warns, and turns on only after an explicit confirmation", async () => {
  vi.spyOn(bridgeApi, "getAppPreferences").mockResolvedValue(prefs());
  const save = vi.spyOn(bridgeApi, "saveAppPreferences").mockResolvedValue(prefs(true));
  act(() => root.render(<OpencodeAppsSetting />)); await settle();
  expect(container.textContent).toContain("less safe");
  expect(container.textContent).toContain("own discretion");
  act(() => toggle().click()); await settle();
  expect(save).not.toHaveBeenCalled();
  act(() => button("Cancel")!.click()); await settle();
  expect(save).not.toHaveBeenCalled();
  act(() => toggle().click()); await settle();
  act(() => button("I understand, turn on")!.click()); await settle();
  expect(save).toHaveBeenCalledWith({ allowOpencodeAppCourier: true });
});

it("turns off at once", async () => {
  vi.spyOn(bridgeApi, "getAppPreferences").mockResolvedValue(prefs(true));
  const save = vi.spyOn(bridgeApi, "saveAppPreferences").mockResolvedValue(prefs(false));
  act(() => root.render(<OpencodeAppsSetting />)); await settle();
  act(() => toggle().click()); await settle();
  expect(save).toHaveBeenCalledWith({ allowOpencodeAppCourier: false });
});
