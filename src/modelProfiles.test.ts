import { describe, expect, it } from "vitest";
import { availableModelOptions, recommendedProfileDrafts } from "./modelProfiles";
import type { AdapterDescriptor } from "./types";

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
});
