import type { AdapterDescriptor, CapabilityTier, ModelProfileDraft, ModelSetupState, ProfilePurpose, ReasoningEffort } from "./types";

export const profilePurposes: ProfilePurpose[] = [
  "standard_orchestrator", "premium_orchestrator", "planner", "implementer", "verifier",
  "reviewer", "research", "documentation", "evaluator",
];

// The orchestrator is the model the user talks to, so it is chosen directly
// from the provider's live catalog rather than routed by a capability tier.
// Worker roles keep the fast/standard/strong tiers.
const orchestratorPurposes: ProfilePurpose[] = ["standard_orchestrator", "premium_orchestrator"];

export function isOrchestratorPurpose(purpose: ProfilePurpose): boolean {
  return orchestratorPurposes.includes(purpose);
}

export const profileLabels: Record<ProfilePurpose, string> = {
  standard_orchestrator: "Standard orchestrator",
  premium_orchestrator: "Premium orchestrator",
  planner: "Planner",
  implementer: "Implementer",
  verifier: "Verifier",
  reviewer: "Reviewer",
  research: "Research",
  documentation: "Documentation",
  evaluator: "Model evaluator",
};

const purposeTier: Record<ProfilePurpose, CapabilityTier> = {
  standard_orchestrator: "standard",
  premium_orchestrator: "strong",
  planner: "strong",
  implementer: "standard",
  verifier: "standard",
  reviewer: "strong",
  research: "standard",
  documentation: "fast",
  evaluator: "strong",
};

const purposeEffort: Record<ProfilePurpose, ReasoningEffort> = {
  standard_orchestrator: "medium",
  premium_orchestrator: "high",
  planner: "high",
  implementer: "medium",
  verifier: "medium",
  reviewer: "high",
  research: "medium",
  documentation: "low",
  evaluator: "high",
};

const purposeFallback: Record<ProfilePurpose, ProfilePurpose | null> = {
  standard_orchestrator: null,
  premium_orchestrator: "standard_orchestrator",
  planner: "standard_orchestrator",
  implementer: "standard_orchestrator",
  verifier: null,
  reviewer: "verifier",
  research: "standard_orchestrator",
  documentation: "standard_orchestrator",
  evaluator: "verifier",
};

export function availableModelOptions(adapters: AdapterDescriptor[]) {
  return adapters
    .filter(adapter => adapter.available)
    .flatMap(adapter => adapter.models
      .filter(model => model.available !== false && model.compatible !== false)
      .map(model => ({ adapter, model, value: `${adapter.id}:${model.id}` })));
}

export function recommendedProfileDrafts(adapters: AdapterDescriptor[]): ModelProfileDraft[] {
  const options = availableModelOptions(adapters);
  return profilePurposes.map(purpose => {
    const tier = purposeTier[purpose];
    const selected = options.find(option => option.model.tier === tier && option.model.defaultForTier)
      ?? options.find(option => option.model.tier === tier);
    if (!selected) throw new Error(`No available ${tier} model for ${profileLabels[purpose]}`);
    const orchestrator = isOrchestratorPurpose(purpose);
    return {
      purpose,
      provider: selected.adapter.id,
      model: selected.model.id,
      effort: purposeEffort[purpose],
      fallbackPurpose: purposeFallback[purpose],
      // The orchestrator is a direct, pinned user choice; workers track their tier.
      selectionMode: orchestrator ? "pinned" : "track_standard",
      pinned: orchestrator,
      learningEnabled: !orchestrator,
      budgetPreference: null,
      latencyPreference: null,
    };
  });
}

export function resolveProfileOption(
  purpose: ProfilePurpose,
  setup: ModelSetupState,
  adapters: AdapterDescriptor[],
) {
  const options = availableModelOptions(adapters);
  const profiles = new Map(setup.profiles.map(profile => [profile.purpose, profile]));
  const seen = new Set<ProfilePurpose>();
  let current: ProfilePurpose | null = purpose;
  while (current && !seen.has(current)) {
    seen.add(current);
    const profile = profiles.get(current);
    if (!profile) break;
    const selectionMode = profile.selectionMode ?? (profile.pinned ? "pinned" : "track_standard");
    if (selectionMode === "track_standard") {
      const tier = purposeTier[current];
      const promoted = options.find(option => option.adapter.id === profile.provider && option.model.tier === tier && option.model.defaultForTier)
        ?? options.find(option => option.model.tier === tier && option.model.defaultForTier);
      if (promoted) return promoted;
      current = profile.fallbackPurpose ?? null;
      continue;
    }
    const selected = options.find(option => option.adapter.id === profile.provider && option.model.id === profile.model);
    if (selected) return selected;
    current = profile.fallbackPurpose ?? null;
  }
  const tier = purposeTier[purpose];
  return options.find(option => option.model.tier === tier && option.model.defaultForTier)
    ?? options.find(option => option.model.tier === tier);
}

export function profileDraftsFromSetup(setup: ModelSetupState): ModelProfileDraft[] {
  return setup.profiles.map(({ purpose, provider, model, effort, fallbackPurpose, selectionMode, pinned, learningEnabled, budgetPreference, latencyPreference }) => ({
    purpose, provider, model, effort, fallbackPurpose,
    selectionMode: selectionMode ?? (pinned ? "pinned" : "track_standard"),
    pinned, learningEnabled, budgetPreference, latencyPreference,
  }));
}

export function modelProfilesChanged(profiles: ModelProfileDraft[], setup?: ModelSetupState): boolean {
  return !setup || JSON.stringify(profiles) !== JSON.stringify(profileDraftsFromSetup(setup));
}

export function shouldRequireModelSetup(setup: ModelSetupState, adapters: AdapterDescriptor[]): boolean {
  return !setup.complete && adapters.some(adapter => adapter.available);
}
