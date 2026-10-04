import type { AdapterDescriptor, ModelSetupState } from "./types";
import type { ManagedAgentStatus } from "./protocol/generated/protocol";

/** Installation and authentication are separate facts. Unknown never means ready. */
export function onboardingAgentReady(agent: ManagedAgentStatus | undefined, adapter: AdapterDescriptor | undefined): boolean {
  return !!agent && agent.backing !== "none"
    && !["not_installed", "broken", "repairable", "unavailable"].includes(agent.state)
    && adapter?.available === true && adapter.authState === "signed_in";
}

export function canInstallManagedAgent(agent: ManagedAgentStatus): boolean {
  // The native runtime reports a pin only when it has an installation recipe
  // for this agent on this platform. Do not invent support for another CLI.
  return !!agent.pinnedVersion;
}

export type OnboardingAgent = ManagedAgentStatus & { installationSourceUnknown?: boolean };

export function onboardingChoices(agents: ManagedAgentStatus[], adapters: AdapterDescriptor[], installationListUnavailable = false): OnboardingAgent[] {
  return [...agents, ...adapters.filter(adapter => adapter.id !== "bridge" && !agents.some(agent => agent.agentId === adapter.id)).map(adapter => ({
    agentId: adapter.id, label: adapter.label,
    state: adapter.available ? "external" : "unavailable",
    backing: adapter.available || adapter.version ? "external" as const : "none" as const,
    removable: false, updateAvailable: false, consecutiveFailures: 0,
    version: adapter.version,
    installationSourceUnknown: installationListUnavailable,
  }))];
}

export const AGENT_ONBOARDING_KEY = "bridge.agent-onboarding.v1";

function browserStorage(): Storage | undefined {
  try {
    return globalThis.localStorage;
  } catch {
    return undefined;
  }
}

export function readAgentOnboardingComplete(storage?: Pick<Storage, "getItem">): boolean {
  try {
    return (storage ?? browserStorage())?.getItem(AGENT_ONBOARDING_KEY) === "complete";
  } catch {
    return false;
  }
}

export function writeAgentOnboardingComplete(storage?: Pick<Storage, "setItem">): void {
  try {
    (storage ?? browserStorage())?.setItem(AGENT_ONBOARDING_KEY, "complete");
  } catch {
    // An unavailable webview store must not trap someone in onboarding for the
    // rest of the current process; App also holds completion in React state.
  }
}

export function shouldShowAgentOnboarding(setup: ModelSetupState, completedLocally: boolean, hasExistingBridgeData: boolean): boolean {
  return !setup.complete && !completedLocally && !hasExistingBridgeData;
}
