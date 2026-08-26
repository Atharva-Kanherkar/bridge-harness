// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { bridgeApi } from "./api";

// #350: New chat must open an *unstarted draft* — no session row is created until the
// first message is submitted. Driven through the real App and the api layer's mock
// backend, spying on the two create calls so "created nothing yet" is exact, not a
// count of rail rows that other tests in this process also mutate.

let container: HTMLDivElement;
let root: Root;

beforeEach(async () => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const store = new Map<string, string>();
  // Pin a last-used repo so a New Chat resolves a workspace draft (with a worktree
  // decision), rather than the no-workspace fallback direct chat.
  store.set("bridge.chat.lastWorkspaceId", "demo-1");
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
  vi.restoreAllMocks();
  act(() => root?.unmount());
  container?.remove();
});

const byLabel = (label: string) => [...container.querySelectorAll<HTMLButtonElement>("button")]
  .find(button => button.getAttribute("aria-label") === label);
const composerField = () => [...container.querySelectorAll<HTMLTextAreaElement>("textarea")]
  .find(field => field.placeholder.startsWith("Ask Bridge"));
const worktreeToggle = () => [...container.querySelectorAll<HTMLButtonElement>("button")]
  .find(button => /On branch|Isolated worktree/.test(button.textContent ?? ""));

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
function spyCreates() {
  return {
    chat: vi.spyOn(bridgeApi, "createChat"),
    workspace: vi.spyOn(bridgeApi, "createWorkspaceSession"),
  };
}
const totalCreates = (s: ReturnType<typeof spyCreates>) => s.chat.mock.calls.length + s.workspace.mock.calls.length;

describe("deferred new-chat creation (#350)", () => {
  it("clicking New Chat creates no session and keeps the draft composer", async () => {
    const creates = spyCreates();
    const newChat = byLabel("New Chat");
    expect(newChat, "the rail exposes a New Chat action").toBeTruthy();

    await act(async () => newChat!.click());
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });

    // No row was written, and we are still on the unstarted draft surface.
    expect(totalCreates(creates)).toBe(0);
    expect(composerField(), "still on the draft composer").toBeTruthy();

    // Navigating away discards the draft silently — still nothing created.
    const projects = byLabel("Projects");
    if (projects) {
      await act(async () => projects.click());
      await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });
    }
    expect(totalCreates(creates)).toBe(0);
  });

  it("the first submitted message creates exactly one session and leaves the draft", async () => {
    await act(async () => byLabel("New Chat")!.click());
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });

    const creates = spyCreates();
    const composer = composerField();
    expect(composer, "draft composer present").toBeTruthy();

    await type(composer!, "start the work");
    await act(async () => pressEnter(composer!));
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 60)); });

    expect(totalCreates(creates)).toBe(1);
    // Left the welcome/draft surface — the welcome-only "New workspace" control is
    // gone because we are now inside the created chat.
    expect(container.querySelector('button[aria-label="New workspace"]')).toBeNull();
  });

  it("an empty submit creates nothing and keeps the draft open", async () => {
    await act(async () => byLabel("New Chat")!.click());
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });

    const creates = spyCreates();
    const composer = composerField();
    // Submit with an empty composer — a chat only exists once it has something in it.
    await act(async () => pressEnter(composer!));
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 40)); });

    expect(totalCreates(creates)).toBe(0);
    expect(composerField(), "still on the draft composer").toBeTruthy();
  });

  it("a rapid double submit still creates only one session", async () => {
    await act(async () => byLabel("New Chat")!.click());
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });

    const creates = spyCreates();
    const composer = composerField();
    await type(composer!, "go go go");
    // Two Enters in the same tick — the second must find the create in flight.
    await act(async () => { pressEnter(composer!); pressEnter(composer!); });
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 60)); });

    expect(totalCreates(creates)).toBe(1);
  });

  it("the worktree decision is held on the draft and applied on submit, not on toggle", async () => {
    await act(async () => byLabel("New Chat")!.click());
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });

    const creates = spyCreates();
    const toggle = worktreeToggle();
    expect(toggle, "the draft surface offers a worktree toggle").toBeTruthy();
    expect(toggle!.getAttribute("aria-pressed")).toBe("false");

    await act(async () => toggle!.click());
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });

    // Toggling records the decision — it does not start a worktree session.
    expect(totalCreates(creates)).toBe(0);
    expect(worktreeToggle()!.getAttribute("aria-pressed")).toBe("true");

    const composer = composerField();
    await type(composer!, "isolate this");
    await act(async () => pressEnter(composer!));
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 60)); });

    expect(creates.workspace).toHaveBeenCalledTimes(1);
    expect(creates.workspace.mock.calls[0][1]).toBe(true); // createWorktree
  });
});
