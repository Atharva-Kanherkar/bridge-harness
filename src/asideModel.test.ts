import { describe, expect, it } from "vitest";
import { resolveAsideModel } from "./asideModel";
import type { AdapterDescriptor } from "./types";

function adapter(partial: Partial<AdapterDescriptor> & Pick<AdapterDescriptor, "id" | "models" | "defaultModel">): AdapterDescriptor {
  return {
    label: partial.id, available: true, authState: "signed_in", version: "test",
    capabilities: [], sandboxModes: [], unavailableReason: null,
    ...partial,
  };
}

const codex = adapter({
  id: "codex",
  defaultModel: "gpt-fast",
  models: [
    { id: "gpt-fast", label: "Fast", tier: "fast", defaultForTier: true },
    { id: "gpt-standard", label: "Standard", tier: "standard", defaultForTier: true },
    { id: "gpt-strong", label: "Strong", tier: "strong", defaultForTier: true },
  ],
});

describe("resolveAsideModel", () => {
  it("carries the source model when the harness matches and the model is exposed", () => {
    expect(resolveAsideModel(codex, { harness: "codex", model: "gpt-strong" })).toBe("gpt-strong");
  });

  it("ignores the source model when the harness differs", () => {
    expect(resolveAsideModel(codex, { harness: "claude", model: "opus" })).toBe("gpt-standard");
  });

  it("ignores a source model the adapter does not expose", () => {
    expect(resolveAsideModel(codex, { harness: "codex", model: "retired-model" })).toBe("gpt-standard");
  });

  it("falls back to the Standard-tier default, not the adapter default", () => {
    expect(resolveAsideModel(codex, null)).toBe("gpt-standard");
  });

  it("falls back to defaultModel when there is no Standard-tier model", () => {
    const fastOnly = adapter({
      id: "opencode",
      defaultModel: "only-fast",
      models: [{ id: "only-fast", label: "Only", tier: "fast", defaultForTier: true }],
    });
    expect(resolveAsideModel(fastOnly, null)).toBe("only-fast");
  });

  it("falls back to the first model when defaultModel is not exposed", () => {
    const stale = adapter({
      id: "opencode",
      defaultModel: "gone",
      models: [{ id: "present", label: "Present", tier: "fast", defaultForTier: false }],
    });
    expect(resolveAsideModel(stale, null)).toBe("present");
  });

  it("returns null only when the adapter exposes no models", () => {
    const empty = adapter({ id: "opencode", defaultModel: null, models: [] });
    expect(resolveAsideModel(empty, null)).toBeNull();
  });
});
