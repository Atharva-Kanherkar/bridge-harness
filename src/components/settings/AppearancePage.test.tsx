// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { SHOW_WORKER_CHATS_STORAGE_KEY } from "../../missionControlSettings";
import { AppearancePage } from "./AppearancePage";

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  const store = new Map<string, string>();
  vi.stubGlobal("localStorage", { getItem: (key: string) => store.get(key) ?? null, setItem: (key: string, value: string) => store.set(key, value), removeItem: (key: string) => store.delete(key) });
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(() => { act(() => root.unmount()); host.remove(); vi.unstubAllGlobals(); });

const render = async () => act(async () => { root.render(<AppearancePage />); });

it("shows the worker-visibility switch off by default", async () => {
  await render();
  const toggle = host.querySelector<HTMLButtonElement>('[role="switch"][aria-label="Show worker chats in Mission Control"]')!;
  expect(toggle.getAttribute("aria-checked")).toBe("false");
});

it("persists the worker-visibility switch to local storage", async () => {
  await render();
  const toggle = host.querySelector<HTMLButtonElement>('[role="switch"][aria-label="Show worker chats in Mission Control"]')!;
  await act(async () => { toggle.click(); });
  expect(toggle.getAttribute("aria-checked")).toBe("true");
  expect(localStorage.getItem(SHOW_WORKER_CHATS_STORAGE_KEY)).toBe("true");
});
