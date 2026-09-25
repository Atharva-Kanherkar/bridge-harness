// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import type { AdapterDescriptor, ModelProfileDraft, ProfilePurpose } from "../../types";
import { ModelsPage } from "./ModelsPage";

function adapter(overrides: Partial<AdapterDescriptor> = {}): AdapterDescriptor {
  return {
    id: "codex", label: "Codex", available: true, authState: "signed_in", version: "test",
    capabilities: ["messages"], sandboxModes: ["workspace_write", "read_only"],
    unavailableReason: null, defaultModel: "gpt-5",
    models: [
      { id: "gpt-5", label: "GPT-5", tier: "standard", defaultForTier: true, supportedEffortLevels: ["low", "high"] },
      { id: "gpt-5-mini", label: "GPT-5 mini", tier: "fast", defaultForTier: true, supportedEffortLevels: ["low"] },
    ],
    ...overrides,
  } as AdapterDescriptor;
}

function profile(purpose: ProfilePurpose, overrides: Partial<ModelProfileDraft> = {}): ModelProfileDraft {
  return {
    purpose, provider: "codex", model: "gpt-5", effort: "high",
    pinned: false, selectionMode: "track_standard", learningEnabled: true,
    fallbackPurpose: null, budgetPreference: null, latencyPreference: null,
    ...overrides,
  } as ModelProfileDraft;
}

function secondAdapter(): AdapterDescriptor {
  return {
    id: "claude", label: "Claude", available: true, authState: "signed_in", version: "test",
    capabilities: ["messages"], sandboxModes: ["workspace_write", "read_only"],
    unavailableReason: null, defaultModel: "opus-5-5",
    models: [
      { id: "opus-5-5", label: "Opus 5.5", tier: "strong", defaultForTier: true, supportedEffortLevels: ["low", "medium", "high"] },
    ],
  } as AdapterDescriptor;
}

/** Drive the kit's Base UI select the way settingsKit.test.tsx does: open the
 *  trigger, then pick an option out of the portaled listbox. */
async function chooseOption(
  view: { button: (label: string) => HTMLButtonElement | null; click: (node: Element | null) => Promise<void> },
  triggerLabel: string,
  optionText: string,
) {
  await view.click(view.button(triggerLabel));
  const listbox = [...document.querySelectorAll('[role="listbox"]')].at(-1)!;
  const option = [...listbox.querySelectorAll<HTMLElement>('[role="option"]')]
    .find(item => item.textContent?.includes(optionText));
  expect(option, `option "${optionText}" must be offered`).toBeTruthy();
  await act(async () => {
    option!.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
    option!.click();
  });
}

const profiles = [
  profile("standard_orchestrator", { pinned: true, selectionMode: "pinned", learningEnabled: false }),
  profile("implementer"),
  profile("reviewer", { pinned: true, selectionMode: "pinned", learningEnabled: false }),
];

async function mount(props: Partial<Parameters<typeof ModelsPage>[0]> = {}) {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const onSave = vi.fn<(next: ModelProfileDraft[]) => Promise<void>>().mockResolvedValue(undefined);
  const onRefreshCatalogs = vi.fn<() => Promise<void>>().mockResolvedValue(undefined);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<ModelsPage
    profiles={profiles}
    adapters={[adapter()]}
    version={7}
    busy={false}
    onSave={onSave}
    onRefreshCatalogs={onRefreshCatalogs}
    onError={() => undefined}
    {...props}
  />));
  return {
    container, onSave, onRefreshCatalogs,
    text: () => container.textContent ?? "",
    button: (label: string) => [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find(node => (node.getAttribute("aria-label") ?? node.textContent ?? "").includes(label)) ?? null,
    click: async (node: Element | null) => {
      expect(node, "control must exist to be clicked").not.toBeNull();
      await act(async () => { (node as HTMLButtonElement).click(); });
    },
    unmount: () => act(async () => { root.unmount(); container.remove(); }),
  };
}

