// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AppearancePage } from "./settings/AppearancePage";
import { EffortControl } from "./effort/EffortControl";

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  // Node's global Storage can shadow jsdom's in Vitest 2. These interactions
  // exercise persisted theme changes, so give each test browser-like storage.
  const values = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (name: string) => values.get(name) ?? null,
    setItem: (name: string, value: string) => { values.set(name, String(value)); },
    removeItem: (name: string) => { values.delete(name); },
    clear: () => values.clear(),
    key: (index: number) => [...values.keys()][index] ?? null,
    get length() { return values.size; },
  } satisfies Storage);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(() => { act(() => root.unmount()); host.remove(); vi.unstubAllGlobals(); vi.restoreAllMocks(); });
const key = (element: HTMLElement, value: string) => act(() => { element.dispatchEvent(new KeyboardEvent("keydown", { key: value, bubbles: true, cancelable: true })); });

it("moves and selects appearance radios with arrow, Home, and End keys", () => {
  act(() => root.render(<AppearancePage />));
  const radios = [...host.querySelectorAll<HTMLButtonElement>('[aria-label="Mode"] [role="radio"]')];
  expect(radios.filter(radio => radio.tabIndex === 0)).toHaveLength(1);
  radios[0].focus(); key(radios[0], "ArrowRight");
  expect(radios[1].getAttribute("aria-checked")).toBe("true");
  expect(localStorage.getItem("bridge.theme")).toBe("light");
  expect(document.activeElement).toBe(radios[1]);
  key(radios[1], "End"); expect(radios[2].getAttribute("aria-checked")).toBe("true");
  key(radios[2], "Home"); expect(radios[0].getAttribute("aria-checked")).toBe("true");
});

it("navigates reasoning radios and keeps disabled choices inert", () => {
  const onChange = vi.fn();
  const props = { levels: [{ value: "low", label: "Low" }, { value: "medium", label: "Med" }, { value: "high", label: "High" }], value: "low", onChange };
  act(() => root.render(<EffortControl {...props} />));
  const radios = [...host.querySelectorAll<HTMLButtonElement>('[role="radio"]')];
  // Arrows wrap, and focus follows the pick.
  radios[0].focus(); key(radios[0], "ArrowLeft");
  expect(onChange).toHaveBeenLastCalledWith("high"); expect(document.activeElement).toBe(radios[2]);
  act(() => root.render(<EffortControl {...props} value="high" />));
  key(radios[2], "ArrowRight"); expect(onChange).toHaveBeenLastCalledWith("low");
  key(radios[2], "Home"); expect(onChange).toHaveBeenLastCalledWith("low");
  key(radios[2], "ArrowLeft"); expect(onChange).toHaveBeenLastCalledWith("medium");
  act(() => root.render(<EffortControl {...props} disabled />)); onChange.mockClear();
  key(radios[0], "End"); expect(onChange).not.toHaveBeenCalled();
});
