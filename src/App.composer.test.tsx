// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { bridgeApi } from "./api";

// The composer's `+` control, exercised through the real App against the api
// layer's mock backend. The unit tests in ComposerPill.test.tsx cover what the
// control does with a given handler; this covers which handler it actually gets,
// which is the part that was wrong: a control labelled "New workspace" that only
// erased the draft.

let container: HTMLDivElement;
let root: Root;

beforeEach(async () => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  // jsdom here has no localStorage, and the sidebar reads it during mount.
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
  // Settle health, state, and the model-setup wizard the first run opens with.
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

// By placeholder, not by position: the welcome surface and a connected project
// both render a composer.
const composerField = () => [...container.querySelectorAll<HTMLTextAreaElement>("textarea")]
  .find(field => field.placeholder.startsWith("Ask Bridge"));

/// React tracks an input's value through its own setter, so assigning `.value`
/// directly is invisible to it and the next render puts the old state back.
/// Going through the prototype setter is what makes the change real.
async function type(field: HTMLTextAreaElement, text: string) {
  const setValue = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
  await act(async () => {
    setValue.call(field, text);
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

describe("the composer's + control inside the app", () => {
  it("opens the folder-first project flow from the welcome surface and leaves the draft alone", async () => {
    const composer = composerField();
    expect(composer, "the app mounted with a composer").not.toBeNull();

    await type(composer!, "keep this draft");
    expect(composerField()!.value).toBe("keep this draft");

    const create = vi.spyOn(bridgeApi, "createWorkspace");
    const connect = vi.spyOn(bridgeApi, "connectWorkspaceFolder");
    // The welcome surface has no conversation and no folder, so the control
    // creates a connected project rather than treating itself as an attachment.
    const plus = container.querySelector<HTMLButtonElement>('button[aria-label="New workspace"]');
    expect(plus, "the + control is present").not.toBeNull();
    await act(async () => plus!.click());
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });

    expect(create).toHaveBeenCalledWith("project");
    expect(connect).toHaveBeenCalledOnce();
    expect(container.textContent).not.toContain("Workspace name");
    expect(composerField()!.value).toBe("keep this draft");
  });
});
