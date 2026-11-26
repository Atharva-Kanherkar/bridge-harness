// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { bridgeApi } from "../../api";
import { EarlyCompactionSetting } from "./EarlyCompactionSetting";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
let container: HTMLDivElement, root: Root;
beforeEach(() => { container = document.createElement("div"); document.body.append(container); root = createRoot(container); });
afterEach(() => { act(() => root.unmount()); container.remove(); vi.restoreAllMocks(); });
const settle = () => act(async () => { await new Promise(resolve => setTimeout(resolve, 0)); });
const toggle = () => container.querySelector<HTMLButtonElement>('[role="switch"]')!;
const mount = async () => { act(() => root.render(<EarlyCompactionSetting />)); await settle(); };

it("the switch reads and saves the global value in mock mode", async () => {
  const read = vi.spyOn(bridgeApi, "earlyCompactionSettings");
  const save = vi.spyOn(bridgeApi, "saveEarlyCompactionSettings");
  await mount();
  expect(read).toHaveBeenCalledWith(undefined);
  expect(container.textContent).toContain("make its context smaller");
  expect(container.textContent).toContain("Off by default");
  expect(toggle().getAttribute("aria-checked")).toBe("false");
  expect(toggle().disabled).toBe(false);

  act(() => toggle().click()); await settle();
  expect(save).toHaveBeenCalledWith(null, true);
  expect(toggle().getAttribute("aria-checked")).toBe("true");
  expect(await bridgeApi.earlyCompactionSettings()).toEqual({ enabled: true, globalEnabled: true, sessionOptedOut: false, sessionEligible: false });

  // A new mount reads the saved value back, and turning it off saves false.
  act(() => root.unmount()); root = createRoot(container);
  await mount();
  expect(toggle().getAttribute("aria-checked")).toBe("true");
  act(() => toggle().click()); await settle();
  expect(save).toHaveBeenLastCalledWith(null, false);
  expect(toggle().getAttribute("aria-checked")).toBe("false");
  expect((await bridgeApi.earlyCompactionSettings()).globalEnabled).toBe(false);
});

it("shows a refused save and keeps the last value", async () => {
  vi.spyOn(bridgeApi, "saveEarlyCompactionSettings").mockRejectedValue(new Error("the store is read-only"));
  await mount();
  act(() => toggle().click()); await settle();
  expect(container.querySelector('[role="alert"]')?.textContent).toBe("The store is read-only.");
  expect(toggle().getAttribute("aria-checked")).toBe("false");
});
