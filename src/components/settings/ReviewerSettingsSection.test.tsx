// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { bridgeApi } from "../../api";
import type { AdapterDescriptor } from "../../types";
import { ReviewerSettingsSection } from "./ReviewerSettingsSection";

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(() => { act(() => root.unmount()); host.remove(); vi.restoreAllMocks(); });

const codex = { id: "codex", label: "Codex", available: true, models: [{ id: "gpt-5-codex", label: "GPT-5 Codex" }] } as unknown as AdapterDescriptor;

it("loads the stored reviewer settings and shows the default prompt as the placeholder", async () => {
  vi.spyOn(bridgeApi, "reviewerSettings").mockResolvedValue({
    settings: { harnesses: { codex: { model: "gpt-5-codex", effort: "xhigh" } }, systemPrompt: "" },
    defaultSystemPrompt: "Review pull request #{number}.",
  });
  await act(async () => { root.render(<ReviewerSettingsSection adapters={[codex]} />); });
  expect(host.querySelector<HTMLButtonElement>('[aria-label="Codex reviewer model"]')?.textContent).toContain("GPT-5 Codex");
  expect(host.querySelector<HTMLButtonElement>('[aria-label="Codex reviewer effort"]')?.textContent).toContain("xhigh");
  const prompt = host.querySelector<HTMLTextAreaElement>('[aria-label="Reviewer instructions"]')!;
  expect(prompt.value).toBe("");
  expect(prompt.placeholder).toBe("Review pull request #{number}.");
  expect(host.querySelector<HTMLButtonElement>('[aria-label="Claude reviewer model"]')?.textContent).toContain("Harness default");
});

it("saves an edited prompt through the save bar and never reports a failed save as saved", async () => {
  vi.spyOn(bridgeApi, "reviewerSettings").mockResolvedValue({ settings: { harnesses: {}, systemPrompt: "" }, defaultSystemPrompt: "default" });
  const save = vi.spyOn(bridgeApi, "saveReviewerSettings")
    .mockRejectedValueOnce(new Error("Could not persist"))
    .mockResolvedValueOnce({ settings: { harnesses: {}, systemPrompt: "Only tests for PR {number}." }, defaultSystemPrompt: "default" });
  await act(async () => { root.render(<ReviewerSettingsSection adapters={[codex]} />); });
  const prompt = host.querySelector<HTMLTextAreaElement>('[aria-label="Reviewer instructions"]')!;
  await act(async () => {
    const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
    setter.call(prompt, "Only tests for PR {number}.");
    prompt.dispatchEvent(new Event("input", { bubbles: true }));
  });
  const submit = async () => act(async () => { [...host.querySelectorAll("button")].find(button => button.textContent === "Save")!.click(); });
  await submit();
  expect(save).toHaveBeenCalledWith({ harnesses: {}, systemPrompt: "Only tests for PR {number}." });
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("Could not persist");
  expect(host.textContent).not.toContain("Saved");
  await submit();
  expect(host.querySelector('[role="status"]')?.textContent).toBe("Saved");
  expect([...host.querySelectorAll("button")].some(button => button.textContent === "Save")).toBe(false);
  // Use default clears the text so the default applies again.
  await act(async () => { [...host.querySelectorAll("button")].find(button => button.textContent === "Use default")!.click(); });
  expect(host.querySelector<HTMLTextAreaElement>('[aria-label="Reviewer instructions"]')!.value).toBe("");
});

it("saves a reviewer model on change without the unsaved prompt", async () => {
  vi.spyOn(bridgeApi, "reviewerSettings").mockResolvedValue({ settings: { harnesses: {}, systemPrompt: "stored" }, defaultSystemPrompt: "default" });
  const save = vi.spyOn(bridgeApi, "saveReviewerSettings").mockImplementation(async settings => ({ settings, defaultSystemPrompt: "default" }));
  await act(async () => { root.render(<ReviewerSettingsSection adapters={[codex]} />); });
  const prompt = host.querySelector<HTMLTextAreaElement>('[aria-label="Reviewer instructions"]')!;
  await act(async () => {
    const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
    setter.call(prompt, "half written");
    prompt.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="Codex reviewer model"]')!.click());
  const option = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(node => node.textContent === "GPT-5 Codex")!;
  await act(async () => { option.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true })); option.click(); });
  expect(save).toHaveBeenCalledWith({ harnesses: { codex: { model: "gpt-5-codex", effort: null } }, systemPrompt: "stored" });
  expect(host.textContent).toContain("Saved");
  expect(host.querySelector<HTMLTextAreaElement>('[aria-label="Reviewer instructions"]')!.value).toBe("half written");
});

it("saves every field of the loaded record, not only the two the form edits", async () => {
  // A field this form does not show (one a newer host adds) must survive a save.
  const loaded = { harnesses: {}, systemPrompt: "stored", futureSetting: "keep me" } as ReviewerSettingsWithExtra;
  vi.spyOn(bridgeApi, "reviewerSettings").mockResolvedValue({ settings: loaded, defaultSystemPrompt: "default" });
  const save = vi.spyOn(bridgeApi, "saveReviewerSettings").mockImplementation(async settings => ({ settings, defaultSystemPrompt: "default" }));
  await act(async () => { root.render(<ReviewerSettingsSection adapters={[codex]} />); });
  const prompt = host.querySelector<HTMLTextAreaElement>('[aria-label="Reviewer instructions"]')!;
  await act(async () => {
    const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
    setter.call(prompt, "Only tests.");
    prompt.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => { [...host.querySelectorAll("button")].find(button => button.textContent === "Save")!.click(); });
  expect(save).toHaveBeenCalledWith({ harnesses: {}, systemPrompt: "Only tests.", futureSetting: "keep me" });
});

type ReviewerSettingsWithExtra = Parameters<typeof bridgeApi.saveReviewerSettings>[0] & { futureSetting: string };
