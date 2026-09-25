// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { AdapterDescriptor, ModelProfileDraft, ProfilePurpose } from "../types";
import { profileLabels } from "../modelProfiles";
import { ModelProfileEditor } from "./ModelProfileEditor";

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => { act(() => root.unmount()); host.remove(); vi.restoreAllMocks(); });

const adapters = [
  {
    id: "codex", label: "Codex", available: true,
    models: [
      { id: "gpt-5", label: "GPT-5", tier: "standard", defaultForTier: true, supportedEffortLevels: ["low", "high"] },
      { id: "gpt-5-mini", label: "GPT-5 mini", tier: "fast", defaultForTier: true, supportedEffortLevels: ["low"] },
    ],
  },
  {
    id: "claude", label: "Claude", available: true,
    models: [
      { id: "opus-5-5", label: "Opus 5.5", tier: "strong", defaultForTier: true, supportedEffortLevels: ["low", "medium", "high"] },
    ],
  },
] as unknown as AdapterDescriptor[];

function profile(purpose: ProfilePurpose, overrides: Partial<ModelProfileDraft> = {}): ModelProfileDraft {
  return {
    purpose, provider: "codex", model: "gpt-5", effort: "medium",
    pinned: false, selectionMode: "track_standard", learningEnabled: true,
    fallbackPurpose: null, budgetPreference: null, latencyPreference: null,
    ...overrides,
  } as ModelProfileDraft;
}

const profiles = [
  profile("standard_orchestrator", { pinned: true, selectionMode: "pinned", learningEnabled: false }),
  profile("implementer"),
  profile("reviewer", { pinned: true, selectionMode: "pinned", learningEnabled: false }),
];

/** The card holding one role's fields, so a query for "Provider & model" cannot
 *  answer with the implementer's copy when the reviewer is the subject. */
function roleCard(purpose: ProfilePurpose): HTMLElement {
  const name = profileLabels[purpose];
  const card = [...host.querySelectorAll("section")].find(section => {
    const heading = section.querySelector("h3")?.textContent ?? section.querySelector("h4")?.textContent;
    return heading === name && section.querySelector("select");
  });
  expect(card, `role "${name}" must render its own fields`).toBeTruthy();
  return card!;
}

/** A labeled native field inside one role's card, matched on the label's own
 *  text so "Model" cannot answer for "Provider & model". */
function field(purpose: ProfilePurpose, label: string): HTMLSelectElement {
  const owner = [...roleCard(purpose).querySelectorAll("label")]
    .find(candidate => candidate.firstChild?.textContent?.trim() === label);
  expect(owner, `field "${label}" must exist for ${profileLabels[purpose]}`).toBeTruthy();
  return owner!.querySelector("select") as HTMLSelectElement;
}

async function choose(purpose: ProfilePurpose, label: string, value: string) {
  const select = field(purpose, label);
  const setter = Object.getOwnPropertyDescriptor(window.HTMLSelectElement.prototype, "value")!.set!;
  await act(async () => {
    setter.call(select, value);
    select.dispatchEvent(new Event("change", { bubbles: true }));
  });
}

async function mount(current: ModelProfileDraft[] = profiles) {
  const onChange = vi.fn<(next: ModelProfileDraft[]) => void>();
  await act(async () => { root.render(<ModelProfileEditor profiles={current} adapters={adapters} onChange={onChange} />); });
  return onChange;
}

it("lets a tracking worker role's model be chosen, and pins the role on the spot", async () => {
  const onChange = await mount();
  expect(field("implementer", "Provider & model").disabled).toBe(false);
  await choose("implementer", "Provider & model", "claude:opus-5-5");
  expect(onChange).toHaveBeenCalledOnce();
  const sent = onChange.mock.calls[0][0];
  expect(sent).toHaveLength(3);
  expect(sent.find(item => item.purpose === "implementer")).toEqual({
    ...profiles[1], provider: "claude", model: "opus-5-5",
    selectionMode: "pinned", pinned: true, learningEnabled: false,
  });
  expect(sent.find(item => item.purpose === "reviewer")).toEqual(profiles[2]);
});

it("keeps a pinned role's model directly changeable", async () => {
  const onChange = await mount();
  await choose("reviewer", "Provider & model", "codex:gpt-5-mini");
  expect(onChange.mock.calls[0][0].find(item => item.purpose === "reviewer"))
    .toEqual({ ...profiles[2], model: "gpt-5-mini" });
});

it("sends a pinned role back to tracking, and gates Allow learning on the mode", async () => {
  const onChange = await mount();
  const learning = (purpose: ProfilePurpose) => roleCard(purpose).querySelector<HTMLInputElement>('input[type="checkbox"]')!;
  expect(learning("implementer").disabled).toBe(false);
  expect(learning("reviewer").disabled).toBe(true);
  await choose("reviewer", "Selection behavior", "track_standard");
  const sent = onChange.mock.calls[0][0].find(item => item.purpose === "reviewer")!;
  expect(sent.selectionMode).toBe("track_standard");
  expect(sent.pinned).toBe(false);
});

it("keeps the orchestrator's direct model choice pinning, and normalizing effort", async () => {
  const onChange = await mount();
  await choose("standard_orchestrator", "Model", "codex:gpt-5-mini");
  expect(onChange.mock.calls[0][0].find(item => item.purpose === "standard_orchestrator")).toEqual({
    ...profiles[0], model: "gpt-5-mini", effort: "low",
  });
});
