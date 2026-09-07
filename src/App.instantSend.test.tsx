// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { bridgeApi } from "./api";

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

// Pressing Send must be felt at once. Before this, the user's bubble, the
// working state, and the Stop control all waited on the first daemon
// round-trip — on a slow turn start the app looked like it had ignored the
// keystroke. These tests hold that round-trip open and check the UI reacted
// anyway.

let container: HTMLDivElement;
let root: Root;

async function settle(rounds = 6) {
  for (let i = 0; i < rounds; i++) {
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 0)); });
  }
}

const chatRows = () => [...container.querySelectorAll<HTMLButtonElement>('button[title*=" — "]')];
const composer = () => container.querySelector<HTMLTextAreaElement>("textarea");
const stopButton = () => container.querySelector<HTMLButtonElement>('button[aria-label="Stop"], button[aria-label="Stopping…"]');

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

/** Live agent events reach the App through a 50 ms batching timer; wait it out. */
async function waitForEvents() {
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 80)); });
  await settle(2);
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
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  await act(async () => root.render(<App />));
  await settle();
  const recommended = [...container.querySelectorAll("button")].find(button => button.textContent === "Use recommended defaults");
  if (recommended) { await click(recommended); await settle(4); }
  await openIdleSession();
});

afterEach(() => {
  vi.restoreAllMocks();
  act(() => root?.unmount());
  container?.remove();
});

describe("sending a message reacts before the daemon answers", () => {
  it("shows the user's bubble and the Stop control while prepareTurn is still pending", async () => {
    const held = deferred<{ text: string; interceptions: [] }>();
    vi.spyOn(bridgeApi, "prepareTurn").mockReturnValue(held.promise);
    const submit = vi.spyOn(bridgeApi, "submitInput");

    await send("are you there?");

    expect(container.textContent).toContain("are you there?");
    expect(composer()!.value).toBe("");
    expect(stopButton(), "Stop is reachable before the turn is acknowledged").not.toBeNull();
    expect(submit).not.toHaveBeenCalled();

    held.resolve({ text: "are you there?", interceptions: [] });
    await settle(4);
    await waitForEvents();
    expect(submit).toHaveBeenCalledOnce();
    // Once the durable user turn lands, the optimistic bubble hands over: after
    // its exit animation only one "are you there?" row remains in the transcript.
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 600)); });
    const transcript = container.querySelector("[data-conversation-content]")!;
    expect(transcript.textContent!.split("are you there?").length - 1).toBe(1);
  });

  it("keeps a single bubble when preparation rewrites the text", async () => {
    vi.spyOn(bridgeApi, "prepareTurn").mockResolvedValue({ text: "hello [secret:sec_1]", interceptions: [] });
    await send("hello sk-live-secret");
    await settle(4);
    expect(container.textContent).toContain("hello [secret:sec_1]");
    expect(container.textContent).not.toContain("sk-live-secret");
  });

  it("holds a Stop pressed before the turn exists, and clears it once the turn settles", async () => {
    const interrupt = vi.spyOn(bridgeApi, "interruptTurn").mockResolvedValue(undefined);
    const realSubmit = bridgeApi.submitInput;
    const gate = deferred<void>();
    vi.spyOn(bridgeApi, "submitInput").mockImplementation(async (...args) => {
      await gate.promise;
      return realSubmit(...args);
    });

    await send("long task please");
    const stop = stopButton();
    expect(stop).not.toBeNull();
    await click(stop!);
    // Nothing to interrupt yet: the request is held, not dropped, and the
    // control says so instead of snapping back to Send.
    expect(interrupt).not.toHaveBeenCalled();
    expect(stopButton()?.getAttribute("aria-label")).toBe("Stopping…");

    // The mock backend runs the whole turn inside submitInput, so the turn is
    // acknowledged and completed in one step; the held request has nothing left
    // to interrupt and the control must not stay stuck on "Stopping…".
    gate.resolve();
    await settle(6);
    await waitForEvents();
    expect(stopButton()).toBeNull();
    expect(container.textContent).toContain("long task please");
  });
});
