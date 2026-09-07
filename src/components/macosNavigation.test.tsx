// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AppearancePage } from "./settings/AppearancePage";
import { EffortList } from "./effort/EffortList";

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(() => { act(() => root.unmount()); host.remove(); localStorage.clear(); vi.restoreAllMocks(); });
const key = (element: HTMLElement, value: string) => act(() => { element.dispatchEvent(new KeyboardEvent("keydown", { key: value, bubbles: true, cancelable: true })); });

it("moves and selects appearance radios with arrow, Home, and End keys", () => {
  act(() => root.render(<AppearancePage />));
  const radios = [...host.querySelectorAll<HTMLButtonElement>('[aria-label="Mode"] [role="radio"]')];
  expect(radios.filter(radio => radio.tabIndex === 0)).toHaveLength(1);
  radios[0].focus(); key(radios[0], "ArrowRight");
  expect(radios[1].getAttribute("aria-checked")).toBe("true");
  expect(document.activeElement).toBe(radios[1]);
  key(radios[1], "End"); expect(radios[2].getAttribute("aria-checked")).toBe("true");
  key(radios[2], "Home"); expect(radios[0].getAttribute("aria-checked")).toBe("true");
});

it("navigates reasoning radios and keeps disabled choices inert", () => {
  const onChange = vi.fn();
  const props = { levels: [{ value: "low", label: "Low" }, { value: "high", label: "High" }], value: "low", onChange, harness: "codex", modelLabel: "Test model" };
  act(() => root.render(<EffortList {...props} />));
  const radios = [...host.querySelectorAll<HTMLButtonElement>('[role="radio"]')];
  radios[0].focus(); key(radios[0], "ArrowUp");
  expect(onChange).toHaveBeenLastCalledWith("high"); expect(document.activeElement).toBe(radios[1]);
  key(radios[1], "Home"); expect(onChange).toHaveBeenLastCalledWith("low");
  act(() => root.render(<EffortList {...props} disabled />)); onChange.mockClear();
  key(radios[0], "End"); expect(onChange).not.toHaveBeenCalled();
});
