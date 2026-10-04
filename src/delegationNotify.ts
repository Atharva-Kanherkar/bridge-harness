import { useCallback, useEffect, useState } from "react";
import { bridgeApi } from "./api";
import type { DelegationNotifyLevel, DelegationNotifySettingsView } from "./protocol/generated/protocol";

export const NOTIFY_LEVELS: { value: DelegationNotifyLevel; label: string; description: string }[] = [
  { value: "all", label: "Everything", description: "Every worker notice, including approvals resolving, wakes the orchestrator." },
  { value: "actionable", label: "Actionable", description: "Skip notices that only say keep waiting. Results, failures and pending approvals still arrive." },
  { value: "results-only", label: "Results only", description: "Only results, stops, launch failures and declined scopes. Your approval cards are unchanged." },
];

/** `sessionId` set: that orchestrator's override over the global level. Unset: the global level. */
export function useDelegationNotify(sessionId?: string): {
  view: DelegationNotifySettingsView | null;
  error: string | null;
  /** `null` clears a session override; a global level cannot be cleared. */
  set: (level: DelegationNotifyLevel | null) => void;
} {
  const [view, setView] = useState<DelegationNotifySettingsView | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let active = true;
    bridgeApi
      .delegationNotifySettings(sessionId)
      .then(next => { if (active) setView(next); })
      .catch(cause => { if (active) setError(cause instanceof Error ? cause.message : String(cause)); });
    return () => { active = false; };
  }, [sessionId]);
  const set = useCallback((level: DelegationNotifyLevel | null) => {
    setError(null);
    bridgeApi
      .saveDelegationNotifySettings(sessionId ?? null, level)
      .then(setView)
      .catch(cause => setError(cause instanceof Error ? cause.message : String(cause)));
  }, [sessionId]);
  return { view, error, set };
}
