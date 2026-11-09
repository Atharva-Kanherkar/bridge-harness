import { useEffect, useRef, useState } from "react";
import { bridgeApi } from "./api";

/** Leases last 30 s in core; renewing at a third of that hands a crashed
 * owner's banners over within 40 s. */
export const PRESENCE_RENEW_MS = 10_000;

/** One random id per app load. */
export const CLIENT_ID = typeof crypto !== "undefined" && "randomUUID" in crypto
  ? crypto.randomUUID()
  : `client-${Math.random().toString(36).slice(2)}`;

/**
 * Whether this client posts OS banners. Core names one owner across every
 * client of the daemon, so one event raises one banner. Reported on mount, on
 * every focus change and on an interval; a closing window reports `gone` so
 * the next client takes over at once. Until the first answer, and after a
 * failed report (an older daemon), it owns: nothing goes silent.
 */
export function useBannerOwner(focused: boolean, clientId = CLIENT_ID, renewMs = PRESENCE_RENEW_MS): boolean {
  const [owner, setOwner] = useState(true);
  const focusedRef = useRef(focused);
  focusedRef.current = focused;
  // One request counter for renewals and focus changes alike, so a late
  // answer never overwrites a newer one.
  const reportRef = useRef<() => void>(() => undefined);

  useEffect(() => {
    let active = true;
    let latest = 0;
    const report = () => {
      const ticket = ++latest;
      bridgeApi.reportPresence(clientId, focusedRef.current).then(
        result => { if (active && ticket === latest) setOwner(result.bannerOwner); },
        () => { if (active && ticket === latest) setOwner(true); },
      );
    };
    reportRef.current = report;
    report();
    const timer = window.setInterval(report, renewMs);
    const leave = () => { void bridgeApi.reportPresence(clientId, false, true).catch(() => undefined); };
    window.addEventListener("pagehide", leave);
    return () => {
      active = false;
      reportRef.current = () => undefined;
      window.clearInterval(timer);
      window.removeEventListener("pagehide", leave);
    };
  }, [clientId, renewMs]);

  const mounted = useRef(false);
  useEffect(() => {
    if (!mounted.current) { mounted.current = true; return; }
    reportRef.current();
  }, [focused]);

  return owner;
}
