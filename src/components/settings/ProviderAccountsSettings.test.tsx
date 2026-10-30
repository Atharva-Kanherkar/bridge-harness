// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../../api";
import type { AdapterDescriptor, HarnessConfig } from "../../types";
import { HarnessesPage } from "./HarnessesPage";

const LABELS = { claude: "Claude Code", codex: "Codex", opencode: "OpenCode", cursor: "Cursor", grok: "Grok Build" } as const;
function harness(id: keyof typeof LABELS, advanced: Record<string, unknown> = {}) {
  return { id, label: LABELS[id], enabled: true, defaultModel: null, effort: null, systemPrompt: "", advanced, isOverride: false } as HarnessConfig;
}
const adapter = (id: string) => ({ id, label: id, available: true, authState: "unknown", version: "1", capabilities: ["messages"], sandboxModes: ["workspace_write"], unavailableReason: null, defaultModel: null, models: [] }) as unknown as AdapterDescriptor;

async function mount(config: HarnessConfig) {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const onSave = vi.fn().mockResolvedValue(undefined);
  const container = document.createElement("div"); document.body.append(container); const root = createRoot(container);
  await act(async () => root.render(<HarnessesPage detailId={config.id} onOpenDetail={vi.fn()} onCloseDetail={() => undefined}
    harnesses={[config]} adapters={[adapter(config.id)]} modelOptions={[]}
    managed={{ agents: [], listError: null, busy: {}, errors: {}, reload: vi.fn(), install: vi.fn(), update: vi.fn(), repair: vi.fn(), requestRemove: vi.fn(), confirmation: null }}
    busy={false} drafts={{}} onDraft={vi.fn()} onDiscard={vi.fn()} onSaveHarness={onSave} onResetHarness={vi.fn()}
    onCatalog={vi.fn()} onError={vi.fn()} onAuthenticationChanged={vi.fn()} />));
  const button = (text: string) => [...container.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent === text);
  const input = (label: string) => container.querySelector<HTMLInputElement>(`input[aria-label="${label}"]`)!;
  const type = async (label: string, value: string) => act(async () => {
    const field = input(label);
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(field, value);
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
  return { container, onSave, button, input, type, unmount: () => act(async () => { root.unmount(); container.remove(); }) };
}

afterEach(() => vi.restoreAllMocks());

describe("ProviderAccountsSettings", () => {
  it("adds a Claude account with its sign-in command and keeps other advanced keys", async () => {
    const view = await mount(harness("claude", { workerCredentialSource: "auto" }));
    expect(view.container.textContent).toContain("Your own login");
    await act(async () => view.button("Add account")!.click());
    await view.type("Account name", "Work Plan");
    expect(view.input("Account folder").value).toBe("~/.claude_work_plan");
    expect(view.container.querySelector('[aria-label="Sign-in command"]')!.textContent)
      .toBe("mkdir -p ~/.claude_work_plan && CLAUDE_CONFIG_DIR=~/.claude_work_plan claude auth login");
    expect(view.container.textContent).not.toContain("Share chats with your main Codex");
    const add = [...view.container.querySelectorAll<HTMLButtonElement>("button")].filter(item => item.textContent === "Add account").at(-1)!;
    await act(async () => add.click());
    expect(view.onSave).toHaveBeenCalledWith(expect.objectContaining({
      id: "claude",
      advanced: { workerCredentialSource: "auto", accounts: [{ id: "work-plan", label: "Work Plan", homePath: "~/.claude_work_plan" }] },
    }));
    await view.unmount();
  });

  it("adds a Codex account as a shadow home by default, or a separate home", async () => {
    const view = await mount(harness("codex"));
    await act(async () => view.button("Add account")!.click());
    expect(view.container.querySelector('[aria-label="Sign-in command"]')!.textContent).toContain("CODEX_HOME=~/.codex_personal codex login");
    const submit = () => [...view.container.querySelectorAll<HTMLButtonElement>("button")].filter(item => item.textContent === "Add account").at(-1)!;
    await act(async () => submit().click());
    expect(view.onSave).toHaveBeenLastCalledWith(expect.objectContaining({
      advanced: { accounts: [{ id: "personal", label: "Personal", shadowHomePath: "~/.codex_personal" }] },
    }));
    await view.unmount();

    const separate = await mount(harness("codex"));
    await act(async () => separate.button("Add account")!.click());
    await act(async () => separate.container.querySelector<HTMLButtonElement>('[aria-label="Share chats with your main Codex"]')!.click());
    await act(async () => [...separate.container.querySelectorAll<HTMLButtonElement>("button")].filter(item => item.textContent === "Add account").at(-1)!.click());
    expect(separate.onSave).toHaveBeenLastCalledWith(expect.objectContaining({
      advanced: { accounts: [{ id: "personal", label: "Personal", homePath: "~/.codex_personal" }] },
    }));
    await separate.unmount();
  });

  it("refuses a relative folder before saving", async () => {
    const view = await mount(harness("claude"));
    await act(async () => view.button("Add account")!.click());
    await view.type("Account folder", "relative/dir");
    expect(view.container.querySelector('[role="alert"]')!.textContent).toContain("starts with / or ~/");
    const add = [...view.container.querySelectorAll<HTMLButtonElement>("button")].filter(item => item.textContent === "Add account").at(-1)!;
    expect(add.disabled).toBe(true);
    await view.unmount();
  });

  it("removes an account and drops the key when none are left", async () => {
    const view = await mount(harness("claude", { accounts: [{ id: "work", label: "Work", homePath: "~/.claude_work" }], workerCredentialSource: "auto" }));
    expect(view.container.textContent).toContain("~/.claude_work");
    await act(async () => view.container.querySelector<HTMLButtonElement>('[aria-label="Remove Work"]')!.click());
    expect(view.onSave).toHaveBeenCalledWith(expect.objectContaining({ advanced: { workerCredentialSource: "auto" } }));
    await view.unmount();
  });

  it("does not offer accounts on other agents", async () => {
    const view = await mount(harness("grok"));
    expect(view.container.textContent).not.toContain("Your own login");
    await view.unmount();
  });

  const submit = (container: HTMLElement) => [...container.querySelectorAll<HTMLButtonElement>("button")].filter(item => item.textContent === "Add account").at(-1)!;

  it("adds an OpenCode account in its own data folder and connects its Go key there", async () => {
    const setKey = vi.spyOn(bridgeApi, "setOpenCodeProviderApiKey").mockResolvedValue({} as never);
    const view = await mount(harness("opencode", { visibleModels: ["opencode-go/kimi"] }));
    await act(async () => view.button("Add account")!.click());
    expect(view.input("Account folder").value).toBe("~/.opencode_personal");
    expect(view.container.textContent).toContain("XDG_DATA_HOME");
    expect(view.container.querySelector('[aria-label="Sign-in command"]')).toBeNull();
    expect(submit(view.container).disabled, "the Go key is optional").toBe(false);
    await view.type("OpenCode Go API key", "sk-go-synthetic");
    await act(async () => submit(view.container).click());
    expect(view.onSave).toHaveBeenCalledWith(expect.objectContaining({
      advanced: { visibleModels: ["opencode-go/kimi"], accounts: [{ id: "personal", label: "Personal", homePath: "~/.opencode_personal" }] },
    }));
    expect(setKey).toHaveBeenCalledWith("opencode-go", "sk-go-synthetic", undefined, "personal");
    await view.unmount();
  });

  it("adds a Cursor account with its own API key and keeps the key out of settings", async () => {
    const setKey = vi.spyOn(bridgeApi, "setAccountApiKey").mockResolvedValue({} as never);
    const view = await mount(harness("cursor"));
    await act(async () => view.button("Add account")!.click());
    expect(view.container.querySelector('input[aria-label="Account folder"]')).toBeNull();
    expect(submit(view.container).disabled, "Cursor needs a key").toBe(true);
    expect(view.container.querySelector('[role="alert"]')!.textContent).toContain("Paste the Cursor API key");
    await view.type("Cursor API key", "key_cursor_synthetic");
    await act(async () => submit(view.container).click());
    expect(view.onSave).toHaveBeenCalledWith(expect.objectContaining({ advanced: { accounts: [{ id: "personal", label: "Personal" }] } }));
    expect(JSON.stringify(view.onSave.mock.calls)).not.toContain("key_cursor_synthetic");
    expect(setKey).toHaveBeenCalledWith("cursor", "personal", "key_cursor_synthetic");
    await view.unmount();
  });

  it("replaces an account's key, and shows a refused key in the group", async () => {
    const setKey = vi.spyOn(bridgeApi, "setAccountApiKey").mockRejectedValueOnce(new Error("Bridge could not save the key in the macOS Keychain")).mockResolvedValue({} as never);
    const view = await mount(harness("cursor", { accounts: [{ id: "team", label: "Team" }] }));
    expect(view.container.textContent).toContain("Signs in with its own API key");
    const open = () => view.container.querySelector<HTMLButtonElement>('[aria-label="Set Cursor API key for Team"]')!;
    const saveKey = () => [...view.container.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent === "Save key")!;
    await act(async () => open().click());
    await view.type("Team: Cursor API key", "key_one");
    await act(async () => saveKey().click());
    expect(view.container.querySelector('[role="alert"]')!.textContent).toContain("Team: Bridge could not save the key");
    await view.type("Team: Cursor API key", "key_two");
    await act(async () => saveKey().click());
    expect(setKey).toHaveBeenLastCalledWith("cursor", "team", "key_two");
    expect(view.container.querySelector('[role="alert"]')).toBeNull();
    expect(view.container.textContent).toContain("Saved");
    await view.unmount();
  });
});
