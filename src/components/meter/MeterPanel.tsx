// The menu-bar meter panel: the root of the `meter` window.
//
// This runs in its own webview, not inside the app, so it cannot read App's
// state and owns its data instead — the provider registry, the account-usage
// stream, and a refresh of its own. That separation is the point: the meter is
// a menu-bar surface, and reaching it must never involve the main window.
//
// The window is deliberately never focused (see `src-tauri/src/meter_tray.rs`:
// focusing it would activate Bridge and drag the main window forward), so this
// panel must not rely on keyboard focus for anything. Closing is a click.
import { useCallback, useEffect, useState } from "react";
import { bridgeApi } from "../../api";
import { extractUsageSnapshot, type UsageProvider, type UsageSnapshot } from "../../usage";
import type { MeterRegistry } from "../../types";
import { MeterPopover } from "./MeterPopover";
import { followThemeAcrossWindows, useThemePreference } from "../../theme";

/** How often the panel re-reads limits while it is on screen. */
const VISIBLE_REFRESH_MS = 60_000;

export function MeterPanel() {
  // `index.html` resolves the theme before first paint in every document, so
  // the panel opens in the right appearance already. What it cannot do is
  // follow a later change: this keeps the panel with the system appearance,
  // and `followThemeAcrossWindows` carries a Settings change over from the
  // main window, which `THEME_EVENT` alone never leaves its own document to do.
  useThemePreference();
  useEffect(followThemeAcrossWindows, []);
  const [usage, setUsage] = useState<Partial<Record<UsageProvider, UsageSnapshot>>>({});
  const [registry, setRegistry] = useState<MeterRegistry | null>(null);
  const [refreshing, setRefreshing] = useState(false);

  const refresh = useCallback(() => {
    setRefreshing(true);
    void bridgeApi.refreshMeter()
      .catch(() => undefined)
      // The reply rides the usage stream rather than resolving this call, so
      // the spinner is a short acknowledgement, not a completion signal.
      .finally(() => window.setTimeout(() => setRefreshing(false), 900));
  }, []);

  useEffect(() => {
    let active = true;
    void bridgeApi.getMeterSnapshot().then(value => { if (active) setRegistry(value); }).catch(() => undefined);
    let off: (() => void) | undefined;
    void bridgeApi.onAccountUsage(payload => {
      const snapshot = extractUsageSnapshot({ rateLimits: payload.rateLimits });
      setUsage(current => {
        // A frame carrying nothing readable is the provider saying it has no
        // current limits — most often because its last window reset with no
        // session running. Ignoring it would leave the percentage that
        // provider reported before it went quiet on screen indefinitely.
        if (!snapshot) {
          if (!(payload.provider in current)) return current;
          const next = { ...current };
          delete next[payload.provider as UsageProvider];
          return next;
        }
        return { ...current, [payload.provider]: snapshot };
      });
    }).then(fn => { if (!active) { fn(); return; } off = fn; });
    return () => { active = false; off?.(); };
  }, []);

  // Ask once on mount, then only while the panel is actually on screen —
  // a hidden menu-bar panel polling limits forever is exactly the waste the
  // adaptive refresh policy exists to avoid.
  useEffect(() => {
    refresh();
    const tick = window.setInterval(() => {
      if (document.visibilityState === "visible") refresh();
    }, VISIBLE_REFRESH_MS);
    return () => window.clearInterval(tick);
  }, [refresh]);

  // The tray asks for a refresh when it opens the panel, so the numbers are
  // current by the time the user reads them.
  useEffect(() => {
    let off: (() => void) | undefined;
    let active = true;
    void bridgeApi.onMeterTray(action => {
      if (active && action === "refresh") refresh();
    }).then(fn => { if (!active) { fn(); return; } off = fn; });
    return () => { active = false; off?.(); };
  }, [refresh]);

  return <MeterPopover
    usage={usage}
    registry={registry}
    refreshing={refreshing}
    onRefresh={refresh}
    onClose={() => { void bridgeApi.hideMeterPanel(); }}
    onOpenBridge={() => {
      // Dismiss first: the app coming forward while a floating panel stays
      // over it is the pile-up this whole change exists to avoid.
      void bridgeApi.hideMeterPanel();
      void bridgeApi.revealMainWindow();
    }}
  />;
}
