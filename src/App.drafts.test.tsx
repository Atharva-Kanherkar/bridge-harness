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

// jsdom has no Web Animations API; Base UI's dialog asks for running
// animations when it closes.
if (typeof Element !== "undefined" && !Element.prototype.getAnimations) {
  Element.prototype.getAnimations = () => [];
}

class MockResizeObserver {
  callback: ResizeObserverCallback;
  constructor(callback: ResizeObserverCallback) { this.callback = callback; }
  observe() { this.callback([{ contentRect: { width: 1280 } } as ResizeObserverEntry], this as unknown as ResizeObserver); }
  unobserve() {}
  disconnect() {}
}

// One composer used to serve every chat: a half-written message or a pasted
// image followed the user into whichever chat they opened next, and a failed
// send put its words back over whatever had been typed since. These tests hold
// each chat to its own draft.

let container: HTMLDivElement;
let root: Root;

async function settle(rounds = 6) {
  for (let i = 0; i < rounds; i++) {
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 0)); });
  }
}

// Sidebar chat rows, by position: the two demo chats share a title.
const chatRows = () => [...container.querySelectorAll<HTMLButtonElement>('button[title*=" — "]')]
  .filter(row => !row.title.startsWith("Usage"));
/** The chat composer, not a worker's steer box or the welcome composer. */
const composer = () => [...container.querySelectorAll<HTMLTextAreaElement>("textarea")]
  .find(field => field.placeholder.startsWith("Message Bridge") || field.placeholder.startsWith("Send a follow-up"));

async function click(element: Element) {
  await act(async () => { element.dispatchEvent(new MouseEvent("click", { bubbles: true })); });
  await settle(2);
}

async function open(index: number) {
  await click(chatRows()[index]);
  expect(composer(), `chat ${index} has a chat composer`).toBeTruthy();
}

async function type(text: string) {
  const box = composer()!;
  await act(async () => {
    const setter = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, "value")!.set!;
    setter.call(box, text);
    box.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

async function pressEnter() {
  const box = composer()!;
  await act(async () => { box.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })); });
  await settle(1);
}

async function pasteImage(name: string) {
  const file = new File([`bytes-of-${name}`], name, { type: "image/png" });
  const pasteEvent = new Event("paste", { bubbles: true, cancelable: true });
  Object.defineProperty(pasteEvent, "clipboardData", {
    value: { items: [{ kind: "file", type: "image/png", getAsFile: () => file }] },
  });
  await act(async () => { composer()!.dispatchEvent(pasteEvent); });
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 50)); });
}

const attachedImages = () => [...container.querySelectorAll<HTMLImageElement>("img")]
  .filter(img => img.src.startsWith("data:image/png") && img.closest("form, [data-composer], .u-glass") !== null);

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

/** Two chats that each have the chat composer, by sidebar position. */
const chatA = 0;
const chatB = 1;

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
  expect(chatRows().length, "the mock workspace has two chats").toBeGreaterThanOrEqual(2);
});

afterEach(() => {
  vi.restoreAllMocks();
  act(() => root?.unmount());
  container?.remove();
});

describe("drafts belong to their chat", () => {
  it("keeps a draft through A → B → A and never sends it from B", async () => {
    await open(chatA);
    await type("draft for A");
    await open(chatB);
    expect(composer()!.value, "B never had a draft").toBe("");

    const submit = vi.spyOn(bridgeApi, "submitInput");
    await type("y");
    await pressEnter();
    await settle(6);
    expect(submit).toHaveBeenCalledOnce();
    const [sessionB, text, sent] = submit.mock.calls[0];
    expect(text).toBe("y");
    expect(sent ?? []).toHaveLength(0);

    await open(chatA);
    expect(composer()!.value).toBe("draft for A");
    // And the chat it went to was B, not A.
    const prepare = vi.spyOn(bridgeApi, "prepareTurn");
    await pressEnter();
    await settle(4);
    expect(prepare).toHaveBeenCalledOnce();
    expect(prepare.mock.calls[0][0]).not.toBe(sessionB);
  });

  it("keeps a pasted image with its chat", async () => {
    await open(chatA);
    await pasteImage("a.png");
    const inA = composer()!.closest("form")?.querySelectorAll("img").length ?? attachedImages().length;
    expect(inA, "the image is on A's composer").toBeGreaterThan(0);

    const submit = vi.spyOn(bridgeApi, "submitInput");
    await open(chatB);
    const inB = composer()!.closest("form")?.querySelectorAll("img").length ?? 0;
    expect(inB, "B's composer has no image").toBe(0);
    await type("words only");
    await pressEnter();
    await settle(6);
    expect(submit).toHaveBeenCalledOnce();
    expect(submit.mock.calls[0][2] ?? []).toHaveLength(0);

    await open(chatA);
    const back = composer()!.closest("form")?.querySelectorAll("img").length ?? 0;
    expect(back, "the image came back with A").toBe(inA);
  });
});

describe("a failed send never overwrites or loses the user's words", () => {
  it("puts the failed words ahead of what was typed while it failed", async () => {
    await open(chatA);
    const held = deferred<{ text: string; interceptions: [] }>();
    vi.spyOn(bridgeApi, "prepareTurn").mockReturnValue(held.promise);
    await type("prev");
    await pressEnter();
    expect(composer()!.value).toBe("");
    await type("next");

    await act(async () => { held.reject(new Error("daemon went away")); });
    await settle(4);
    expect(composer()!.value).toBe("prev\n\nnext");
  });

  it("returns the words to the chat they came from when another chat is open", async () => {
    await open(chatA);
    const held = deferred<{ text: string; interceptions: [] }>();
    vi.spyOn(bridgeApi, "prepareTurn").mockReturnValue(held.promise);
    await type("prev");
    await pressEnter();

    await open(chatB);
    await type("typing in B");
    await act(async () => { held.reject(new Error("daemon went away")); });
    await settle(4);
    expect(composer()!.value, "B's composer is untouched").toBe("typing in B");

    await open(chatA);
    expect(composer()!.value).toBe("prev");
  });

  it("does not hand a delivered local command back when only the refresh failed", async () => {
    await open(chatA);
    const submit = vi.spyOn(bridgeApi, "submitInput");
    vi.spyOn(bridgeApi, "prepareTurn").mockResolvedValue({ text: "/usage", interceptions: [] });
    vi.spyOn(bridgeApi, "state").mockRejectedValue(new Error("refresh failed"));
    await type("/usage");
    // Discovery owns the first Enter; dismiss it to submit the bare command.
    await act(async () => { composer()!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })); });
    await pressEnter();
    await settle(6);
    expect(submit).toHaveBeenCalledOnce();
    expect(composer()!.value, "nothing to send twice").toBe("");
    expect(container.textContent).toContain("refresh failed");
  });

  it("a sign-in failure restores ahead of newer typing and offers the retry", async () => {
    await open(chatA);
    const held = deferred<{ text: string; interceptions: [] }>();
    const prepare = vi.spyOn(bridgeApi, "prepareTurn").mockReturnValueOnce(held.promise);
    await type("prev");
    await pressEnter();
    await type("next");
    await act(async () => { held.reject(new Error("Not logged in · Please run /login")); });
    await settle(4);
    expect(composer()!.value).toBe("prev\n\nnext");
    expect(prepare).toHaveBeenCalledOnce();
    expect(document.body.textContent).toContain("Sign in to");
  });
});
