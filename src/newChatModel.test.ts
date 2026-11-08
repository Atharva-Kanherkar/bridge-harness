import { describe, expect, it } from "vitest";
import { resolveDraftHarnessModel } from "./newChatModel";
import type { AdapterDescriptor, ModelSetupState } from "./types";

function adapter(partial: Partial<AdapterDescriptor> & Pick<AdapterDescriptor, "id" | "models">): AdapterDescriptor {
  return {
    label: partial.id,
    available: true,
    authState: "signed_in",
    version: "test",
    capabilities: [],
    unavailableReason: null,
    defaultModel: partial.models[0]?.id ?? null,
    ...partial,
  } as AdapterDescriptor;
}

const opencode = adapter({
  id: "opencode",
  models: [{ id: "opencode-go/kimi", label: "Kimi K2.5", tier: "standard", defaultForTier: true }],
  defaultModel: "opencode-go/kimi",
});

const claude = adapter({
  id: "claude",
  models: [{ id: "sonnet", label: "Claude Sonnet", tier: "standard", defaultForTier: true }],
  defaultModel: "sonnet",
});

const setup: ModelSetupState = {
  complete: true,
  activeVersion: 1,
  profiles: [{
    schemaVersion: 1, version: 1, profileId: "p1", canonicalRole: "planning", createdAt: "now",
    purpose: "standard_orchestrator", provider: "claude", model: "sonnet", effort: "medium",
    fallbackPurpose: null, selectionMode: "pinned", pinned: true, learningEnabled: false,
    budgetPreference: null, latencyPreference: null,
  }],
};

describe("resolveDraftHarnessModel", () => {
  it("TestResolveDraftHarnessModel_PrefersExplicitOverride", () => {
    expect(resolveDraftHarnessModel({
      adapters: [claude, opencode],
      setup,
      session: { kind: "direct", harness: "claude", model: "sonnet" },
      harnessOverride: opencode,
      stored: { harness: "claude", model: "sonnet" },
    })).toEqual({ harness: "opencode", model: "opencode-go/kimi" });
  });

  it("TestResolveDraftHarnessModel_CarriesCurrentDirectSession", () => {
    expect(resolveDraftHarnessModel({
      adapters: [claude, opencode],
      setup,
      session: { kind: "direct", harness: "opencode", model: "opencode-go/kimi" },
      stored: { harness: "claude", model: "sonnet" },
    })).toEqual({ harness: "opencode", model: "opencode-go/kimi" });
  });

  it("TestResolveDraftHarnessModel_UsesStoredChoiceOverProfile", () => {
    expect(resolveDraftHarnessModel({
      adapters: [claude, opencode],
      setup,
      stored: { harness: "opencode", model: "opencode-go/kimi", label: "Kimi K2.5" },
    })).toEqual({ harness: "opencode", model: "opencode-go/kimi" });
  });

  it("TestResolveDraftHarnessModel_IgnoresStoredChoiceForUnavailableAdapter", () => {
    const signedOut = adapter({ id: "opencode", available: false, models: [] });
    expect(resolveDraftHarnessModel({
      adapters: [claude, signedOut],
      setup,
      stored: { harness: "opencode", model: "opencode-go/kimi" },
    })).toEqual({ harness: "claude", model: "sonnet" });
  });

  it("TestResolveDraftHarnessModel_KeepsStoredModelWhileCatalogEmpty", () => {
    // Discovery has not landed: the adapter exists but its catalog is empty.
    const discovering = adapter({ id: "opencode", models: [] });
    expect(resolveDraftHarnessModel({
      adapters: [claude, discovering],
      setup,
      stored: { harness: "opencode", model: "opencode-go/kimi" },
    })).toEqual({ harness: "opencode", model: "opencode-go/kimi" });
  });

  it("TestResolveDraftHarnessModel_DropsRetiredStoredModel", () => {
    // The catalog is in and no longer lists the remembered id.
    expect(resolveDraftHarnessModel({
      adapters: [claude, opencode],
      setup,
      stored: { harness: "opencode", model: "opencode-go/retired" },
    })).toEqual({ harness: "claude", model: "sonnet" });
  });

  it("TestResolveDraftHarnessModel_FallsBackToProfileThenAdapter", () => {
    // No stored choice and no profile: the first available adapter's default.
    const empty: ModelSetupState = { complete: false, activeVersion: null, profiles: [] };
    expect(resolveDraftHarnessModel({ adapters: [claude, opencode], setup: empty }))
      .toEqual({ harness: "claude", model: "sonnet" });
    expect(resolveDraftHarnessModel({ adapters: [], setup: null }))
      .toEqual({ harness: "codex", model: null });
  });
});
