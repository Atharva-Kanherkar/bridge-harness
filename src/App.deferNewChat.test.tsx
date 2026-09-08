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
const workModeMenu = () => container.querySelector<HTMLButtonElement>('[aria-label^="Work mode:"]');

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

  it("rapid repeat clicks on New Chat produce at most one draft and no session", async () => {
    const creates = spyCreates();
    const newChat = byLabel("New Chat")!;
    await act(async () => { newChat.click(); newChat.click(); });
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });

    expect(totalCreates(creates)).toBe(0);
    // One draft surface, one composer — not two.
    expect(container.querySelectorAll('button[aria-label="New workspace"]').length).toBe(1);
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

  it("offers an interactive model picker on the draft and applies the choice on submit", async () => {
    await act(async () => byLabel("New Chat")!.click());
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });

    // The draft is a real chat-in-waiting: an interactive model control, not a
    // display-only "Balanced" badge.
    const modelButton = [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => (button.getAttribute("aria-label") ?? "").startsWith("Chat model:"));
    expect(modelButton, "the draft exposes an interactive model picker").toBeTruthy();

    const updateModel = vi.spyOn(bridgeApi, "updateChatModel");
    await act(async () => modelButton!.click());
    const opus = [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => (button.textContent ?? "").includes("Claude Opus"));
    expect(opus, "the picker lists more than one model").toBeTruthy();
    await act(async () => opus!.click());
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });

    // The picker stays open across the pick so model and thinking are set in
    // one go; the footer already shows Opus's ladder.
    const max = container.querySelector<HTMLButtonElement>('[data-effort="max"]')!;
    expect(max, "thinking can be selected before the first message").toBeTruthy();
    await act(async () => max.click());

    const composer = composerField();
    await type(composer!, "with opus please");
    await act(async () => pressEnter(composer!));
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 60)); });

    // The chosen model reaches the created session (workspace orchestrator syncs via
    // updateChatModel since create_workspace_session takes no model).
    const call = updateModel.mock.calls.find(c => c[1] === "claude" && c[2] === "opus");
    expect(call, "the draft's chosen harness/model was applied").toBeTruthy();
    expect(call?.[3]).toBe("max");
  });

  it("the worktree decision is held on the draft and applied on submit, not on selection", async () => {
    await act(async () => byLabel("New Chat")!.click());
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });

    const creates = spyCreates();
    const menu = workModeMenu();
    expect(menu, "the draft surface offers work mode choices").toBeTruthy();
    expect(menu!.getAttribute("aria-label")).toBe("Work mode: Work on branch");

    await act(async () => menu!.click());
    const isolated = [...document.querySelectorAll<HTMLButtonElement>('[role="menu"][aria-label="Work mode"] [role="menuitemradio"]')]
      .find(option => option.textContent === "Isolated worktree")!;
    expect(isolated.disabled).toBe(false);
    expect(isolated.getAttribute("aria-checked")).toBe("false");
    await act(async () => isolated.click());
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });

    // Choosing a mode records the decision — it does not start a session.
    expect(totalCreates(creates)).toBe(0);
    expect(workModeMenu()!.getAttribute("aria-label")).toBe("Work mode: Isolated worktree");

    const composer = composerField();
    await type(composer!, "isolate this");
    await act(async () => pressEnter(composer!));
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 60)); });

    expect(creates.workspace).toHaveBeenCalledTimes(1);
    expect(creates.workspace.mock.calls[0][1]).toBe(true); // createWorktree
  });
});
