// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import type { AdapterDescriptor, ModelSetupState } from "../types";
import { ModelSetupWizard } from "./ModelSetupWizard";
import { RouterSettingsDialog } from "./RouterSettingsDialog";

const adapters: AdapterDescriptor[] = [{
  id: "catalog", label: "Catalog", available: true, version: "1", capabilities: [], unavailableReason: null, defaultModel: "balanced",
  models: [
    { id: "quick", label: "Quick", tier: "fast", defaultForTier: true },
    { id: "balanced", label: "Balanced", tier: "standard", defaultForTier: true },
    { id: "deep", label: "Deep", tier: "strong", defaultForTier: true },
  ],
}, {
  id: "offline", label: "Offline", available: false, version: null, capabilities: [], unavailableReason: "not installed", defaultModel: "hidden",
  models: [{ id: "hidden", label: "Unsupported", tier: "strong", defaultForTier: true }],
}];

function button(container: HTMLElement, label: string): HTMLButtonElement {
  const match = [...container.querySelectorAll("button")]
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

  it("completes first-run setup with one recommended-default action", async () => {
    let completed: ModelSetupState | undefined;
    await act(async () => {
      root.render(<ModelSetupWizard adapters={adapters} onComplete={value => { completed = value; }} onError={error => { throw new Error(error); }} />);
      await flush();
    });
    await act(async () => {
      button(container, "Use recommended defaults").click();
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
    await act(async () => button(container, "Customize role profiles").click());
    expect(container.textContent).toContain("Advanced role profiles");
    expect(container.textContent).toContain("Model evaluator");
    expect(container.textContent).not.toContain("Unsupported");
  });

  it("runs manual learning and renders its explicit no-op report", async () => {
    const errors: string[] = [];
    await act(async () => {
      root.render(<RouterSettingsDialog open workspaceId="demo-1" adapters={adapters.slice(0, 1)} databasePath="/tmp/bridge.db" onClose={() => undefined} onError={error => errors.push(error)} />);
      await flush();
    });
    await act(async () => {
      button(container, "Run learning now").click();
      await flush();
    });
    expect(errors).toEqual([]);
    expect(container.textContent).toContain("insufficient evidence");
    expect(container.textContent).toContain("Cost comparison is unknown");
    expect(container.textContent).toContain("Policy");
  });

  it("does not create a profile version when settings save without profile edits", async () => {
    const before = await bridgeApi.modelSetup();
    await act(async () => {
      root.render(<RouterSettingsDialog open workspaceId="demo-1" adapters={adapters.slice(0, 1)} databasePath="/tmp/bridge.db" onClose={() => undefined} onError={error => { throw new Error(error); }} />);
      await flush();
    });
    await act(async () => {
      button(container, "Save").click();
      await flush();
    });
    expect((await bridgeApi.modelSetup()).activeVersion).toBe(before.activeVersion);
  });
});
