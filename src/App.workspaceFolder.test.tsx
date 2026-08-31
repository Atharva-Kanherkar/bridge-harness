// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { bridgeApi } from "./api";

let container: HTMLDivElement;
let root: Root;

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
  const setValue = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;

  it("clones a bare repo URL instead of sending it as a chat message", async () => {
    const clone = vi.spyOn(bridgeApi, "cloneWorkspaceRepo");
    const composer = container.querySelector<HTMLTextAreaElement>("textarea")!;
    await act(async () => {
      setValue.call(composer, "https://github.com/rimo/bridge-harness");
      composer.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => {
      composer.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    });
    await settle();

    expect(clone).toHaveBeenCalledWith("https://github.com/rimo/bridge-harness");
  });

  it("leaves an ordinary message alone", async () => {
    const clone = vi.spyOn(bridgeApi, "cloneWorkspaceRepo");
    const composer = container.querySelector<HTMLTextAreaElement>("textarea")!;
    await act(async () => {
      setValue.call(composer, "what does this project do?");
      composer.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => {
      composer.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    });
    await settle();

    expect(clone).not.toHaveBeenCalled();
  });
});
