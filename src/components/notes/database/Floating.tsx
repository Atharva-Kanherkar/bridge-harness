// A panel floating over the page at its anchor, for menus and editors that
// open from a table cell: in the body, so a scrolling table never clips it.

import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
// clsx rather than cn: tailwind-merge does not know the text-ui and
// text-caption sizes, and drops them beside a text color.
import { clsx } from "clsx";

const NESTED_LAYER = '[data-slot="select-positioner"],[data-slot="popover-positioner"],[data-slot="menu-positioner"]';
const OPEN_NESTED_LAYER = '[data-slot="select-popup"][data-open],[data-slot="popover-popup"][data-open],[data-slot="menu-popup"][data-open]';

/** A panel that floats over the page at its anchor, so a cell in a scrolling
 *  table is never clipped. Closes on a click outside or Escape. */
export function Floating({ anchor, label, onClose, width = "w-72", children }: { anchor: React.RefObject<HTMLElement>; label: string; onClose: () => void; width?: string; children: ReactNode }) {
  const panel = useRef<HTMLDivElement>(null);
  const [place, setPlace] = useState<{ left: number; top?: number; bottom?: number }>({ left: 0, top: 0 });
  useLayoutEffect(() => {
    const rect = anchor.current?.getBoundingClientRect();
    if (!rect) return;
    const below = rect.bottom + 320 < window.innerHeight || rect.top < 320;
    // Measuring the anchor and placing the panel before paint is what a
    // layout effect is for.
    // oxlint-disable-next-line react/set-state-in-effect -- placement comes from the anchor's measured box
    setPlace({ left: Math.max(8, Math.min(rect.left, window.innerWidth - 296)), ...(below ? { top: rect.bottom + 4 } : { bottom: window.innerHeight - rect.top + 4 }) });
  }, [anchor]);
  useEffect(() => {
    // A select or menu opened from inside the panel renders its popup in the
    // body: a press there, or the Escape that closes it, belongs to that popup.
    const nested = (target: EventTarget | null) => target instanceof Element && !!target.closest(NESTED_LAYER);
    const outside = (event: MouseEvent) => {
      const target = event.target as Node;
      if (nested(event.target)) return;
      if (!panel.current?.contains(target) && !anchor.current?.contains(target)) onClose();
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || document.querySelector(OPEN_NESTED_LAYER)) return;
      event.stopPropagation(); onClose();
    };
    document.addEventListener("mousedown", outside);
    document.addEventListener("keydown", escape, true);
    return () => { document.removeEventListener("mousedown", outside); document.removeEventListener("keydown", escape, true); };
  }, [anchor, onClose]);
  // In the body: a transformed pane would make `fixed` relative to itself.
  return createPortal(<div
    ref={panel}
    role="dialog"
    aria-label={label}
    style={place}
    onKeyDown={event => event.stopPropagation()}
    className={clsx("fixed z-50 rounded-xl bg-popover p-1.5 text-popover-foreground shadow-xl ring-1 ring-border", width)}
  >{children}</div>, document.body);
}