describe("ModelsPage", () => {
  it("groups profiles by what they are for", async () => {
    const view = await mount();
    expect(view.text()).toContain("Orchestration");
    expect(view.text()).toContain("Workers");
    expect(view.text()).toContain("Verification");
    expect(view.text()).toContain("Catalog");
    await view.unmount();
  });

  // A row has to answer "what is this set to" without being opened, which the
  // old six-field grid never did.
  it("states each profile's provider, model, and effort on its row", async () => {
    const view = await mount();
    expect(view.text()).toContain("Codex · GPT-5 · high");
    await view.unmount();
  });

  it("marks a worker as pinned or tracking, and says nothing of the sort for the orchestrator", async () => {
    const view = await mount();
    expect(view.text()).toContain("Tracks standard");
    expect(view.text()).toContain("Pinned");
    await view.unmount();
  });

  it("reveals the six fields and Allow learning only when a worker row is expanded", async () => {
    const view = await mount();
    expect(view.text()).not.toContain("Allow learning");
    await view.click(view.button("Implementer settings"));
    for (const field of ["Selection behavior", "Provider and model", "Reasoning effort",
                         "Fallback profile", "Budget preference", "Latency preference", "Allow learning"]) {
      expect(view.text(), field).toContain(field);
    }
    await view.unmount();
  });

  // The orchestrator is chosen directly from the catalog, so it has no tier
  // behavior to configure.
  it("gives the orchestrator a model and a thinking level, and no tier controls", async () => {
    const view = await mount();
    await view.click(view.button("Standard orchestrator settings"));
    expect(view.text()).toContain("Thinking");
    expect(view.text()).not.toContain("Selection behavior");
    await view.unmount();
  });

  it("persists on change, sending the whole profile set with one row altered", async () => {
    const view = await mount();
    await view.click(view.button("Implementer settings"));
    await view.click(view.button("Implementer allow learning"));
    expect(view.onSave).toHaveBeenCalledOnce();
    const sent = view.onSave.mock.calls[0][0];
    expect(sent).toHaveLength(3);
    expect(sent.find(item => item.purpose === "implementer")!.learningEnabled).toBe(false);
    expect(sent.find(item => item.purpose === "reviewer")).toEqual(profiles[2]);
    await view.unmount();
  });

  it("carries no Save button, because there is nothing on this page to hold back", async () => {
    const view = await mount();
    expect([...view.container.querySelectorAll("button")].map(node => node.textContent?.trim()))
      .not.toContain("Save");
    expect(view.text()).toContain("Version 7");
    await view.unmount();
  });

  it("flags a stale catalog and offers to retry it", async () => {
    const view = await mount({
      adapters: [adapter({ modelCatalog: { stale: true, source: "last_known_good", lastError: null } } as Partial<AdapterDescriptor>)],
    });
    expect(view.text()).toContain("Stale");
    expect(view.text()).toContain("Using last-known-good models");
    await view.click(view.button("Retry"));
    expect(view.onRefreshCatalogs).toHaveBeenCalledOnce();
    await view.unmount();
  });

  it("uses no native select and no native checkbox", async () => {
    const view = await mount();
    await view.click(view.button("Implementer settings"));
    expect(view.container.querySelectorAll("select")).toHaveLength(0);
    expect(view.container.querySelectorAll('input[type="checkbox"]')).toHaveLength(0);
    await view.unmount();
  });

  // Issue #708: the picker was greyed out for every role that ships tracking,
  // so the only way to change a worker model was to first find "Selection
  // behavior" and pin the role by hand.
  it("lets a tracking worker's model be chosen, and pins the role on the spot", async () => {
    const view = await mount({ adapters: [adapter(), secondAdapter()] });
    await view.click(view.button("Implementer settings"));
    expect(view.button("Implementer model")!.disabled).toBe(false);
    await chooseOption(view, "Implementer model", "Claude · Opus 5.5");
    expect(view.onSave).toHaveBeenCalledOnce();
    const sent = view.onSave.mock.calls[0][0];
    expect(sent).toHaveLength(3);
    expect(sent.find(item => item.purpose === "implementer")).toEqual({
      ...profiles[1], provider: "claude", model: "opus-5-5",
      selectionMode: "pinned", pinned: true, learningEnabled: false,
    });
    expect(sent.find(item => item.purpose === "reviewer")).toEqual(profiles[2]);
    await view.unmount();
  });

  it("does not pin a role when the model already shown is chosen again", async () => {
    const view = await mount();
    await view.click(view.button("Implementer settings"));
    await chooseOption(view, "Implementer model", "Codex · GPT-5");
    expect(view.onSave).not.toHaveBeenCalled();
    await view.unmount();
  });

  it("keeps a pinned role's model directly changeable", async () => {
    const view = await mount();
    await view.click(view.button("Reviewer settings"));
    expect(view.button("Reviewer model")!.disabled).toBe(false);
    await chooseOption(view, "Reviewer model", "Codex · GPT-5 mini");
    expect(view.onSave.mock.calls[0][0].find(item => item.purpose === "reviewer"))
      .toEqual({ ...profiles[2], model: "gpt-5-mini" });
    await view.unmount();
  });

  it("sends a pinned role back to tracking, and gates Allow learning on the mode", async () => {
    const view = await mount();
    await view.click(view.button("Reviewer settings"));
    expect(view.button("Reviewer allow learning")!.disabled).toBe(true);
    await chooseOption(view, "Reviewer selection behavior", "Track standard");
    expect(view.onSave).toHaveBeenCalledOnce();
    const sent = view.onSave.mock.calls[0][0].find(item => item.purpose === "reviewer")!;
    expect(sent.selectionMode).toBe("track_standard");
    expect(sent.pinned).toBe(false);
    // The implementer has tracked all along, so its learning switch is live:
    // the gate is the mode, not the role.
    await view.click(view.button("Reviewer settings"));
    await view.click(view.button("Implementer settings"));
    expect(view.button("Implementer allow learning")!.disabled).toBe(false);
    await view.unmount();
  });

  it("says what a tracked role does, and that choosing a model pins it", async () => {
    const view = await mount();
    await view.click(view.button("Implementer settings"));
    expect(view.text()).toContain("Follows the standard model for this role's tier");
    expect(view.text()).toContain("Choose a model here to pin it.");
    await view.click(view.button("Implementer settings"));
    await view.click(view.button("Reviewer settings"));
    expect(view.text()).toContain("This role always uses this model");
    await view.unmount();
  });
});
