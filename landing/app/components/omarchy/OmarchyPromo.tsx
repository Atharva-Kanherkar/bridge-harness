"use client";

import { X } from "lucide-react";
import { useEffect, useState } from "react";
import { DEFAULT_OMARCHY_THEME } from "../../content/omarchyThemes";
import { useOmarchy } from "../../lib/useOmarchy";

/** Set once the card has been closed or used, so it never nags. */
export const PROMO_STORAGE_KEY = "bridge-site-omarchy-promo";

/*
 * A small card, once per browser, telling visitors the Omarchy edition exists
 * and letting them see the page in it. Closing it or trying the mode retires
 * it; the footer link stays as the way back in.
 */
export default function OmarchyPromo() {
  const [theme, setTheme] = useOmarchy();
  const [visible, setVisible] = useState(false);

  useEffect(() => {
    let seen = false;
    try {
      seen = localStorage.getItem(PROMO_STORAGE_KEY) !== null;
    } catch {
      seen = true;
    }
    if (seen) return;
    const timer = window.setTimeout(() => setVisible(true), 1200);
    return () => window.clearTimeout(timer);
  }, []);

  const retire = () => {
    setVisible(false);
    try {
      localStorage.setItem(PROMO_STORAGE_KEY, "seen");
    } catch {
      // Without storage the card simply comes back next visit.
    }
  };

  if (!visible || theme !== null) return null;
  return (
    <aside
      aria-label="Omarchy edition"
      className="fixed bottom-5 right-5 z-50 w-80 rounded-xl border border-border-card bg-popover p-4 shadow-2xl motion-safe:animate-fade-up max-sm:inset-x-4 max-sm:bottom-4 max-sm:w-auto"
    >
      <button
        type="button"
        aria-label="Dismiss"
        onClick={retire}
        className="absolute right-2.5 top-2.5 grid size-6 place-items-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground"
      >
        <X size={14} aria-hidden="true" />
      </button>
      <p className="flex items-center gap-2 font-mono text-[12px] text-foreground">
        <span aria-hidden="true" className="size-2 bg-foreground" />
        On Omarchy?
      </p>
      <p className="mt-2 pr-4 text-[13px] leading-5 text-muted-foreground">
        Bridge has an Omarchy edition that takes its colors and font from your theme. See this page the way it would look on your desktop.
      </p>
      <div className="mt-3 flex items-center gap-2">
        <button
          type="button"
          onClick={() => {
            retire();
            setTheme(DEFAULT_OMARCHY_THEME);
          }}
          className="rounded-md bg-foreground px-3 py-1.5 font-mono text-[12px] text-background hover:bg-foreground/90"
        >
          Try Omarchy mode
        </button>
        <button type="button" onClick={retire} className="rounded-md px-2 py-1.5 text-[12px] text-muted-foreground hover:text-foreground">
          Not now
        </button>
      </div>
    </aside>
  );
}
