import type { AdapterDescriptor, Harness } from "./types";

// The model a side chat (aside) begins on. A side chat is a consult from the
// chat you are already in, so when it targets the same harness you are on it
// carries your model; otherwise it opens on that harness's Standard-tier
// default rather than the bare adapter default (Codex's default is a Fast-tier
// model; OpenCode's is null until a provider catalog loads). The returned id is
// always one the adapter currently exposes, or null only when it exposes none.
export function resolveAsideModel(
  adapter: AdapterDescriptor,
  source?: { harness: Harness; model: string | null } | null,
): string | null {
  const exposes = (id: string | null | undefined): id is string =>
    !!id && adapter.models.some(model => model.id === id);
  if (source && source.harness === adapter.id && exposes(source.model)) return source.model;
  const standard = adapter.models.find(model => model.tier === "standard" && model.defaultForTier)
    ?? adapter.models.find(model => model.tier === "standard");
  if (standard) return standard.id;
  if (exposes(adapter.defaultModel)) return adapter.defaultModel;
  return adapter.models[0]?.id ?? null;
}
