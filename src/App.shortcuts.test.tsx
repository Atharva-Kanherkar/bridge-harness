// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { bridgeApi } from "./api";

// The keymap's own suite proves what a chord means. This one proves the app
// is listening: the bindings are only worth anything mounted, dispatching
// through the real handlers and the api layer's mock backend.

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

let container: HTMLDivElement;
let root: Root;

const settle = () => act(async () => { await new Promise(resolve => setTimeout(resolve, 40)); });

beforeEach(async () => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const store = new Map<string, string>();
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => { store.set(key, String(value)); },
      removeItem: (key: string) => { store.delete(key); },
      clear: () => store.clear(),
      key: () => null,
      length: 0,
    },
  });
  container = document.createElement("div");
  document.body.append(container);
  await act(async () => {
    root = createRoot(container);
    root.render(<App />);
  });
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });
  const recommended = [...container.querySelectorAll("button")]
    .find(button => button.textContent === "Use recommended defaults");
  if (recommended) {
    await act(async () => recommended.click());
    await settle();
  }
});

afterEach(() => {
  vi.restoreAllMocks();
  act(() => root?.unmount());
  container?.remove();
  document.documentElement.removeAttribute("data-fullscreen");
});

// Dispatched on the body rather than the window so the event travels the
// path a real keystroke does: overlays listening on the document see it too.
async function press(init: KeyboardEventInit) {
  await act(async () => {
    document.body.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
  });
}

const composerField = () => [...container.querySelectorAll<HTMLTextAreaElement>("textarea")]
  .find(field => field.placeholder.startsWith("Ask Bridge"));

const onWelcome = () => container.querySelector('button[aria-label="New workspace"]') !== null;

// Turn an open draft into a real chat by sending its first message.
async function sendFirst(text: string) {
  const composer = composerField()!;
  const setValue = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
  await act(async () => { setValue.call(composer, text); composer.dispatchEvent(new Event("input", { bubbles: true })); });
  await act(async () => composer.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })));
  await settle();
}

async function pasteWelcomeImage() {
  const composer = composerField()!;
  const file = new File(["image bytes"], "screenshot.png", { type: "image/png" });
  const event = new Event("paste", { bubbles: true, cancelable: true });
  Object.defineProperty(event, "clipboardData", {
    value: {
      items: {
        0: { kind: "file", type: "image/png", getAsFile: () => file },
        length: 1,
      },
    },
  });
  await act(async () => { composer.dispatchEvent(event); });
  await settle();
}

describe("keyboard shortcuts inside the app", () => {
  it("previews an image pasted into the welcome composer", async () => {
    await pasteWelcomeImage();

    expect(container.querySelector('img[alt="Attached image, image/png"]')).not.toBeNull();
    expect(container.querySelector('button[aria-label="Remove attached image"]')).not.toBeNull();
  });

  it("creates a chat and delivers an image-only first turn", async () => {
    const submitInput = vi.spyOn(bridgeApi, "submitInput");
    await pasteWelcomeImage();

    await act(async () => composerField()!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })));
    await settle();

    expect(onWelcome(), "image-only submit created the selected chat").toBe(false);
    expect(submitInput).toHaveBeenCalledWith(
      expect.any(String),
      "",
      [expect.objectContaining({ mediaType: "image/png", dataUri: expect.stringMatching(/^data:image\/png;base64,/) })],
    );
  });

  it("opens an unstarted draft on ⌘N without the mouse", async () => {
    expect(onWelcome(), "the app mounted on the welcome surface").toBe(true);
    await press({ key: "n", code: "KeyN", metaKey: true });
    await settle();
    // #350: the new-chat shortcut opens a draft — the composer is ready, but no
    // session is created, so the welcome/draft surface is still what's showing.
    expect(composerField(), "the draft composer is focused").not.toBeUndefined();
    expect(onWelcome(), "still an unstarted draft, not a created chat").toBe(true);
  });

  it("opens the shortcuts sheet on ⌘/ and closes it again", async () => {
    await press({ key: "/", code: "Slash", metaKey: true });
    expect(document.body.textContent).toContain("Keyboard shortcuts");
    // Every command is written down, not just the one that opened the sheet.
    expect(document.body.textContent).toContain("Toggle the sidebar");

    await press({ key: "Escape", code: "Escape" });
    await settle();
    expect(document.body.textContent).not.toContain("Toggle the sidebar");
  });

  it("collapses and restores the rail on ⌘B", async () => {
    await press({ key: "b", code: "KeyB", metaKey: true });
    await settle();
    expect(localStorage.getItem("bridge.sidebar.collapsed")).toBe("1");
    await press({ key: "b", code: "KeyB", metaKey: true });
    await settle();
    expect(localStorage.getItem("bridge.sidebar.collapsed")).toBe("0");
  });

  it("never reads the composer's own typing as a command", async () => {
    await press({ key: "n", code: "KeyN", metaKey: true });
    await settle();
    const composer = composerField()!;
    const setValue = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
    await act(async () => {
      setValue.call(composer, "b/n plans, then k");
      composer.dispatchEvent(new Event("input", { bubbles: true }));
    });
    // Bare letters that spell three different commands, typed where they are
    // text and nothing else.
    for (const key of ["b", "n", "k", "/"]) {
      await act(async () => composer.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true })));
    }
    expect(document.body.textContent).not.toContain("Keyboard shortcuts");
    expect(localStorage.getItem("bridge.sidebar.collapsed")).not.toBe("1");
    expect(composer.value).toBe("b/n plans, then k");
  });

  it("keeps the dock and fullscreen chords working from the table", async () => {
    // ⌘N now opens a draft (#350); send a first message so a real chat — with a
    // dock — actually exists for the dock/fullscreen chords to act on.
    await press({ key: "n", code: "KeyN", metaKey: true });
    await settle();
    await sendFirst("open the dock");
    // The dock rail is always mounted; the pane tablist is what open means.
    const docked = () => container.querySelector('[role="tablist"][aria-label="Dock panes"]') !== null;
    const before = docked();

    // ⌥ rewrites the printed character on macOS, so the chord arrives as a
    // symbol and only the code still says which key it was.
    await press({ key: "º", code: "Digit0", metaKey: true, altKey: true });
    await settle();
    expect(docked(), "⌥⌘0 flipped the dock").toBe(!before);

    await press({ key: "º", code: "Digit0", metaKey: true, altKey: true });
    await settle();
    expect(docked(), "and flipped it back").toBe(before);

    await press({ key: "ƒ", code: "KeyF", metaKey: true, altKey: true });
    await settle();
    expect(document.documentElement.hasAttribute("data-fullscreen"), "⌥⌘F squared the layout").toBe(true);
  });
});
