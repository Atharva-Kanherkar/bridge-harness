// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import type { AdapterDescriptor, HarnessConfig } from "../../types";
import { HarnessesPage } from "./HarnessesPage";

const harness = { id: "pi", label: "Pi", enabled: true, defaultModel: "local/one", effort: "high", systemPrompt: "", advanced: {}, isOverride: false } as HarnessConfig;
const adapter = { id: "pi", label: "Pi", available: true, authState: "unknown", version: "1.1.0", capabilities: ["messages", "approvals"], sandboxModes: ["danger_full_access"], unavailableReason: null, defaultModel: "local/one", models: [
  { id: "local/one", label: "Local One", tier: "standard", defaultForTier: true, supportedEffortLevels: ["off", "minimal", "low", "high"] },
  { id: "local/two", label: "Local Two", tier: "standard", defaultForTier: false, supportedEffortLevels: ["off"] },
] } as AdapterDescriptor;
async function mount(detailId: string | null, descriptor = adapter) {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const onOpen = vi.fn(); const onRefresh = vi.fn(); const onSave = vi.fn().mockResolvedValue(undefined);
  const container = document.createElement("div"); document.body.append(container); const root = createRoot(container);
  await act(async () => root.render(<HarnessesPage detailId={detailId} onOpenDetail={onOpen} onCloseDetail={() => undefined}
    harnesses={[harness]} adapters={[descriptor]} modelOptions={descriptor.models.map(model => ({ adapter: "pi", id: model.id, label: model.label }))}
    managed={{ agents: [], listError: null, busy: {}, errors: {}, reload: vi.fn(), install: vi.fn(), update: vi.fn(), repair: vi.fn(), requestRemove: vi.fn(), confirmation: null }}
    busy={false} drafts={{}} onDraft={vi.fn()} onDiscard={vi.fn()} onSaveHarness={onSave} onResetHarness={vi.fn()}
    onCatalog={vi.fn()} onError={vi.fn()} onAuthenticationChanged={onRefresh} />));
  return { container, onOpen, onRefresh, onSave, unmount: () => act(async () => { root.unmount(); container.remove(); }) };
}
describe("Pi Harness settings", () => {
  it("shows availability and opens the existing harness detail surface", async () => {
    const view = await mount(null);
    expect(view.container.textContent).toContain("Models and sign-in managed in Pi");
    await act(async () => view.container.querySelector<HTMLButtonElement>('[aria-label="Configure Pi"]')!.click());
    expect(view.onOpen).toHaveBeenCalledWith("pi"); await view.unmount();
  });
  it("shows actionable unavailability and refresh with honest scope", async () => {
    const view = await mount("pi", { ...adapter, available: false, unavailableReason: "Pi is not installed. Install the Pi CLI." });
    expect(view.container.textContent).toContain("Pi is not installed");
    expect(view.container.textContent).toContain("full access"); expect(view.container.textContent).toContain("hidden memory or credential context are unavailable");
    const refresh = [...view.container.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "Refresh Pi")!;
    await act(async () => refresh.click()); expect(view.onRefresh).toHaveBeenCalledOnce();
    expect(view.container.textContent).not.toContain("Sign in status unknown"); await view.unmount();
  });
  it("offers only supported Bridge efforts and clears stale effort on a model change", async () => {
    const view = await mount("pi");
    const effort = view.container.querySelector<HTMLButtonElement>('[aria-label="Pi default effort"]')!;
    await act(async () => effort.click());
    expect([...document.querySelectorAll('[role="option"]')].map(option => option.textContent)).toEqual(["Role default", "low", "high"]);
    await act(async () => effort.click());
    const model = view.container.querySelector<HTMLButtonElement>('[aria-label="Pi default model"]')!;
    await act(async () => model.click());
    const two = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(option => option.textContent?.includes("Local Two"))!;
    await act(async () => { two.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true })); two.click(); });
    expect(view.onSave).toHaveBeenCalledWith(expect.objectContaining({ defaultModel: "local/two", effort: null }));
    await view.unmount();
  });
});
