// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { bridgeApi } from "./api";
import { acceptApplicationChatDefaults, resetActiveTurnInputForTests } from "./activeTurnSettings";


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

// Reply on an earlier agent message: a banner above the composer, the quote in
// front of the next prompt, and a target that belongs to its chat like a draft.

let container: HTMLDivElement;
let root: Root;

async function settle(rounds = 6) {
  for (let i = 0; i < rounds; i++) {
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 0)); });
  }
}

// Sidebar chat rows, by position: the two demo chats share a title.
const chatRows = () => [...container.querySelectorAll<HTMLButtonElement>('button[data-tip*=" — "]')]
  .filter(row => !(row.dataset.tip ?? "").startsWith("Usage"));
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
  resetActiveTurnInputForTests();
  acceptApplicationChatDefaults({ application: { revision: 0, defaults: {} } });
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

const banner = () => container.querySelector<HTMLElement>("[data-composer-reply]");
const replyButtons = () => [...container.querySelectorAll<HTMLButtonElement>('button[aria-label="Reply"]')];

/** Reply on the n-th settled agent message of the open chat. */
async function replyTo(index: number) {
  const buttons = replyButtons();
  expect(buttons.length, "the chat has agent messages to reply to").toBeGreaterThan(index);
  await click(buttons[index]);
}

const MARKER = /^<!-- bridge-reply[^\n]* -->\n/;

describe("Reply on an agent message", () => {
  it("puts a banner above the composer, focuses it, and sends nothing", async () => {
    await open(chatA);
    const submit = vi.spyOn(bridgeApi, "submitInput");
    await replyTo(0);
    expect(banner()).toBeTruthy();
    expect(banner()!.textContent).toContain("Replying to");
    expect(document.activeElement).toBe(composer());
    await settle(2);
    expect(submit).not.toHaveBeenCalled();
  });

  it("replaces the target on a second Reply, and ✕ clears it", async () => {
    await open(chatA);
    expect(replyButtons().length, "two agent messages in the demo chat").toBeGreaterThanOrEqual(2);
    await replyTo(0);
    const first = banner()!.textContent;
    await replyTo(1);
    expect(container.querySelectorAll("[data-composer-reply]")).toHaveLength(1);
    expect(banner()!.textContent).not.toBe(first);
    await click(banner()!.querySelector('button[aria-label="Cancel reply"]')!);
    expect(banner()).toBeNull();
  });

  it("sends the quote as > lines, a blank line, then the words, and clears the target", async () => {
    await open(chatA);
    const submit = vi.spyOn(bridgeApi, "submitInput");
    await replyTo(0);
    await type("go with the second option");
    await pressEnter();
    await settle(6);
    expect(submit).toHaveBeenCalledOnce();
    const text = submit.mock.calls[0][1];
    expect(text).toMatch(MARKER);
    const [, ...rest] = text.split("\n");
    const blank = rest.indexOf("");
    expect(blank).toBeGreaterThan(0);
    expect(rest.slice(0, blank).every(line => line.startsWith("> "))).toBe(true);
    expect(rest.slice(blank + 1).join("\n")).toBe("go with the second option");
    expect(banner(), "a delivered reply lets go of its target").toBeNull();
    expect(composer()!.value).toBe("");
  });

  it("keeps the target and the words when the send fails", async () => {
    await open(chatA);
    const held = deferred<{ text: string; interceptions: [] }>();
    vi.spyOn(bridgeApi, "prepareTurn").mockReturnValue(held.promise);
    await replyTo(0);
    await type("my answer");
    await pressEnter();
    expect(banner(), "the banner goes with the send").toBeNull();
    await act(async () => { held.reject(new Error("daemon went away")); });
    await settle(4);
    expect(banner(), "and comes back when it fails").toBeTruthy();
    expect(composer()!.value, "the words come back without the quote").toBe("my answer");
  });

  it("lets go of the target on Esc in an empty composer", async () => {
    await open(chatA);
    await replyTo(0);
    await type("not yet");
    await act(async () => { composer()!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })); });
    expect(banner(), "Esc with words typed keeps it").toBeTruthy();
    await type("");
    await act(async () => { composer()!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })); });
    expect(banner()).toBeNull();
  });

  it("does not put the quote on a slash command", async () => {
    await open(chatA);
    const submit = vi.spyOn(bridgeApi, "submitInput");
    vi.spyOn(bridgeApi, "prepareTurn").mockResolvedValue({ text: "/usage", interceptions: [] });
    await replyTo(0);
    await type("/usage");
    await act(async () => { composer()!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })); });
    await pressEnter();
    await settle(6);
    expect(submit).toHaveBeenCalledOnce();
    expect(submit.mock.calls[0][1]).toBe("/usage");
    expect(banner(), "the reply waits for a real message").toBeTruthy();
  });

  it("keeps each chat's target with its draft across a switch", async () => {
    await open(chatA);
    await replyTo(0);
    const target = banner()!.textContent;
    await type("half an answer");
    await open(chatB);
    expect(banner(), "B has no reply").toBeNull();
    await open(chatA);
    expect(banner()?.textContent).toBe(target);
    expect(composer()!.value).toBe("half an answer");
  });
});
