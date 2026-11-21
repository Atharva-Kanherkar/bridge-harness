import { useCallback, useEffect, useState } from "react";
import { bridgeApi } from "./api";
import type { EarlyCompactionSettings } from "./types";

/** The text of both controls. The chat item shows only while the global switch is on, so its note names no default. */
export const EARLY_COMPACTION_DESCRIPTION = "When a Claude or Codex chat ends a turn and stays idle, Bridge asks the agent to make its context smaller. Off by default.";
export const EARLY_COMPACTION_CHAT_NOTE = "Makes the context smaller at an idle turn end. Clear the tick to keep this chat out.";

/** `sessionId` set: the switch as that chat sees it. Unset: the global switch only. */
export function useEarlyCompaction(sessionId?: string): {
  view: EarlyCompactionSettings | null;
  error: string | null;
  busy: boolean;
  /** No `sessionId`: sets the global switch. With one: `false` opts the chat out and `null` removes the opt-out. */
  set: (enabled: boolean | null) => void;
} {
  const [view, setView] = useState<EarlyCompactionSettings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    let active = true;
    bridgeApi
      .earlyCompactionSettings(sessionId)
      .then(next => { if (active) setView(next); })
      .catch(cause => { if (active) setError(cause instanceof Error ? cause.message : String(cause)); });
    return () => { active = false; };
  }, [sessionId]);
  const set = useCallback((enabled: boolean | null) => {
    setError(null);
    setBusy(true);
    bridgeApi
      .saveEarlyCompactionSettings(sessionId ?? null, enabled)
      .then(setView)
      .catch(cause => setError(cause instanceof Error ? cause.message : String(cause)))
      .finally(() => setBusy(false));
  }, [sessionId]);
  return { view, error, busy, set };
}
