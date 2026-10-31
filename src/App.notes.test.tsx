// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { bridgeApi } from "./api";
import { resetMockNotes, setMockNotesPassDelay } from "./notesMock";

// Notes inside the real App: what a Start from a note hands to the chat path,
// and which defaults the pickers start from.

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
const selectedHarness = (scope: Element) => scope.querySelector("[role='listbox'][aria-label='Chat harness'] [role='option'][aria-selected='true'] [data-option-label]")?.textContent;

async function openNotes() {
  await click(byLabel("Notes", container.querySelector("nav[aria-label='Main navigation']")!));
  await wait(50);
}

/** A page titled `title`, opened. */
async function newPage(title: string) {
  await click(byLabel("New page"));
  await wait(50);
  const field = container.querySelector<HTMLTextAreaElement>("textarea[aria-label='Title']")!;
  await act(async () => { field.dispatchEvent(new FocusEvent("focusin", { bubbles: true })); });
  await type(field, title);
  await act(async () => { field.dispatchEvent(new FocusEvent("focusout", { bubbles: true })); });
  await wait(50);
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
  setMockNotesPassDelay(5);
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

describe("Notes in the App", () => {
  it("starts a chat for a note titled like a command, and the brief reaches the chat", async () => {
    await mountApp();
    const submit = vi.spyOn(bridgeApi, "submitInput");
    await openNotes();
    await newPage("/find syntax");
    await click(byLabel("Start a chat"));
    await click(byLabel("Start", container.querySelector("[aria-label='Start a chat']")!));
    await wait(200);
    expect(submit).toHaveBeenCalledTimes(1);
    expect(submit.mock.calls[0][1]).toContain("/find syntax");
    expect(submit.mock.calls[0][1].startsWith("/")).toBe(false);
  });
});
