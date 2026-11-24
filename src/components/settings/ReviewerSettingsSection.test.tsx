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

// --- review job settings (testing/feat-review-pane.md) ------------------------

type JobSettings = Awaited<ReturnType<typeof bridgeApi.getReviewJobSettings>>;
const JOB_DEFAULTS: JobSettings = { defaultHarnesses: [], fallbackHarnesses: [], maxConcurrentReviews: 2, maxReviewWorktrees: 8, postMode: "draft", secondOpinion: false };

function mockJobSettings(initial: JobSettings = JOB_DEFAULTS) {
  vi.spyOn(bridgeApi, "reviewerSettings").mockResolvedValue({ settings: { harnesses: {}, systemPrompt: "" }, defaultSystemPrompt: "default" });
  let stored = { ...initial };
  vi.spyOn(bridgeApi, "getReviewJobSettings").mockImplementation(async () => ({ ...stored }));
  return vi.spyOn(bridgeApi, "saveReviewJobSettings").mockImplementation(async next => { stored = { ...next }; return { ...stored }; });
}

const pick = async (label: string, text: string) => {
  await act(async () => host.querySelector<HTMLButtonElement>(`[aria-label="${label}"]`)!.click());
  // An earlier select's list can still be mounted: take the newest match.
  const option = [...document.querySelectorAll<HTMLElement>('[role="option"]')].filter(node => node.textContent === text).at(-1)!;
  await act(async () => { option.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true })); option.click(); });
};
const pressButton = async (text: string) => act(async () => { [...host.querySelectorAll("button")].find(button => button.textContent === text)!.click(); });

it("round-trips every review job setting", async () => {
  const save = mockJobSettings();
  await act(async () => { root.render(<ReviewerSettingsSection adapters={[codex]} />); });
  expect(host.textContent).toContain("Findings wait in the pane");

  await pressButton("Post automatically");
  expect(save).toHaveBeenLastCalledWith(expect.objectContaining({ postMode: "auto" }));
  expect(host.textContent).toContain("Bridge posts every finding as one comment review");

  await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="Second opinion"]')!.click());
  expect(save).toHaveBeenLastCalledWith(expect.objectContaining({ secondOpinion: true }));
  expect(host.querySelector('[aria-label="Second opinion"]')!.getAttribute("aria-checked")).toBe("true");

  await pick("Default reviewer", "Claude");
  expect(save).toHaveBeenLastCalledWith(expect.objectContaining({ defaultHarnesses: ["claude"] }));
  await pick("Second reviewer", "Codex");
  expect(save).toHaveBeenLastCalledWith(expect.objectContaining({ defaultHarnesses: ["claude", "codex"] }));

  await pick("First fallback", "OpenCode");
  await pick("Then", "Codex");
  expect(save).toHaveBeenLastCalledWith(expect.objectContaining({ fallbackHarnesses: ["opencode", "codex"] }));

  const concurrency = host.querySelector<HTMLInputElement>('[aria-label="Reviews at once"]')!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(concurrency, "3");
    concurrency.dispatchEvent(new Event("input", { bubbles: true }));
    concurrency.dispatchEvent(new FocusEvent("blur", { bubbles: true }));
    concurrency.dispatchEvent(new FocusEvent("focusout", { bubbles: true }));
  });
  expect(save).toHaveBeenLastCalledWith(expect.objectContaining({ maxConcurrentReviews: 3 }));
});

it("round-trips auto-review, draft skipping and incremental re-review, each saying what it does", async () => {
  const save = mockJobSettings();
  await act(async () => { root.render(<ReviewerSettingsSection adapters={[codex]} />); });
  expect(host.textContent).toContain("Reviews start only when you ask from the GitHub pane.");
  const skip = () => host.querySelector<HTMLButtonElement>('[aria-label="Skip draft PRs"]')!;
  expect(skip().disabled, "nothing to skip while auto-review is off").toBe(true);
  expect(skip().getAttribute("aria-checked")).toBe("true");

  await pressButton("My PRs");
  expect(save).toHaveBeenLastCalledWith(expect.objectContaining({ autoReview: "mine" }));
  expect(host.textContent).toContain("Pull requests you opened are reviewed");
  await pressButton("All PRs");
  expect(save).toHaveBeenLastCalledWith(expect.objectContaining({ autoReview: "all" }));
  expect(skip().disabled).toBe(false);
  await act(async () => skip().click());
  expect(save).toHaveBeenLastCalledWith(expect.objectContaining({ skipDraftPrs: false }));
  expect(host.textContent).toContain("Drafts are reviewed like any other pull request.");

  expect(host.textContent).toContain("Every review looks at the whole pull request.");
  await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="Incremental re-review"]')!.click());
  expect(save).toHaveBeenLastCalledWith(expect.objectContaining({ incremental: true }));
  expect(host.textContent).toContain("remembers what was kept, dropped or resolved");

  await pressButton("Off");
  expect(save).toHaveBeenLastCalledWith(expect.objectContaining({ autoReview: "off", skipDraftPrs: false, incremental: true }));
});

it("turning Second opinion off keeps only the first default reviewer", async () => {
  const save = mockJobSettings({ ...JOB_DEFAULTS, secondOpinion: true, defaultHarnesses: ["codex", "claude"] });
  await act(async () => { root.render(<ReviewerSettingsSection adapters={[codex]} />); });
  expect(host.querySelector('[aria-label="Second reviewer"]')).not.toBeNull();
  await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="Second opinion"]')!.click());
  expect(save).toHaveBeenLastCalledWith(expect.objectContaining({ secondOpinion: false, defaultHarnesses: ["codex"] }));
  expect(host.querySelector('[aria-label="Second reviewer"]')).toBeNull();
});

it("a refused job settings save shows the reason and keeps the stored value", async () => {
  const save = mockJobSettings();
  save.mockRejectedValueOnce(new Error("Reviews at once must be between 1 and 4"));
  await act(async () => { root.render(<ReviewerSettingsSection adapters={[codex]} />); });
  await pressButton("Post automatically");
  expect(host.querySelector('[role="alert"]')!.textContent).toContain("between 1 and 4");
  expect(host.querySelector('[aria-label="When a review finishes"] [aria-pressed="true"]')!.textContent).toBe("Draft first");
});
