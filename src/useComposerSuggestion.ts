import { useCallback, useEffect, useRef, useState } from "react";
import { bridgeApi } from "./api";
import { friendlyError } from "./errors";
import { createSuggestionQueue, scheduleSuggestion } from "./suggestionTypeahead";
import type { SuggestCompletionResult, SuggestionSettingsSnapshot } from "./protocol/generated/protocol";
import { useResetOnChange } from "./useResetOnChange";

export function useComposerSuggestion(text: string, settings: SuggestionSettingsSnapshot | undefined, scope: string | undefined, blocked = false) {
  const [result, setResult] = useState<SuggestCompletionResult>();
  const [error, setError] = useState<string>();
  const [attempt, setAttempt] = useState(0);
  const generation = useRef(0);
  const [queue] = useState(createSuggestionQueue);
  const enabled = !!settings?.settings.enabled && !!scope && !blocked;
  // Depend on stable identity, never the refreshed Session object.
  useEffect(() => scheduleSuggestion({
    text, enabled, request: bridgeApi.suggestCompletion, onResult: setResult,
    generation, queue,
    onError: reason => setError(reason === undefined ? undefined : friendlyError(reason, "Couldn't get an inline suggestion.")),
  }), [text, enabled, scope, settings?.settings.provider, settings?.settings.model, attempt, queue]);
  useResetOnChange([scope, settings?.settings.enabled, settings?.settings.provider, settings?.settings.model], () => setError(undefined));
  const clear = useCallback(() => setResult(undefined), []);
  const retry = useCallback(() => { setError(undefined); setAttempt(value => value + 1); }, []);
  return { result, clear, error, retry };
}
