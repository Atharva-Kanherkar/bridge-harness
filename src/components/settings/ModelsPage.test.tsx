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
    expect(view.text()).toContain("Automatic");
    expect(view.text()).toContain("Specific model");
    await view.unmount();
  });

  it("toggles a profile by clicking its name or status anywhere in the row", async () => {
    const view = await mount();
    const row = view.button("Implementer settings")!;
    const name = [...row.querySelectorAll("span")].find(node => node.textContent === "Implementer")!;
    await view.click(name);
    expect(row.getAttribute("aria-expanded")).toBe("true");
    expect(view.text()).toContain("How Bridge chooses a model");

    const status = [...row.querySelectorAll("span")].find(node => node.textContent === "Automatic")!;
    await view.click(status);
    expect(row.getAttribute("aria-expanded")).toBe("false");
    expect(view.text()).not.toContain("How Bridge chooses a model");
    await view.unmount();
  });

  it("reveals the six fields and Allow learning only when a worker row is expanded", async () => {
    const view = await mount();
    expect(view.text()).not.toContain("Allow learning");
    await view.click(view.button("Implementer settings"));
    for (const field of ["How Bridge chooses a model", "Provider and model", "Reasoning effort",
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
    expect(view.text()).not.toContain("How Bridge chooses a model");
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

  it("lets a tracking worker choose a connected model and pins that choice", async () => {
    const view = await mount({ adapters: [adapter(), adapter({
      id: "claude", label: "Claude", defaultModel: "sonnet",
      models: [{ id: "sonnet", label: "Sonnet", tier: "standard", defaultForTier: true, supportedEffortLevels: ["low"] }],
    })] });
    await view.click(view.button("Implementer settings"));
    const picker = view.container.querySelector<HTMLButtonElement>('button[aria-label="Implementer model"]');
    expect(picker?.disabled).toBe(false);
    await view.click(picker);
    const sonnet = [...document.querySelectorAll<HTMLElement>('[role="option"]')]
      .find(option => option.textContent?.includes("Claude · Sonnet"));
    expect(sonnet).toBeDefined();
    await act(async () => {
      sonnet!.dispatchEvent(new Event("pointerdown", { bubbles: true }));
      sonnet!.click();
    });
    const sent = view.onSave.mock.calls[0]?.[0];
    expect(sent?.find(item => item.purpose === "implementer")).toMatchObject({
      provider: "claude", model: "sonnet", effort: "low",
      selectionMode: "pinned", pinned: true, learningEnabled: false,
    });
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
});
