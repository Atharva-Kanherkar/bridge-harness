import { useCallback, useEffect, useRef, useState } from "react";
import { bridgeApi } from "./api";
import type { ChatDefaults, ResolvedChatSettings } from "./protocol/generated/protocol";

/** Responses are bound to both scope and choices, including during retargeting. */
export function useResolvedChatDraft(workspaceId: string | null, overrides: ChatDefaults, enabled: boolean) {
  const [result, setResult] = useState<{ key: string; settings?: ResolvedChatSettings; error?: string }>();
  const [refresh, setRefresh] = useState(0);
  const generation = useRef(0);
  const key = JSON.stringify([workspaceId, overrides]);
  useEffect(() => {
    const request = ++generation.current;
    if (!enabled) return;
    setResult(undefined);
    void bridgeApi.resolveChatSettings({ workspaceId, overrides }).then(settings => {
      if (generation.current === request) setResult({ key, settings });
    }).catch(error => {
      if (generation.current === request) setResult({ key, error: error instanceof Error ? error.message : String(error) });
    });
    return () => { generation.current++; };
    // The serialized key covers the complete typed request without object-identity churn.
  }, [key, enabled, refresh]);
  return { settings: result?.key === key ? result.settings : undefined, error: result?.key === key ? result.error : undefined,
    loading: enabled && (result?.key !== key || (!result.settings && !result.error)),
    reload: useCallback(() => setRefresh(value => value + 1), []) };
}
