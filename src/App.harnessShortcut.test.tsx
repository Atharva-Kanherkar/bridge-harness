// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { App } from "./App";

// `$harness message` is a shortcut typed into the composer, not a feature
// with its own button — so it's only trustworthy exercised through the real
// App and the api layer's mock backend, the same way App.composer.test.tsx
// covers the `+` control.

let container: HTMLDivElement;
let root: Root;

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
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 40)); });
  }
});

afterEach(() => {
  act(() => root?.unmount());
  container?.remove();
});

const composerField = () => [...container.querySelectorAll<HTMLTextAreaElement>("textarea")]
  .find(field => field.placeholder.startsWith("Ask Bridge"));

async function type(field: HTMLTextAreaElement, text: string) {
  const setValue = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
  await act(async () => {
    setValue.call(field, text);
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

function pressEnter(field: HTMLTextAreaElement) {
  field.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
}

describe("the $harness composer shortcut inside the app", () => {
  it("starts a new chat pinned to the named harness instead of the welcome default", async () => {
    const composer = composerField();
    expect(composer, "the app mounted with a composer").not.toBeNull();

    await type(composer!, "$claude do you think we are right?");
    await act(async () => pressEnter(composer!));
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 40)); });

    // Landed in a chat, not still on the welcome surface, and the harness
    // control names the harness the shortcut asked for rather than the
    // welcome screen's own default pick.
    expect(Array.from(container.querySelectorAll("button")).find(button => button.textContent?.trim() === "Add project") ?? null).toBeNull();
    expect(container.textContent).toMatch(/Claude/);

    // The shortcut token itself never reached the model — only the message
    // that followed it did, delivered as the chat's first turn.
    expect(container.textContent).not.toContain("$claude");
    expect(container.textContent).toMatch(/do you think we are right\?/);
  });

  it("leaves an unrecognized $token as ordinary text sent to the default chat", async () => {
    const composer = composerField();
    await type(composer!, "$5 is cheaper than I expected");
    await act(async () => pressEnter(composer!));
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 40)); });

    expect(container.textContent).toMatch(/\$5 is cheaper than I expected/);
  });
});
