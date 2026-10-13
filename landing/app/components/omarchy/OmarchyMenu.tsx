"use client";

import { ChevronDown } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { OMARCHY_THEMES } from "../../content/omarchyThemes";
import { useOmarchy } from "../../lib/useOmarchy";
import Swatch from "./Swatch";

/*
 * The header control while Omarchy mode is on: one button naming the mode,
 * opening the theme list and the way back. In the Graphite site it renders
 * nothing; the promo card (OmarchyPromo) is how people find the mode.
 */
export default function OmarchyMenu() {
  const [theme, setTheme] = useOmarchy();
  const [open, setOpen] = useState(false);
  const menu = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onPointer = (event: PointerEvent) => {
      if (!menu.current?.contains(event.target as Node)) setOpen(false);
    };
    const onKey = (event: KeyboardEvent) => event.key === "Escape" && setOpen(false);
    document.addEventListener("pointerdown", onPointer);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", onPointer);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  if (theme === null) return null;
  const current = OMARCHY_THEMES.find(option => option.name === theme);
  return (
    <div ref={menu} className="relative">
      <button
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={`Omarchy theme: ${current?.label ?? theme}`}
        onClick={() => setOpen(value => !value)}
        className="inline-flex h-7 items-center gap-1.5 rounded-md border border-primary bg-primary px-2 font-mono text-[12px] text-primary-foreground"
      >
        <span aria-hidden="true" className="size-2 bg-primary-foreground" />
        <span className="max-sm:hidden">Omarchy</span>
        <ChevronDown size={12} aria-hidden="true" />
      </button>
      {open && (
        <div role="menu" aria-label="Omarchy themes" className="absolute right-0 top-9 z-50 max-h-[70vh] w-60 overflow-y-auto rounded-md border border-border-card bg-popover p-1 shadow-xl max-sm:fixed max-sm:inset-x-4 max-sm:top-16 max-sm:w-auto">
          {OMARCHY_THEMES.map(option => (
            <button
              key={option.name}
              type="button"
              role="menuitemradio"
              aria-checked={option.name === theme}
              onClick={() => {
                setTheme(option.name);
                setOpen(false);
              }}
              className={`flex w-full items-center gap-2.5 rounded-sm px-2 py-1.5 text-left font-mono text-[12px] ${
                option.name === theme ? "bg-selection text-foreground" : "text-muted-foreground hover:bg-accent hover:text-foreground"
              }`}
            >
              <Swatch theme={option} />
              <span className="min-w-0 flex-1 truncate">{option.label}</span>
              {option.mode === "light" && <span className="text-[10px] text-faint">light</span>}
            </button>
          ))}
          <div className="my-1 border-t border-border" />
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              setTheme(null);
              setOpen(false);
            }}
            className="w-full rounded-sm px-2 py-1.5 text-left font-mono text-[12px] text-muted-foreground hover:bg-accent hover:text-foreground"
          >
            Back to the Bridge theme
          </button>
        </div>
      )}
    </div>
  );
}
