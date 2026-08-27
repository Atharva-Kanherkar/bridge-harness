import { readFileSync } from "node:fs";
import { join } from "node:path";
import { beforeEach, describe, expect, it } from "vitest";
import { useUiStore } from "./uiStore";

const APP = readFileSync(join(__dirname, "App.tsx"), "utf8");

describe("UI store modals", () => {
  beforeEach(() => useUiStore.setState({ modal: null }));

  it("opens and replaces the active modal", () => {
    useUiStore.getState().openModal("workspace");
    expect(useUiStore.getState().modal).toBe("workspace");

    useUiStore.getState().openModal("memory");
    expect(useUiStore.getState().modal).toBe("memory");
  });

  it("closes the active modal", () => {
    useUiStore.getState().openModal("router");
    useUiStore.getState().closeModal();

    expect(useUiStore.getState().modal).toBeNull();
  });

  it("replaces App's local modal state", () => {
    expect(APP).toContain("useUiStore(state => state.modal)");
    expect(APP).toContain("useUiStore(state => state.openModal)");
    expect(APP).toContain("useUiStore(state => state.closeModal)");
    expect(APP).not.toMatch(/useState<[^>]*workspace[^>]*orchestrator/);
    expect(APP).not.toContain("setModal");
  });
});
