// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { bridgeApi } from "./api";
import { resetMockNotes } from "./notesMock";
import { mockRemoveTrailMember, resetMockTrails } from "./trailsMock";

// Trails inside the real App: Continue this trail must never send the first
// message without the brief the person chose.

vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    options: Record<string, unknown> = {};
    open() {}
    loadAddon() {}
    dispose() {}
    onData() { return { dispose() {} }; }
    write() {}
    writeln() {}
  },
}));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit() {} } }));

class MockResizeObserver {
  callback: ResizeObserverCallback;
  constructor(callback: ResizeObserverCallback) { this.callback = callback; }
  observe() { this.callback([{ contentRect: { width: 1280 } } as ResizeObserverEntry], this as unknown as ResizeObserver); }
  unobserve() {}
  disconnect() {}
}

let container: HTMLDivElement;
let root: Root;

async function settle(rounds = 6) {
  for (let i = 0; i < rounds; i++) await act(async () => { await new Promise(resolve => setTimeout(resolve, 0)); });
}
const wait = async (ms: number) => { await act(async () => { await new Promise(resolve => setTimeout(resolve, ms)); }); await settle(2); };
const buttons = (scope: ParentNode = container) => [...scope.querySelectorAll<HTMLButtonElement>("button")];
const byLabel = (label: string, scope: ParentNode = container) => buttons(scope).find(node => node.getAttribute("aria-label") === label || node.textContent?.trim() === label);
async function click(element: Element | undefined) {
  if (!element) throw new Error("nothing to click");
  await act(async () => { element.dispatchEvent(new MouseEvent("click", { bubbles: true })); });
  await settle(2);
}
async function type(field: HTMLTextAreaElement | HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(field), "value")!.set!;
  await act(async () => { setter.call(field, value); field.dispatchEvent(new Event("input", { bubbles: true })); });
}

async function openContinue() {
  const nav = container.querySelector("nav[aria-label='Main navigation']")!;
  await click(buttons(nav).find(item => item.getAttribute("aria-label")?.startsWith("Trails")));
  for (let tries = 0; tries < 100 && !buttons().some(item => item.textContent?.includes("Archive a chat from the sidebar")); tries += 1) await wait(50);
  await click(buttons().find(item => item.textContent?.includes("Archive a chat from the sidebar")));
  for (let tries = 0; tries < 100 && !buttons().some(item => item.textContent?.includes("Continue this trail")); tries += 1) await wait(50);
  await click(buttons().find(item => item.textContent?.includes("Continue this trail")));
  for (let tries = 0; tries < 100 && !document.querySelector("[data-testid=brief-total]"); tries += 1) await wait(50);
  const dialog = document.querySelector<HTMLElement>("[aria-label^='New chat on']")!;
  await type(dialog.querySelector("textarea")!, "Implement the decisions in the selected plan.");
  return dialog;
}

beforeAll(async () => {
  vi.stubGlobal("ResizeObserver", MockResizeObserver);
  await bridgeApi.resetModelProfiles();
});

beforeEach(async () => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const store = new Map<string, string>();
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => { store.set(key, value); },
      removeItem: (key: string) => { store.delete(key); },
      clear: () => store.clear(),
      key: () => null,
      length: 0,
    },
  });
  resetMockNotes();
  resetMockTrails();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

async function mountApp() {
  await act(async () => root.render(<App />));
  await settle();
  const recommended = buttons().find(button => button.textContent === "Use recommended defaults");
  if (recommended) { await click(recommended); await settle(4); }
}

afterEach(() => {
  vi.restoreAllMocks();
  act(() => root?.unmount());
  container?.remove();
});

describe("Continue this trail in the App", () => {
  it("stops before creating the chat when a chosen item left the trail, keeping the text", { timeout: 20_000 }, async () => {
    await mountApp();
    const create = vi.spyOn(bridgeApi, "createConfiguredChat");
    const send = vi.spyOn(bridgeApi, "submitInput");
    const dialog = await openContinue();
    mockRemoveTrailMember("trail-archive", { kind: "note", ref: "note-plan" });
    await click(byLabel("Start chat", dialog));
    await wait(100);
    expect(create).not.toHaveBeenCalled();
    expect(send).not.toHaveBeenCalled();
    expect(dialog.isConnected).toBe(true);
    expect(dialog.textContent).toContain("“Plan: archive action per chat” is no longer on this trail");
    expect(dialog.querySelector<HTMLTextAreaElement>("textarea")!.value).toBe("Implement the decisions in the selected plan.");
    expect(dialog.querySelector("input[aria-label^='Note']")).toBeNull();
  });

  it("does not send the first message when the carry fails after the chat exists", { timeout: 20_000 }, async () => {
    await mountApp();
    vi.spyOn(bridgeApi, "carryTrailBrief").mockRejectedValue(new Error("The trail has no brief item “note:note-plan”. Reload the trail and try again."));
    const send = vi.spyOn(bridgeApi, "submitInput");
    const archive = vi.spyOn(bridgeApi, "archiveChat");
    const create = vi.spyOn(bridgeApi, "createConfiguredChat");
    const dialog = await openContinue();
    await click(byLabel("Start chat", dialog));
    await wait(300);
    expect(create).toHaveBeenCalledTimes(1);
    expect(send).not.toHaveBeenCalled();
    const created = (await create.mock.results[0].value).sessionId;
    expect(archive).toHaveBeenCalledWith(created);
    expect(dialog.isConnected).toBe(true);
    expect(dialog.textContent).toContain("no brief item");
    expect(dialog.querySelector<HTMLTextAreaElement>("textarea")!.value).toBe("Implement the decisions in the selected plan.");
  });
});
