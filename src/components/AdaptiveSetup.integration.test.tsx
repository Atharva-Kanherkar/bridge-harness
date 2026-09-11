// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import type { AdapterDescriptor, ModelSetupState } from "../types";
import { ModelSetupWizard } from "./ModelSetupWizard";
import { RouterSettingsDialog } from "./RouterSettingsDialog";

const adapters: AdapterDescriptor[] = [{
  id: "catalog", label: "Catalog", available: true, authState: "signed_in", version: "1", capabilities: [], unavailableReason: null, defaultModel: "balanced",
  models: [
    { id: "quick", label: "Quick", tier: "fast", defaultForTier: true },
    { id: "balanced", label: "Balanced", tier: "standard", defaultForTier: true },
    { id: "deep", label: "Deep", tier: "strong", defaultForTier: true },
  ],
}, {
  id: "offline", label: "Offline", available: false, authState: "unknown", version: null, capabilities: [], unavailableReason: "not installed", defaultModel: "hidden",
  models: [{ id: "hidden", label: "Unsupported", tier: "strong", defaultForTier: true }],
}];

function button(label: string): HTMLButtonElement {
  const match = [...document.body.querySelectorAll("button")]
    .find(candidate => candidate.textContent?.includes(label));
  if (!match) throw new Error(`Button ${label} was not rendered`);
  return match;
}

async function flush() {
  await new Promise(resolve => setTimeout(resolve, 0));
}

