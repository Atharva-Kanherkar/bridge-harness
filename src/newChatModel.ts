import { resolveProfileOption } from "./modelProfiles";
import type { LastDraftModel } from "./lastDraftModel";
import type { AdapterDescriptor, Harness, ModelSetupState } from "./types";

export type DraftChoice = { harness: Harness; model: string | null };

/** The slice of a session the draft carry-over reads. Loosely typed so the
 *  resolver stays pure and testable without a whole `Session`. */
export type DraftSession = {
  kind?: string | null;
  harness: string;
  model?: string | null;
};

type ResolveArgs = {
  adapters: readonly AdapterDescriptor[];
  setup?: ModelSetupState | null;
  /** The session being left, when it is a direct chat whose choices carry over. */
  session?: DraftSession | null;
  /** A `$harness` shortcut's adapter. It wins over everything, including the
   *  current session, because the user named it explicitly. */
  harnessOverride?: AdapterDescriptor | null;
  /** The last confirmed new-chat choice, read from storage. */
  stored?: LastDraftModel | null;
};

/** A stored choice only applies while its adapter is installed, and its model
 *  only while the provider still offers it. An empty catalog means discovery has
 *  not landed yet: keep the remembered id rather than guessing a different one. */
function fromStoredChoice(stored: LastDraftModel | null | undefined, adapters: readonly AdapterDescriptor[]): DraftChoice | null {
  if (!stored) return null;
  const adapter = adapters.find(item => item.id === stored.harness && item.available);
  if (!adapter) return null;
  if (stored.model == null) return { harness: adapter.id as Harness, model: null };
  if (adapter.models.length > 0) {
    const offered = adapter.models.some(model =>
      model.id === stored.model && model.available !== false && model.compatible !== false);
    if (!offered) return null;
  }
  return { harness: adapter.id as Harness, model: stored.model };
}

/**
 * What a new chat starts on. Order: an explicit `$harness` override, the direct
 * chat being left, the last confirmed choice, the standard orchestrator profile,
 * then the first available adapter's default.
 *
 * The stored choice sits above the profile on purpose: the profile can only be
 * resolved once provider catalogs are in, so waiting for it is what made a new
 * chat paint a fallback model for the first two seconds and then snap.
 */
export function resolveDraftHarnessModel({ adapters, setup, session, harnessOverride, stored }: ResolveArgs): DraftChoice {
  if (harnessOverride) {
    return {
      harness: harnessOverride.id as Harness,
      model: harnessOverride.defaultModel ?? harnessOverride.models[0]?.id ?? null,
    };
  }
  const currentAdapter = session?.kind === "direct"
    ? adapters.find(adapter => adapter.id === session.harness && adapter.available)
    : undefined;
  if (currentAdapter) {
    return {
      harness: currentAdapter.id as Harness,
      model: session?.model ?? currentAdapter.defaultModel ?? currentAdapter.models[0]?.id ?? null,
    };
  }
  const storedChoice = fromStoredChoice(stored, adapters);
  if (storedChoice) return storedChoice;
  const profile = setup ? resolveProfileOption("standard_orchestrator", setup, [...adapters]) : undefined;
  const preferred = profile?.adapter ?? adapters.find(adapter => adapter.available) ?? adapters[0];
  return {
    harness: (preferred?.id as Harness) ?? "codex",
    model: profile?.model.id ?? preferred?.defaultModel ?? preferred?.models[0]?.id ?? null,
  };
}
