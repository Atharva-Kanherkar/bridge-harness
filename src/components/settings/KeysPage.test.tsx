// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../../api";
import type { ApiKeysSnapshot } from "../../types";
import { KeysPage } from "./KeysPage";

// Contract: testing/feat-credential-vault.md, UI. Most cases drive the mock api
// itself, so a key that is still listed after a remount proved the round trip.

let host: HTMLDivElement;
let root: Root;
const onError = vi.fn();

async function mount() {
  await act(async () => { root.render(<KeysPage onError={onError} />); });
}

const button = (label: string) => host.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)
  ?? [...host.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent === label)!;
const input = (label: string) => host.querySelector<HTMLInputElement>(`input[aria-label="${label}"]`)!;

async function type(label: string, value: string) {
  const field = input(label);
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  await act(async () => {
    setter.call(field, value);
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
}
const click = async (label: string) => { await act(async () => button(label).click()); };

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  onError.mockReset();
});

afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  vi.restoreAllMocks();
});

describe("API keys settings", () => {
  it("lists saved keys with their sites and pasted keys with their chat", async () => {
    await mount();
    expect(host.textContent).toContain("OPENAI_API_KEY");
    expect(host.textContent).toContain("OpenAI key · api.openai.com");
    expect(host.textContent).toContain("GitHub token in Triage open issues");
    expect(host.textContent).toContain("api.github.com, uploads.github.com");
    expect(host.textContent).toContain("Forgotten when Bridge quits");
  });

  it("fills in a recognized key's usual name and site, then saves it", async () => {
    await mount();
    expect(button("Save key").disabled).toBe(true);
    await type("Key value", "xoxb-not-a-real-token");
    expect(input("Key name").value).toBe("SLACK_TOKEN");
    expect(input("Sites").value).toBe("slack.com");
    const save = vi.spyOn(bridgeApi, "saveApiKey");
    await click("Save key");
    expect(save).toHaveBeenCalledWith({ name: "SLACK_TOKEN", value: "xoxb-not-a-real-token", hosts: ["slack.com"] });
    expect(input("Key value").value).toBe("");
    expect(host.textContent).toContain("Slack token · slack.com");
    await act(async () => root.unmount());
    root = createRoot(host);
    await mount();
    expect(host.textContent).toContain("SLACK_TOKEN");
    await click("Forget SLACK_TOKEN");
    expect(host.textContent).not.toContain("SLACK_TOKEN");
  });

  it("refuses to save without a name, a value and a site", async () => {
    await mount();
    await type("Key value", "some-opaque-value");
    expect(input("Key name").value).toBe("");
    await type("Key name", "internal api");
    expect(input("Key name").value).toBe("INTERNAL_API");
    expect(button("Save key").disabled).toBe(true);
    await type("Sites", "api.internal.example");
    expect(button("Save key").disabled).toBe(false);
  });

  it("edits a saved key's sites without sending a value", async () => {
    await mount();
    const save = vi.spyOn(bridgeApi, "saveApiKey");
    await click("Edit sites for OPENAI_API_KEY");
    await type("Sites for OPENAI_API_KEY", "api.openai.com, *.openai.azure.com");
    await click("Save");
    expect(save).toHaveBeenCalledWith({ name: "OPENAI_API_KEY", hosts: ["api.openai.com", "*.openai.azure.com"] });
    expect(host.textContent).toContain("api.openai.com, *.openai.azure.com");
    await click("Edit sites for OPENAI_API_KEY");
    await type("Sites for OPENAI_API_KEY", "api.openai.com");
    await click("Save");
  });

  it("saves a pasted key for every chat by reference, never by value", async () => {
    await mount();
    const save = vi.spyOn(bridgeApi, "saveApiKey");
    await click("Save this GitHub token for every chat");
    expect(input("Name to save it as").value).toBe("GITHUB_TOKEN");
    expect(input("Sites to allow").value).toBe("api.github.com, uploads.github.com");
    await click("Save");
    expect(save).toHaveBeenCalledWith({ name: "GITHUB_TOKEN", hosts: ["api.github.com", "uploads.github.com"], fromReference: "sec_3f9c2a71d4b84e0f9a1c6b2d7e5f8a90" });
    expect(host.textContent).toContain("Saved as GITHUB_TOKEN");
    await bridgeApi.forgetApiKey("GITHUB_TOKEN");
  });

  it("asks for a key again when this build cannot read it, keeping its sites", async () => {
    const unreadable: ApiKeysSnapshot = { canSave: true, proxyRunning: true, saved: [{ name: "GITHUB_TOKEN", kind: "github", hosts: ["api.github.com"], needsValue: true }], chat: [] };
    vi.spyOn(bridgeApi, "listApiKeys").mockResolvedValue(unreadable);
    const save = vi.spyOn(bridgeApi, "saveApiKey").mockResolvedValue({ ...unreadable, saved: [{ ...unreadable.saved[0], needsValue: false }] });
    await mount();
    expect(host.textContent).toContain("cannot read the saved value");
    expect(button("Edit sites for GITHUB_TOKEN")).toBeUndefined();
    await click("Paste GITHUB_TOKEN again");
    await type("New value for GITHUB_TOKEN", "ghp_newvalue");
    await click("Save");
    expect(save).toHaveBeenCalledWith({ name: "GITHUB_TOKEN", value: "ghp_newvalue", hosts: ["api.github.com"] });
    expect(host.textContent).toContain("GitHub token · api.github.com");
  });

  it("says so when agents cannot reach the key proxy", async () => {
    const off: ApiKeysSnapshot = { canSave: false, proxyRunning: false, saved: [], chat: [] };
    vi.spyOn(bridgeApi, "listApiKeys").mockResolvedValue(off);
    await mount();
    expect(host.textContent).toContain("Agents cannot use keys right now");
    expect(host.textContent).toContain("Saving needs the macOS Keychain");
    expect(input("Key value")).toBeNull();
  });

  it("reports a refused save instead of clearing the form", async () => {
    vi.spyOn(bridgeApi, "saveApiKey").mockRejectedValue(new Error("Keychain refused"));
    await mount();
    await type("Key value", "sk-proj-abcdefghijklmnopqrstuvwxyz");
    expect(host.textContent).toContain("Replaces the saved OPENAI_API_KEY");
    await click("Replace key");
    expect(onError).toHaveBeenCalledWith("Keychain refused");
    expect(input("Key value").value).toBe("sk-proj-abcdefghijklmnopqrstuvwxyz");
  });
});
