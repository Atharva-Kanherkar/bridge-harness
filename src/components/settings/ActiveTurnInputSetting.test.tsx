// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { ACTIVE_TURN_INPUT_KEY, readActiveTurnInput, useActiveTurnInput, writeActiveTurnInput } from "../../activeTurnSettings";
import { ActiveTurnInputSetting } from "./ActiveTurnInputSetting";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
afterEach(() => { localStorage.clear(); vi.restoreAllMocks(); });

it("defaults to steer and persists queue across fresh reads", () => {
  expect(readActiveTurnInput()).toBe("steer");
  localStorage.setItem(ACTIVE_TURN_INPUT_KEY, "invalid");
  expect(readActiveTurnInput()).toBe("steer");
  writeActiveTurnInput("queue");
  expect(readActiveTurnInput()).toBe("queue");
});

it("updates all mounted consumers immediately and receives cross-window changes", async () => {
  const container = document.createElement("div");
  const root = createRoot(container);
  function ChatMode() { return <output>{useActiveTurnInput()}</output>; }
  try {
    await act(async () => { root.render(<><ActiveTurnInputSetting onError={() => {}} /><ChatMode /><ChatMode /></>); });
    const queue = Array.from(container.querySelectorAll("button")).find(button => button.textContent === "Queue")!;
    await act(async () => { queue.click(); });
    expect(queue.getAttribute("aria-pressed")).toBe("true");
    expect(Array.from(container.querySelectorAll("output")).map(output => output.textContent)).toEqual(["queue", "queue"]);
    expect(localStorage.getItem(ACTIVE_TURN_INPUT_KEY)).toBe("queue");
    expect(container.textContent).toContain("If a provider cannot steer");
    await act(async () => {
      localStorage.setItem(ACTIVE_TURN_INPUT_KEY, "steer");
      window.dispatchEvent(new StorageEvent("storage", { key: ACTIVE_TURN_INPUT_KEY }));
    });
    expect(Array.from(container.querySelectorAll("output")).map(output => output.textContent)).toEqual(["steer", "steer"]);
  } finally { act(() => root.unmount()); }
});

it("reports persistence failure without selecting the unsaved mode", async () => {
  const container = document.createElement("div");
  const root = createRoot(container);
  const onError = vi.fn();
  vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("Storage unavailable"); });
  try {
    await act(async () => { root.render(<ActiveTurnInputSetting onError={onError} />); });
    const queue = Array.from(container.querySelectorAll("button")).find(button => button.textContent === "Queue")!;
    await act(async () => { queue.click(); });
    expect(onError).toHaveBeenCalledWith("Error: Storage unavailable");
    expect(queue.getAttribute("aria-pressed")).toBe("false");
  } finally { act(() => root.unmount()); }
});