describe("adaptive setup journeys", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.restoreAllMocks();
  });

  it("detects agents before completing first-run setup with recommended defaults", async () => {
    let completed: ModelSetupState | undefined;
    await act(async () => {
      root.render(<ModelSetupWizard adapters={adapters} onComplete={value => { completed = value; }} onError={error => { throw new Error(error); }} />);
      await flush();
    });
    await act(async () => {
      button("Continue").click();
      await flush();
    });
    await act(async () => {
      button("Use recommended defaults").click();
      await flush();
    });
    expect(completed).toMatchObject({ complete: true, activeVersion: 1 });
    expect(completed?.profiles).toHaveLength(9);
  });

  it("reveals every advanced profile without unavailable models", async () => {
    await act(async () => {
      root.render(<ModelSetupWizard adapters={adapters} onComplete={() => undefined} onError={error => { throw new Error(error); }} />);
      await flush();
    });
    await act(async () => { button("Continue").click(); await flush(); });
    await act(async () => button("Customize role profiles").click());
    expect(document.body.textContent).toContain("Advanced role profiles");
    expect(document.body.textContent).toContain("Model evaluator");
    expect(document.body.textContent).not.toContain("Unsupported");
  });

  it("offers every supported agent and lets a person continue without installing one", async () => {
    const skipped = vi.fn();
    const recommendations = vi.spyOn(bridgeApi, "recommendedModelProfiles");
    await act(async () => {
      root.render(<ModelSetupWizard adapters={[]} onComplete={() => undefined} onSkip={skipped} onError={error => { throw new Error(error); }} />);
      await flush();
    });
    for (const label of ["Claude Code", "Codex", "Cursor", "OpenCode"])
      expect(document.body.textContent).toContain(label);
    expect(document.body.textContent).toContain("Bring your agents with you");
    await act(async () => button("Continue without an agent").click());
    expect(skipped).toHaveBeenCalledTimes(1);
    expect(recommendations).not.toHaveBeenCalled();
  });

  it("marks an existing signed-in Codex as detected without asking for sign-in", async () => {
    const codex = adapters[0];
    await act(async () => {
      root.render(<ModelSetupWizard adapters={[{ ...codex, id: "codex", label: "Codex" }]} onComplete={() => undefined} onError={error => { throw new Error(error); }} />);
      await flush();
    });
    const card = document.body.querySelector('[data-testid="onboarding-agent-codex"]');
    expect(card?.textContent).toContain("Detected");
    expect(card?.textContent).toContain("Signed in");
    expect([...card!.querySelectorAll("button")].some(candidate => candidate.textContent?.includes("Sign in"))).toBe(false);
  });

  it("runs manual learning and renders its explicit no-op report", async () => {
    const errors: string[] = [];
    await act(async () => {
      root.render(<RouterSettingsDialog open workspaceId="demo-1" adapters={adapters.slice(0, 1)} databasePath="/tmp/bridge.db" onClose={() => undefined} onError={error => errors.push(error)} />);
      await flush();
    });
    await act(async () => {
      button("Run learning now").click();
      await flush();
    });
    expect(errors).toEqual([]);
    expect(document.body.textContent).toContain("insufficient evidence");
    expect(document.body.textContent).toContain("Cost comparison is unknown");
    expect(document.body.textContent).toContain("Policy");
    expect(document.body.textContent).toContain("not run");
  });

  it("refetches learning state when a learning job changes", async () => {
    let notify: (() => void) | undefined;
    vi.spyOn(bridgeApi, "onLearningJobChanged").mockImplementation(async handler => {
      notify = handler;
      return () => undefined;
    });
    const learningState = vi.spyOn(bridgeApi, "learningState");
    await act(async () => {
      root.render(<RouterSettingsDialog open workspaceId="demo-1" adapters={adapters.slice(0, 1)} onClose={() => undefined} onError={error => { throw new Error(error); }} />);
      await flush();
    });
    expect(learningState).toHaveBeenCalledTimes(1);
    await act(async () => {
      notify?.();
      await flush();
    });
    expect(learningState).toHaveBeenCalledTimes(2);
  });

  it("a slower learning-state read cannot win", async () => {
    let notify: (() => void) | undefined;
    vi.spyOn(bridgeApi, "onLearningJobChanged").mockImplementation(async handler => {
      notify = handler;
      return () => undefined;
    });
    const initial = await bridgeApi.learningState("demo-1");
    let resolveSlow!: (state: typeof initial) => void;
    let resolveFast!: (state: typeof initial) => void;
    vi.spyOn(bridgeApi, "learningState")
      .mockImplementationOnce(() => Promise.resolve(initial))
      .mockImplementationOnce(() => new Promise(resolve => { resolveSlow = resolve; }))
      .mockImplementationOnce(() => new Promise(resolve => { resolveFast = resolve; }));
    await act(async () => {
      root.render(<RouterSettingsDialog open workspaceId="demo-1" adapters={adapters.slice(0, 1)} onClose={() => undefined} onError={error => { throw new Error(error); }} />);
      await flush();
    });
    await act(async () => {
      notify?.();
      notify?.();
      await flush();
    });
    await act(async () => {
      resolveFast({ ...initial, activePolicyVersion: 7 });
      await flush();
    });
    await act(async () => {
      resolveSlow({ ...initial, activePolicyVersion: 3 });
      await flush();
    });
    expect(document.body.textContent).toContain("Active policy v7");
    expect(document.body.textContent).not.toContain("Active policy v3");
  });

  it("an emptied pass floor cannot commit zero on the way to a number", async () => {
    const update = vi.spyOn(bridgeApi, "updateRouterPreferences");
    await act(async () => {
      root.render(<RouterSettingsDialog open workspaceId="demo-1" adapters={adapters.slice(0, 1)} onClose={() => undefined} onError={error => { throw new Error(error); }} />);
      await flush();
    });
    const field = document.body.querySelector<HTMLInputElement>('input[type="number"][max="100"]')!;
    const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value")!.set!;
    await act(async () => {
      setter.call(field, "");
      field.dispatchEvent(new Event("input", { bubbles: true }));
      field.dispatchEvent(new FocusEvent("focusout", { bubbles: true }));
      await flush();
    });
    expect(field.value, "blurring an empty field restores the saved floor").toBe("65");
    await act(async () => {
      button("Save").click();
      await flush();
    });
    const committed = update.mock.calls.at(-1);
    expect(committed?.[1]?.minimumPassBps).toBe(6500);
  });

  it("typing an out-of-range pass floor shows a validation message and disables Save", async () => {
    await act(async () => {
      root.render(<RouterSettingsDialog open workspaceId="demo-1" adapters={adapters.slice(0, 1)} onClose={() => undefined} onError={error => { throw new Error(error); }} />);
      await flush();
    });
    const field = document.body.querySelector<HTMLInputElement>('input[type="number"][max="100"]')!;
    const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value")!.set!;
    await act(async () => {
      setter.call(field, "150");
      field.dispatchEvent(new Event("input", { bubbles: true }));
      await flush();
    });
    expect(document.body.textContent).toContain("Enter a percentage between 0 and 100.");
    expect(button("Save").disabled).toBe(true);
  });

  it("typing a sub-minimum cadence shows a validation message and disables Save", async () => {
    await act(async () => {
      root.render(<RouterSettingsDialog open workspaceId="demo-1" adapters={adapters.slice(0, 1)} onClose={() => undefined} onError={error => { throw new Error(error); }} />);
      await flush();
    });
    const cadenceLabel = [...document.body.querySelectorAll("label")].find(label => label.textContent?.startsWith("Cadence"))!;
    const field = cadenceLabel.querySelector("input") as HTMLInputElement;
    const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value")!.set!;
    await act(async () => {
      setter.call(field, "5");
      field.dispatchEvent(new Event("input", { bubbles: true }));
      await flush();
    });
    expect(document.body.textContent).toContain("Cadence must be at least 15 minutes.");
    expect(button("Save").disabled).toBe(true);
  });

  it("typing a fractional cadence shows a validation message and disables Save", async () => {
    await act(async () => {
      root.render(<RouterSettingsDialog open workspaceId="demo-1" adapters={adapters.slice(0, 1)} onClose={() => undefined} onError={error => { throw new Error(error); }} />);
      await flush();
    });
    const cadenceLabel = [...document.body.querySelectorAll("label")].find(label => label.textContent?.startsWith("Cadence"))!;
    const field = cadenceLabel.querySelector("input") as HTMLInputElement;
    const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value")!.set!;
    await act(async () => {
      setter.call(field, "15.5");
      field.dispatchEvent(new Event("input", { bubbles: true }));
      await flush();
    });
    expect(document.body.textContent).toContain("Cadence must be a whole number of minutes.");
    expect(button("Save").disabled).toBe(true);
  });

  it("typing a negative spend ceiling shows a validation message and disables Save", async () => {
    await act(async () => {
      root.render(<RouterSettingsDialog open workspaceId="demo-1" adapters={adapters.slice(0, 1)} onClose={() => undefined} onError={error => { throw new Error(error); }} />);
      await flush();
    });
    const ceilingLabel = [...document.body.querySelectorAll("label")].find(label => label.textContent?.startsWith("Spend ceiling"))!;
    const field = ceilingLabel.querySelector("input") as HTMLInputElement;
    const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value")!.set!;
    await act(async () => {
      setter.call(field, "-1");
      field.dispatchEvent(new Event("input", { bubbles: true }));
      await flush();
    });
    expect(document.body.textContent).toContain("Must be zero or greater.");
    expect(button("Save").disabled).toBe(true);
  });

  it("picking a harness clears a previously pinned model", async () => {
    await act(async () => {
      root.render(<RouterSettingsDialog open workspaceId="demo-1" adapters={adapters.slice(0, 1)} onClose={() => undefined} onError={error => { throw new Error(error); }} />);
      await flush();
    });
    const select = (label: string) => [...document.body.querySelectorAll("label")].find(candidate => candidate.textContent?.includes(label))!.querySelector("select") as HTMLSelectElement;
    const harnessSelect = select("Pin harness");
    const modelSelect = select("Pin model");
    const setter = Object.getOwnPropertyDescriptor(window.HTMLSelectElement.prototype, "value")!.set!;
    await act(async () => {
      setter.call(harnessSelect, "catalog");
      harnessSelect.dispatchEvent(new Event("change", { bubbles: true }));
      await flush();
    });
    await act(async () => {
      setter.call(modelSelect, "balanced");
      modelSelect.dispatchEvent(new Event("change", { bubbles: true }));
      await flush();
    });
    expect(modelSelect.value).toBe("balanced");
    await act(async () => {
      setter.call(harnessSelect, "");
      harnessSelect.dispatchEvent(new Event("change", { bubbles: true }));
      await flush();
    });
    expect(modelSelect.value).toBe("");
  });

  it("does not rewrite the learning schedule when save has no schedule edits", async () => {
    const update = vi.spyOn(bridgeApi, "updateLearningSchedule");
    await act(async () => {
      root.render(<RouterSettingsDialog open workspaceId="demo-1" adapters={adapters.slice(0, 1)} onClose={() => undefined} onError={error => { throw new Error(error); }} />);
      await flush();
    });
    await act(async () => {
      button("Save").click();
      await flush();
    });
    expect(update).not.toHaveBeenCalled();
  });

  it("closing the dialog drops unsaved schedule edits", async () => {
    const initial = await bridgeApi.learningState("demo-1");
    vi.spyOn(bridgeApi, "learningState")
      .mockImplementationOnce(() => Promise.resolve(initial))
      .mockImplementation(() => new Promise(() => undefined));
    const render = (open: boolean) => root.render(<RouterSettingsDialog open={open} workspaceId="demo-1" adapters={adapters.slice(0, 1)} onClose={() => undefined} onError={error => { throw new Error(error); }} />);
    await act(async () => { render(true); await flush(); });
    const checkbox = [...document.body.querySelectorAll("label")]
      .find(label => label.textContent?.includes("In-app schedule"))
      ?.querySelector("input[type=checkbox]") as HTMLInputElement;
    await act(async () => { checkbox.click(); await flush(); });
    expect(checkbox.checked).toBe(true);
    await act(async () => { render(false); await flush(); });
    await act(async () => { render(true); await flush(); });
    expect(
      [...document.body.querySelectorAll("label")].some(label => label.textContent?.includes("In-app schedule")),
      "a reopened dialog must show the loading state, not last session's unsaved edits",
    ).toBe(false);
  });

  it("offers editable evaluator spend and token ceilings", async () => {
    await act(async () => {
      root.render(<RouterSettingsDialog open workspaceId="demo-1" adapters={adapters.slice(0, 1)} onClose={() => undefined} onError={error => { throw new Error(error); }} />);
      await flush();
    });
    const disabledCeilings = [...document.body.querySelectorAll("input[type=number]")].filter(input => (input as HTMLInputElement).disabled);
    expect(disabledCeilings).toHaveLength(0);
    expect(document.body.textContent).toContain("Spend ceiling");
    expect(document.body.textContent).toContain("Token ceiling");
    expect(document.body.textContent).not.toContain("No executor yet");
    expect(document.body.textContent).toContain("not Bridge's memory engine");
  });

  it("does not create a profile version when settings save without profile edits", async () => {
    const before = await bridgeApi.modelSetup();
    await act(async () => {
      root.render(<RouterSettingsDialog open workspaceId="demo-1" adapters={adapters.slice(0, 1)} databasePath="/tmp/bridge.db" onClose={() => undefined} onError={error => { throw new Error(error); }} />);
      await flush();
    });
    await act(async () => {
      button("Save").click();
      await flush();
    });
    expect((await bridgeApi.modelSetup()).activeVersion).toBe(before.activeVersion);
  });
});
