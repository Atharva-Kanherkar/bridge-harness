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

it("saves an edited prompt and never reports a failed save as saved", async () => {
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
  const submit = async () => act(async () => { host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); });
  await submit();
  expect(save).toHaveBeenCalledWith({ harnesses: {}, systemPrompt: "Only tests for PR {number}." });
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("Could not persist");
  expect(host.querySelector('[role="status"]')?.textContent).not.toBe("Saved");
  await submit();
  expect(host.querySelector('[role="status"]')?.textContent).toBe("Saved");
  // Reset clears the text so the default applies again.
  await act(async () => { [...host.querySelectorAll("button")].find(button => button.textContent === "Reset instructions to default")!.click(); });
  expect(host.querySelector<HTMLTextAreaElement>('[aria-label="Reviewer instructions"]')!.value).toBe("");
});
