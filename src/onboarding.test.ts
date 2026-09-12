import { describe, expect, it, vi } from "vitest";
import { AGENT_ONBOARDING_KEY, readAgentOnboardingComplete, shouldShowAgentOnboarding, writeAgentOnboardingComplete } from "./onboarding";
import type { ModelSetupState } from "./types";

const incomplete: ModelSetupState = { complete: false, activeVersion: null, profiles: [] };

describe("agent onboarding persistence", () => {
  it("shows fresh setup even when no adapter exists, and stays done after model setup", () => {
    expect(shouldShowAgentOnboarding(incomplete, false, false)).toBe(true);
    expect(shouldShowAgentOnboarding(incomplete, true, false)).toBe(false);
    expect(shouldShowAgentOnboarding({ ...incomplete, complete: true }, false, false)).toBe(false);
  });

  it("does not interrupt an existing Bridge workspace during migration", () => {
    expect(shouldShowAgentOnboarding(incomplete, false, true)).toBe(false);
  });

  it("round-trips the optional skip without storing provider state", () => {
    const values = new Map<string, string>();
    const storage = {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: vi.fn((key: string, value: string) => values.set(key, value)),
    };
    expect(readAgentOnboardingComplete(storage)).toBe(false);
    writeAgentOnboardingComplete(storage);
    expect(storage.setItem).toHaveBeenCalledWith(AGENT_ONBOARDING_KEY, "complete");
    expect(readAgentOnboardingComplete(storage)).toBe(true);
  });

  it("fails open for a webview storage error", () => {
    const storage = { getItem: () => { throw new Error("blocked"); } };
    expect(readAgentOnboardingComplete(storage)).toBe(false);
  });
});
