import { describe, expect, it } from "vitest";
import { availableModelOptions, recommendedProfileDrafts, resolveProfileOption } from "./modelProfiles";
import type { AdapterDescriptor, ModelSetupState } from "./types";

const adapters: AdapterDescriptor[] = [{
  id: "catalog", label: "Catalog", available: true, version: "1", capabilities: [], unavailableReason: null, defaultModel: "balanced",
  models: [
    { id: "quick", label: "Quick", tier: "fast", defaultForTier: true },
    { id: "balanced", label: "Balanced", tier: "standard", defaultForTier: true },
    { id: "deep", label: "Deep", tier: "strong", defaultForTier: true },
  ],
}, {
  id: "offline", label: "Offline", available: false, version: null, capabilities: [], unavailableReason: "not installed", defaultModel: "hidden",
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
});
