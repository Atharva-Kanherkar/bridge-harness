import { useCallback, useEffect, useState } from "react";
import { bridgeApi } from "./api";

export function readHideAiAttribution(value: { hideAiAttribution?: boolean | null } | null | undefined): boolean {
  return value?.hideAiAttribution === true;
}

export function useAttributionSettings(): {
  hide: boolean;
  loaded: boolean;
  saving: boolean;
  error: string | null;
  setHide: (next: boolean) => void;
} {
  const [hide, setHideState] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    bridgeApi
      .attributionSettings()
      .then(settings => {
        if (!active) return;
        setHideState(readHideAiAttribution(settings));
        setLoaded(true);
      })
      .catch(cause => {
        if (!active) return;
        setError(cause instanceof Error ? cause.message : String(cause));
        setLoaded(true);
      });
    return () => {
      active = false;
    };
  }, []);

  const setHide = useCallback((next: boolean) => {
    setHideState(next);
    setSaving(true);
    setError(null);
    bridgeApi
      .saveAttributionSettings({ hideAiAttribution: next })
      .then(settings => {
        setHideState(readHideAiAttribution(settings));
      })
      .catch(cause => {
        setError(cause instanceof Error ? cause.message : String(cause));
      })
      .finally(() => setSaving(false));
  }, []);

  return { hide, loaded, saving, error, setHide };
}
