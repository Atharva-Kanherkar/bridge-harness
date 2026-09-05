import { describe, expect, it } from "vitest";
import { availableModelOptions, modelProfilesChanged, profileDraftsFromSetup, recommendedProfileDrafts, resolveProfileOption, shouldRequireModelSetup } from "./modelProfiles";
import type { AdapterDescriptor, ModelSetupState } from "./types";

const adapters: AdapterDescriptor[] = [{
  id: "catalog", label: "Catalog", available: true, authState: "signed_in", version: "1", capabilities: [], unavailableReason: null, defaultModel: "balanced",
  models: [
    { id: "quick", label: "Quick", tier: "fast", defaultForTier: true },
    { id: "balanced", label: "Balanced", tier: "standard", defaultForTier: true },
    { id: "deep", label: "Deep", tier: "strong", defaultForTier: true },
  ],
}, {
  id: "offline", label: "Offline", available: false, authState: "unknown", version: null, capabilities: [], unavailableReason: "not installed", defaultModel: "hidden",
  models: [{ id: "hidden", label: "Hidden", tier: "strong", defaultForTier: true }],
}];

describe("model profile catalog helpers", () => {
  it("derives every recommendation from defaultForTier metadata", () => {
    const profiles = recommendedProfileDrafts(adapters);
    expect(profiles).toHaveLength(9);
    expect(profiles.every(profile => profile.provider === "catalog")).toBe(true);
    expect(profiles.find(profile => profile.purpose === "documentation")?.model).toBe("quick");
    expect(profiles.find(profile => profile.purpose === "reviewer")?.model).toBe("deep");
  });

  it("excludes unavailable adapters from advanced choices", () => {
    expect(availableModelOptions(adapters).map(option => option.value)).not.toContain("offline:hidden");
  });

  it("keeps availability independent from promotion and excludes incompatible models", () => {
    const catalog = structuredClone(adapters);
    catalog[0].models.push({ id: "selectable", label: "Selectable", tier: "standard", defaultForTier: false, available: true, compatible: true, lifecycle: "stable", source: "runtime_api" });
    catalog[0].models.push({ id: "incompatible", label: "Incompatible", tier: "standard", defaultForTier: false, available: true, compatible: false, lifecycle: "stable", source: "runtime_api" });
    expect(availableModelOptions(catalog).map(option => option.value)).toContain("catalog:selectable");
    expect(availableModelOptions(catalog).map(option => option.value)).not.toContain("catalog:incompatible");
  });

  it("resolves new chats from the persisted Standard profile instead of adapter order", () => {
    const alternate: AdapterDescriptor = {
      ...adapters[0],
      id: "alternate",
      label: "Alternate",
      models: adapters[0].models.map(model => ({ ...model, id: `alternate-${model.id}` })),
    };
    const drafts = recommendedProfileDrafts(adapters);
    const standard = drafts.find(profile => profile.purpose === "standard_orchestrator")!;
    standard.provider = "alternate";
    standard.model = "alternate-balanced";
    const setup: ModelSetupState = {
      complete: true,
      activeVersion: 1,
      profiles: drafts.map(profile => ({
        ...profile,
        schemaVersion: 1,
        version: 1,
        profileId: profile.purpose,
        canonicalRole: "planning" as const,
        createdAt: "now",
      })),
    };
    expect(resolveProfileOption("standard_orchestrator", setup, [...adapters, alternate])?.value)
      .toBe("alternate:alternate-balanced");
    alternate.available = false;
    expect(resolveProfileOption("standard_orchestrator", setup, [...adapters, alternate])?.value)
      .toBe("catalog:balanced");
  });

  it("tracking follows promotion while pinned profiles preserve their model", () => {
    const drafts = recommendedProfileDrafts(adapters);
    const setup: ModelSetupState = {
      complete: true,
      activeVersion: 1,
      profiles: drafts.map(profile => ({ ...profile, schemaVersion: 1, version: 1, profileId: profile.purpose, canonicalRole: "planning", createdAt: "now" })),
    };
    const refreshed = structuredClone(adapters);
    refreshed[0].models.find(model => model.id === "balanced")!.defaultForTier = false;
    refreshed[0].models.push({ id: "balanced-v2", label: "Balanced v2", tier: "standard", defaultForTier: true, available: true, compatible: true, lifecycle: "stable", source: "runtime_api" });
    // A tracking worker (standard tier) follows the newly promoted default.
    expect(resolveProfileOption("implementer", setup, refreshed)?.value).toBe("catalog:balanced-v2");
    // The orchestrator is pinned by default, so it holds its exact model across promotion.
    expect(resolveProfileOption("standard_orchestrator", setup, refreshed)?.value).toBe("catalog:balanced");
  });

  it("does not trap users in setup when no adapter is available", () => {
    const setup: ModelSetupState = { complete: false, activeVersion: null, profiles: [] };
    expect(shouldRequireModelSetup(setup, adapters.map(adapter => ({ ...adapter, available: false })))).toBe(false);
    expect(shouldRequireModelSetup(setup, adapters)).toBe(true);
  });

  it("detects profile edits without churning identical immutable versions", () => {
    const drafts = recommendedProfileDrafts(adapters);
    const setup: ModelSetupState = {
      complete: true,
      activeVersion: 1,
      profiles: drafts.map(profile => ({ ...profile, schemaVersion: 1, version: 1, profileId: profile.purpose, canonicalRole: "planning", createdAt: "now" })),
    };
    expect(profileDraftsFromSetup(setup)).toEqual(drafts);
    expect(modelProfilesChanged(drafts, setup)).toBe(false);
    expect(modelProfilesChanged(drafts.map((profile, index) => index ? profile : { ...profile, effort: "high" }), setup)).toBe(true);
  });
});
