import { useCallback, useEffect, useRef, useState } from "react";
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
  // The last server-confirmed value. A rejected save rolls the switch back
  // to this instead of leaving an unpersisted value on screen.
  const confirmedRef = useRef(false);

  useEffect(() => {
    let active = true;
    bridgeApi
      .attributionSettings()
      .then(settings => {
        if (!active) return;
        const value = readHideAiAttribution(settings);
        setHideState(value);
        confirmedRef.current = value;
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
        const value = readHideAiAttribution(settings);
        setHideState(value);
        confirmedRef.current = value;
      })
      .catch(cause => {
        // Roll back to the last confirmed value so the switch never shows
        // an unpersisted state as if the backend had accepted it.
        setHideState(confirmedRef.current);
        setError(cause instanceof Error ? cause.message : String(cause));
      })
      .finally(() => setSaving(false));
  }, []);

  return { hide, loaded, saving, error, setHide };
}
