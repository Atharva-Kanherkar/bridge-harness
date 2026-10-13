"use client";

import { Palette } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { DEFAULT_OMARCHY_THEME, OMARCHY_THEMES } from "../../content/omarchyThemes";
import { useOmarchy } from "../../lib/useOmarchy";
import Swatch from "./Swatch";

/*
 * The header switch into Omarchy mode, and, once on, a theme menu shaped like
 * omarchy.org's palette button. Off by default: the Graphite site is the
 * site; this is the page as an Omarchy user's desktop would paint it.
 */
export default function OmarchyToggle() {
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

  const on = theme !== null;
  return (
    <div ref={menu} className="relative flex items-center gap-1">
      <button
        type="button"
        aria-pressed={on}
        onClick={() => setTheme(on ? null : DEFAULT_OMARCHY_THEME)}
        title={on ? "Back to the Bridge site theme" : "See Bridge the way Omarchy paints it"}
        className={`inline-flex h-7 items-center gap-1.5 rounded-md border px-2 font-mono text-[12px] transition-colors ${
          on ? "border-primary bg-primary text-primary-foreground" : "border-border-card text-muted-foreground hover:text-foreground"
        }`}
      >
        <span aria-hidden="true" className={`size-2 ${on ? "bg-primary-foreground" : "bg-current"}`} />
        Omarchy
      </button>
      {on && (
        <button
          type="button"
          aria-haspopup="menu"
          aria-expanded={open}
          aria-label="Choose an Omarchy theme"
          onClick={() => setOpen(value => !value)}
          className="grid size-7 place-items-center rounded-md border border-border-card text-muted-foreground hover:text-foreground max-sm:hidden"
        >
          <Palette size={14} aria-hidden="true" />
        </button>
      )}
      {on && open && (
        <div role="menu" aria-label="Omarchy themes" className="absolute right-0 top-9 z-50 max-h-[70vh] w-60 overflow-y-auto rounded-md border border-border-card bg-popover p-1 shadow-xl">
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
        </div>
      )}
    </div>
  );
}
