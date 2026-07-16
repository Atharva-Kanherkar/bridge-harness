import type { AdapterDescriptor, CapabilityTier, ModelProfileDraft, ProfilePurpose, ReasoningEffort } from "./types";

export const profilePurposes: ProfilePurpose[] = [
  "standard_orchestrator", "premium_orchestrator", "planner", "implementer", "verifier",
  "reviewer", "research", "documentation", "evaluator",
];

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
    .flatMap(adapter => adapter.models.map(model => ({ adapter, model, value: `${adapter.id}:${model.id}` })));
}

export function recommendedProfileDrafts(adapters: AdapterDescriptor[]): ModelProfileDraft[] {
  const options = availableModelOptions(adapters);
  return profilePurposes.map(purpose => {
    const tier = purposeTier[purpose];
    const selected = options.find(option => option.model.tier === tier && option.model.defaultForTier)
      ?? options.find(option => option.model.tier === tier);
    if (!selected) throw new Error(`No available ${tier} model for ${profileLabels[purpose]}`);
    return {
      purpose,
      provider: selected.adapter.id,
      model: selected.model.id,
      effort: purposeEffort[purpose],
      fallbackPurpose: purposeFallback[purpose],
      pinned: false,
      learningEnabled: true,
      budgetPreference: null,
      latencyPreference: null,
    };
  });
}
