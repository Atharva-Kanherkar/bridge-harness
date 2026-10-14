"use client";

import { useSyncExternalStore } from "react";
import { OMARCHY_CHANGE_EVENT } from "./omarchySkin";

function subscribe(onChange: () => void) {
  window.addEventListener(OMARCHY_CHANGE_EVENT, onChange);
  return () => window.removeEventListener(OMARCHY_CHANGE_EVENT, onChange);
}

/** The Omarchy theme showing (null in the Graphite skin) and a setter. */
export function useOmarchy(): [string | null, (theme: string | null) => void] {
  const theme = useSyncExternalStore(
    subscribe,
    () => window.__bridgeOmarchy?.current() ?? null,
    () => null,
  );
  return [theme, next => window.__bridgeOmarchy?.apply(next)];
}
