// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { bridgeApi } from "./api";

// The welcome surface's "Add project" control, exercised through the real App
// against the api layer's mock backend. It covers which handler the control
// actually gets — the part that was wrong when this lived on the composer's
// `+`: a control labelled "New workspace" that only erased the draft.

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
  HTMLElement.prototype.scrollIntoView = vi.fn();
  vi.spyOn(bridgeApi, "listSlashCommands").mockResolvedValue([
    { name: "composer-review", description: "Review this change", harness: "codex", kind: "skill" },
    { name: "composer-review", description: "Review this change", harness: "claude", kind: "skill" },
    { name: "clear", description: "Clear chat", harness: "bridge", kind: "builtin" },
  ]);
  vi.spyOn(bridgeApi, "getSuggestionSettings").mockResolvedValue({ configured: true, settings: { enabled: true, provider: "codex", model: "test-model" } });
  vi.spyOn(bridgeApi, "suggestCompletion").mockResolvedValue({ suggestion: "", usedFallback: false, fallbackReason: null });
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
  vi.restoreAllMocks();
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

describe("the welcome surface's add-project control inside the app", () => {
  it("opens the folder-first project flow from the welcome surface and leaves the draft alone", async () => {
    const composer = composerField();
    expect(composer, "the app mounted with a composer").not.toBeNull();

    await type(composer!, "keep this draft");
    expect(composerField()!.value).toBe("keep this draft");

    const create = vi.spyOn(bridgeApi, "createWorkspace");
    const connect = vi.spyOn(bridgeApi, "connectWorkspaceFolder");
    // The welcome surface has no conversation and no folder, so the control
    // creates a connected project rather than treating itself as an attachment.
    const add = Array.from(container.querySelectorAll("button")).find(button => button.textContent?.trim() === "Add project");
    expect(add, "the add-project control is present").not.toBeUndefined();
    await act(async () => add!.click());
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });

    expect(create).toHaveBeenCalledWith("project");
    expect(connect).toHaveBeenCalledOnce();
    expect(container.textContent).not.toContain("Workspace name");
    expect(composerField()!.value).toBe("keep this draft");
  });
});

async function openChat() {
  const row = container.querySelector<HTMLButtonElement>('button[title*=" — "]');
  expect(row).toBeTruthy();
  await act(async () => row!.click());
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 50)); });
  const field = [...container.querySelectorAll<HTMLTextAreaElement>("textarea")].find(field => field.placeholder.startsWith("Message Bridge") || field.placeholder.startsWith("Send a follow-up"));
  expect(field).toBeTruthy();
  return field!;
}

const pause = (ms: number) => act(async () => { await new Promise(resolve => setTimeout(resolve, ms)); });
const press = (field: HTMLTextAreaElement, key: string, shiftKey = false) => act(async () => { field.dispatchEvent(new KeyboardEvent("keydown", { key, shiftKey, bubbles: true, cancelable: true })); });

