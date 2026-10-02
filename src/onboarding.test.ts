import { describe, expect, it, vi } from "vitest";
import { AGENT_ONBOARDING_KEY, canInstallManagedAgent, onboardingAgentReady, onboardingChoices, readAgentOnboardingComplete, shouldShowAgentOnboarding, writeAgentOnboardingComplete } from "./onboarding";
import type { AdapterDescriptor, ModelSetupState } from "./types";
import type { ManagedAgentStatus } from "./protocol/generated/protocol";

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

  it("round-trips completed setup without storing provider state", () => {
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

describe("onboarding readiness", () => {
  const agent: ManagedAgentStatus = { agentId: "codex", label: "Codex", backing: "external", state: "external", removable: false, consecutiveFailures: 0, updateAvailable: false };
  const adapter: AdapterDescriptor = { id: "codex", label: "Codex", available: true, authState: "signed_in", capabilities: [], models: [] };
  it("requires installation, availability, and known sign-in independently", () => {
    expect(onboardingAgentReady(agent, adapter)).toBe(true);
    for (const authState of ["signed_out", "unknown"] as const) expect(onboardingAgentReady(agent, { ...adapter, authState })).toBe(false);
    expect(onboardingAgentReady(agent, { ...adapter, available: false })).toBe(false);
    for (const state of ["not_installed", "repairable", "broken", "unavailable"]) expect(onboardingAgentReady({ ...agent, state }, adapter)).toBe(false);
    expect(onboardingAgentReady({ ...agent, backing: "none" }, adapter)).toBe(false);
    expect(onboardingAgentReady(undefined, adapter)).toBe(false);
    expect(onboardingAgentReady(agent, undefined)).toBe(false);
  });
  it("offers managed installation only when the runtime reports a package pin", () => {
    expect(canInstallManagedAgent(agent)).toBe(false);
    expect(canInstallManagedAgent({ ...agent, pinnedVersion: "1.0" })).toBe(true);
  });
  it("includes externally detected adapters without inventing managed install support", () => {
    const choices = onboardingChoices([agent], [adapter, { ...adapter, id: "grok", label: "Grok" }, { ...adapter, id: "bridge" }]);
    expect(choices.map(choice => choice.agentId)).toEqual(["codex", "grok"]);
    expect(choices[1]).toMatchObject({ backing: "external", removable: false });
    expect(canInstallManagedAgent(choices[1])).toBe(false);
  });
});
