"use client";

import { OMARCHY_THEMES } from "../../content/omarchyThemes";
import { useOmarchy } from "../../lib/useOmarchy";
import Swatch from "./Swatch";

/*
 * Every theme Omarchy ships, one square each, under the hero. Clicking one
 * retints the page and the demo below it, which is the feature itself: Bridge
 * on Omarchy takes the palette of whatever theme you switch to.
 */
export default function OmarchyThemeStrip() {
  const [theme, setTheme] = useOmarchy();
  const current = OMARCHY_THEMES.find(option => option.name === theme);
  return (
    <div className="mt-7 hidden flex-col items-center gap-3 omarchy:flex">
      <div role="radiogroup" aria-label="Omarchy theme" className="flex max-w-[620px] flex-wrap justify-center gap-1.5">
        {OMARCHY_THEMES.map(option => (
          <button
            key={option.name}
            type="button"
            role="radio"
            aria-checked={option.name === theme}
            aria-label={option.label}
            title={option.label}
            onClick={() => setTheme(option.name)}
            className={`outline-offset-2 transition-transform hover:-translate-y-0.5 ${option.name === theme ? "outline-2 outline-primary" : ""}`}
          >
            <Swatch theme={option} size="md" />
          </button>
        ))}
      </div>
      <p className="font-mono text-[12px] text-muted-foreground">
        <span className="text-foreground">{current?.label ?? "Tokyo Night"}</span> · pick a theme and the app below retints, like it does on your desktop
      </p>
    </div>
  );
}