describe("composer discovery and completion", () => {
  it("never recommends skills for ordinary prose", async () => {
    const skills = vi.spyOn(bridgeApi, "skillSuggestions");
    const field = await openChat();
    await type(field, "The skill suggestion does not work");
    await pause(450);
    expect(skills).not.toHaveBeenCalled();
    expect(container.textContent).not.toContain("Available skills for this task");
    expect(container.querySelector("#slash-listbox")).toBeNull();
    expect(field.placeholder).toContain("/ skills & commands");
    expect(field.placeholder).toContain("$ provider");
    expect(field.placeholder).toContain("@ files");
    expect(field.placeholder).toContain("# agents");
  });

  it("inserts an inline skill at the cursor and preserves both sides", async () => {
    const field = await openChat();
    const submit = vi.spyOn(bridgeApi, "submitInput");
    await type(field, "Please use /comp on this change");
    await act(async () => { field.focus(); field.setSelectionRange(16, 16); field.dispatchEvent(new MouseEvent("click", { bubbles: true })); });
    expect(container.querySelector("#slash-listbox")).toBeTruthy();
    expect(container.querySelector("#slash-listbox")!.textContent).toContain("/composer-review");
    expect(container.querySelector("#slash-listbox")!.textContent).not.toContain("/clear");
    await press(field, "Tab");
    await pause(10);
    expect(field.value).toBe("Please use /composer-review on this change");
    expect(field.selectionStart).toBe("Please use /composer-review ".length);
    expect(document.activeElement).toBe(field);
    expect(submit).not.toHaveBeenCalled();
  });

  it("opens leading commands, dismisses with Escape and reopens on new input", async () => {
    const field = await openChat();
    await type(field, "/");
    expect(container.querySelector("#slash-listbox")!.textContent).toContain("/clear");
    await press(field, "Escape");
    expect(container.querySelector("#slash-listbox")).toBeNull();
    await type(field, "use /");
    expect(container.querySelector("#slash-listbox")).toBeTruthy();
    const row = container.querySelector<HTMLButtonElement>("#slash-option-0")!;
    await act(async () => row.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true })));
    expect(field.value).toBe("use /composer-review ");
  });

  it("keeps the provider when selecting a local command in a direct chat", async () => {
    await act(async () => { await bridgeApi.createChat("codex", null, "Composer direct regression"); });
    await pause(30);
    const row = [...container.querySelectorAll<HTMLButtonElement>("button[title]")].find(button => button.title.includes("Composer direct regression"))!;
    await act(async () => row.click());
    await pause(30);
    const field = [...container.querySelectorAll<HTMLTextAreaElement>("textarea")].find(field => field.placeholder.startsWith("Message Bridge"))!;
    const update = vi.spyOn(bridgeApi, "updateChatModel");
    const submit = vi.spyOn(bridgeApi, "submitInput");
    await type(field, "/clear");
    await press(field, "Tab");
    expect(field.value).toBe("/clear ");
    expect(update).not.toHaveBeenCalled();
    expect(submit).not.toHaveBeenCalled();
  });

  it("completes a welcome draft before a session exists", async () => {
    vi.mocked(bridgeApi.suggestCompletion).mockResolvedValue({ suggestion: " release notes", usedFallback: false, fallbackReason: null });
    const field = composerField()!;
    await type(field, "Help me write the");
    await pause(450);
    expect(bridgeApi.suggestCompletion).toHaveBeenCalledWith("Help me write the");
    await press(field, "Tab");
    expect(field.value).toBe("Help me write the release notes");
  });

  it("supports slash discovery in a welcome draft", async () => {
    const field = composerField()!;
    await type(field, "Please use /comp");
    expect(container.querySelector("#welcome-slash-listbox")).toBeTruthy();
    await press(field, "Enter");
    expect(field.value).toBe("Please use /composer-review ");
    expect(bridgeApi.suggestCompletion).not.toHaveBeenCalled();
  });

  it("keeps a slow completion through an unrelated session snapshot update", async () => {
    const field = await openChat();
    let resolve!: (result: Awaited<ReturnType<typeof bridgeApi.suggestCompletion>>) => void;
    vi.mocked(bridgeApi.suggestCompletion).mockImplementation(() => new Promise(done => { resolve = done; }));
    await type(field, "Help me write the");
    await pause(450);
    expect(bridgeApi.suggestCompletion).toHaveBeenCalledOnce();
    await act(async () => { await bridgeApi.savePermissionPolicy((await bridgeApi.configState()).permissionPolicy); });
    await pause(30);
    await act(async () => resolve({ suggestion: " release notes", usedFallback: false, fallbackReason: null }));
    expect(container.textContent).toContain("release notes");
    expect(bridgeApi.suggestCompletion).toHaveBeenCalledOnce();
    await press(field, "Tab");
    expect(field.value).toBe("Help me write the release notes");
  });

  it("shows suggestion failures and recovers on retry without sending", async () => {
    const field = await openChat();
    const submit = vi.spyOn(bridgeApi, "submitInput");
    vi.mocked(bridgeApi.suggestCompletion).mockRejectedValueOnce(new Error("provider unavailable")).mockResolvedValue({ suggestion: " release notes", usedFallback: false, fallbackReason: null });
    await type(field, "Help me write the");
    await pause(450);
    expect(container.textContent).toContain("Inline suggestions unavailable");
    const retry = [...container.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "Retry")!;
    await act(async () => retry.click());
    await pause(450);
    expect(container.textContent).not.toContain("Inline suggestions unavailable");
    expect(container.textContent).toContain("release notes");
    expect(submit).not.toHaveBeenCalled();
  });
});
