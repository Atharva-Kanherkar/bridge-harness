import { useCallback, useEffect, useRef, useState } from "react";
import { bridgeApi } from "./api";
import type { ChatDefaults, ResolvedChatSettings } from "./protocol/generated/protocol";

type Resolution = {
  key: string;
  workspaceId: string | null;
  settings?: ResolvedChatSettings;
  error?: string;
};

/** Responses are bound to both scope and choices, including during retargeting.
 *
 *  The last resolution for the same project stays on screen while a new one is
 *  in flight. Clearing it on every choice made the whole draft blank mid-request
 *  (the model pill fell back to a guess and a worktree toggle flipped back),
 *  which is the flicker a fast local read was hiding rather than avoiding. A
 *  change of project still clears, because those defaults are not this
 *  project's. */
export function useResolvedChatDraft(workspaceId: string | null, overrides: ChatDefaults, enabled: boolean) {
  const [result, setResult] = useState<Resolution>();
  const [refresh, setRefresh] = useState(0);
  const generation = useRef(0);
  const key = JSON.stringify([workspaceId, overrides]);
  useEffect(() => {
    const request = ++generation.current;
    if (!enabled) return;
    void bridgeApi.resolveChatSettings({ workspaceId, overrides }).then(settings => {
      if (generation.current === request) setResult({ key, workspaceId, settings });
    }).catch(error => {
      if (generation.current === request) setResult({ key, workspaceId, error: error instanceof Error ? error.message : String(error) });
    });
    return () => { generation.current++; };
    // The serialized key covers the complete typed request without object-identity churn.
  }, [key, enabled, refresh, workspaceId]);
  // A resolution for another project is not a stale answer, it is a different
  // question: drop it rather than paint one project's defaults on another.
  const current = result?.workspaceId === workspaceId ? result : undefined;
  return {
    settings: current?.settings,
    error: current?.key === key ? current.error : undefined,
    loading: enabled && current?.key !== key,
    reload: useCallback(() => setRefresh(value => value + 1), []),
  };
}
