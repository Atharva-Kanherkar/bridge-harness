// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { bridgeApi } from "./api";
import type { Workspace } from "./types";

let container: HTMLDivElement;
let root: Root;

const setTextareaValue = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;

async function typeAndSubmit(composer: HTMLTextAreaElement, text: string) {
  await act(async () => {
    setTextareaValue.call(composer, text);
    composer.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => {
    composer.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
  });
}

async function settle() {
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });
}

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
    },
  });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  await act(async () => root.render(<App />));
  await settle();
  const recommended = [...container.querySelectorAll<HTMLButtonElement>("button")]
    .find(button => button.textContent === "Use recommended defaults");
  if (recommended) {
    await act(async () => recommended.click());
    await settle();
  }
});

afterEach(() => {
  vi.restoreAllMocks();
  act(() => root.unmount());
  container.remove();
});

describe("new project folder flow (#351)", () => {
  it("creates and connects a leaf-named project, then reuses it on a repeat pick", async () => {
    const create = vi.spyOn(bridgeApi, "createWorkspace");
    const connect = vi.spyOn(bridgeApi, "connectWorkspaceFolder");
    const projects = container.querySelector<HTMLButtonElement>('button[aria-label="Projects"]')!;
    await act(async () => projects.click());
    await settle();

    const newProject = [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => button.textContent?.includes("New project"))!;
    await act(async () => projects.click());
    await settle();
    const repeatNewProject = [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => button.textContent?.includes("New project"))!;
    await act(async () => repeatNewProject.click());
    await settle();

    expect(create).toHaveBeenCalledWith("project");
    expect(connect).toHaveBeenCalledOnce();
    expect(connect.mock.calls[0][1]).toBe("/Users/you/Developer/project");
    expect(container.textContent).not.toContain("Workspace name");

    await act(async () => newProject.click());
    await settle();
    expect(create).toHaveBeenCalledOnce();
    expect(connect).toHaveBeenCalledOnce();
  });
});

describe("welcome composer resolves a pasted repo directly (#288)", () => {
  it("clones a bare repo URL, clears the composer, and starts the next message in that workspace", async () => {
    const clone = vi.spyOn(bridgeApi, "cloneWorkspaceRepo");
    const createSession = vi.spyOn(bridgeApi, "createWorkspaceSession");
    const composer = container.querySelector<HTMLTextAreaElement>("textarea")!;

    await typeAndSubmit(composer, "https://github.com/rimo/bridge-harness");
    await settle();

    expect(clone).toHaveBeenCalledWith("https://github.com/rimo/bridge-harness");
    // No session exists yet after a clone, so <Welcome> stays mounted — the
    // resolved URL must not linger in the box (#413 review finding 1).
    expect(composer.value).toBe("");

    const clonedState = await clone.mock.results[0]!.value;
    const cloned = (clonedState.workspaces as Workspace[]).find(item => item.title === "bridge-harness");
    expect(cloned).toBeDefined();

    await typeAndSubmit(composer, "what does this repo do?");
    await settle();

    expect(createSession).toHaveBeenCalledWith(cloned!.id, false);
  });

  it("leaves an ordinary message alone", async () => {
    const clone = vi.spyOn(bridgeApi, "cloneWorkspaceRepo");
    const composer = container.querySelector<HTMLTextAreaElement>("textarea")!;
    await typeAndSubmit(composer, "what does this project do?");
    await settle();

    expect(clone).not.toHaveBeenCalled();
  });
});

describe("welcome composer works with no model adapter installed (#413 review finding 2)", () => {
  it("still resolves a pasted repo URL when every adapter is unavailable", async () => {
    const base = await bridgeApi.health();
    vi.spyOn(bridgeApi, "health").mockResolvedValue({
      ...base,
      adapters: base.adapters.map(adapter => ({ ...adapter, available: false, authState: "signed_out" as const })),
    });
    const clone = vi.spyOn(bridgeApi, "cloneWorkspaceRepo");

    const bareContainer = document.createElement("div");
    document.body.append(bareContainer);
    const bareRoot = createRoot(bareContainer);
    await act(async () => bareRoot.render(<App />));
    await settle();

    // No adapters means no chat is possible, but opening a project needs none —
    // the composer must stay typable, unlike before this fix.
    const composer = bareContainer.querySelector<HTMLTextAreaElement>("textarea")!;
    expect(composer.disabled).toBe(false);

    await typeAndSubmit(composer, "https://github.com/rimo/bridge-harness");
    await settle();

    expect(clone).toHaveBeenCalledWith("https://github.com/rimo/bridge-harness");

    act(() => bareRoot.unmount());
    bareContainer.remove();
  });
});
