// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { bridgeApi } from "./api";
import { validateBrowserSelectionPage } from "./browserRuntime";
import type { BrowserSelectionContext } from "./browserSelection";

vi.mock("./browserRuntime", () => ({ validateBrowserSelectionPage: vi.fn() }));

const browser = vi.hoisted(() => ({ props: undefined as undefined | { sessionId: string; onAttachSelection: (context: BrowserSelectionContext) => void; onInvalidateSelection: (tabId: string, navigationId?: number) => void } }));
vi.mock("./components/SimpleBrowser", () => ({ SimpleBrowser: (props: typeof browser.props) => { browser.props = props; return null; } }));

// xterm paints to canvas, which jsdom lacks; the pane only needs to mount here.
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
  for (let i = 0; i < rounds; i++) {
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 0)); });
  }
}

const chatRows = () => [...container.querySelectorAll<HTMLButtonElement>('button[title*=" — "]')];
const composer = () => container.querySelector<HTMLTextAreaElement>("textarea");

async function click(element: Element) {
  await act(async () => { element.dispatchEvent(new MouseEvent("click", { bubbles: true })); });
  await settle(2);
}

/** Open the idle demo session (no turn in flight), identified by its dirty-file pill. */
async function openIdleSession() {
  for (const row of chatRows()) {
    await click(row);
    if (container.textContent?.includes("7 files")) return;
  }
  throw new Error("no idle session showed up");
}

async function send(text: string) {
  const box = composer()!;
  await act(async () => {
    const setter = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, "value")!.set!;
    setter.call(box, text);
    box.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => { box.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })); });
  // One microtask flush only: the daemon has not answered anything yet.
  await settle(1);
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(r => { resolve = r; });
  return { promise, resolve };
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
  vi.mocked(validateBrowserSelectionPage).mockReset().mockResolvedValue(true);
  browser.props = undefined;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  await act(async () => root.render(<App />));
  await settle();
  const recommended = [...container.querySelectorAll("button")].find(button => button.textContent === "Use recommended defaults");
  if (recommended) { await click(recommended); await settle(4); }
  await openIdleSession();
  await click(container.querySelector('button[aria-label="Session actions"]')!);
  const item = [...document.querySelectorAll('[role="menu"] [role="menuitemcheckbox"]')].find(node => node.textContent?.includes("Browser"))!;
  await click(item);
  expect(browser.props).toBeDefined();
});

afterEach(() => {
  vi.restoreAllMocks();
  act(() => root?.unmount());
  container?.remove();
});


function selectElement() {
  const props = browser.props!;
  act(() => props.onAttachSelection({ id: "page-selection", sessionId: props.sessionId, tabId: "preview", navigationId: 1,
    url: "http://localhost:3000/page", title: "Preview", selector: "main > button", snippet: "<button>Save</button>",
    bounds: { x: 10, y: 10, width: 80, height: 30 }, annotations: [{ kind: "note", text: "Make this blue" }],
  }));
}
const selectionChip = () => container.querySelector('[aria-label="Attached browser selections"]');

describe("browser selection delivery", () => {
  it("delivers reviewed context through normal input and consumes it only after acceptance", async () => {
    selectElement();
    const held = deferred<Awaited<ReturnType<typeof bridgeApi.submitInput>>>();
    const submit = vi.spyOn(bridgeApi, "submitInput").mockReturnValue(held.promise);
    await send("Make this blue");
    await settle();
    expect(submit).toHaveBeenCalledOnce();
    expect(submit.mock.calls[0][0]).toBe(browser.props!.sessionId);
    expect(submit.mock.calls[0][1]).toContain("untrusted page content");
    expect(submit.mock.calls[0][1]).toContain("main > button");
    expect(submit.mock.calls[0][2]).toEqual([]);
    expect(selectionChip()).not.toBeNull();
    await act(async () => held.resolve({ disposition: "startedNewTurn", interceptions: [] }));
    await settle();
    expect(selectionChip()).toBeNull();
  });

  it("preserves the draft and selection on provider rejection", async () => {
    selectElement();
    vi.spyOn(bridgeApi, "submitInput").mockRejectedValue(new Error("Provider unavailable"));
    await send("Make this blue");
    await settle();
    expect(composer()!.value).toBe("Make this blue");
    expect(selectionChip()).not.toBeNull();
  });

  it("does not route selected context through a command or another harness", async () => {
    selectElement();
    const submit = vi.spyOn(bridgeApi, "submitInput");
    await send("$codex change this");
    expect(submit).not.toHaveBeenCalled();
    expect(composer()!.value).toBe("$codex change this");
    expect(selectionChip()).not.toBeNull();
    expect(container.textContent).toContain("Remove the browser selection before using a command or shortcut");
  });

  it("rejects a selection invalidated while prompt preparation is in flight", async () => {
    selectElement();
    const held = deferred<{ text: string; interceptions: [] }>();
    vi.spyOn(bridgeApi, "prepareTurn").mockReturnValue(held.promise);
    const submit = vi.spyOn(bridgeApi, "submitInput");
    await send("Make this blue");
    act(() => browser.props!.onInvalidateSelection("preview", 2));
    await act(async () => held.resolve({ text: "Make this blue", interceptions: [] }));
    await settle();
    expect(submit).not.toHaveBeenCalled();
    expect(composer()!.value).toBe("Make this blue");
    expect(selectionChip()).toBeNull();
  });
  it("drops prior-task context and ignores a late selection from that task", async () => {
    selectElement();
    const old = browser.props!;
    for (const row of chatRows()) {
      await click(row);
      if (browser.props?.sessionId !== old.sessionId) break;
    }
    expect(browser.props?.sessionId).not.toBe(old.sessionId);
    act(() => old.onAttachSelection({ id: "late", sessionId: old.sessionId, tabId: "preview", navigationId: 1,
      url: "http://localhost:3000/", title: "Old task", selector: "button", snippet: "<button>Save</button>",
      bounds: { x: 0, y: 0, width: 80, height: 30 }, annotations: [],
    }));
    expect(selectionChip()).toBeNull();
  });

  it("rejects a changed native page even before the UI polling catches up", async () => {
    selectElement();
    vi.mocked(validateBrowserSelectionPage).mockResolvedValueOnce(true).mockResolvedValueOnce(false);
    const submit = vi.spyOn(bridgeApi, "submitInput");
    await send("Make this blue");
    await settle();
    expect(validateBrowserSelectionPage).toHaveBeenCalledTimes(2);
    expect(submit).not.toHaveBeenCalled();
    expect(composer()!.value).toBe("Make this blue");
  });

});
