import type { ModelSetupState } from "./types";

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
