// A panel floating over the page at its anchor, for menus and editors that
// open from a table cell: in the body, so a scrolling table never clips it.

import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
// clsx rather than cn: tailwind-merge does not know the text-ui and
// text-caption sizes, and drops them beside a text color.
import { clsx } from "clsx";

/** A panel that floats over the page at its anchor, so a cell in a scrolling
 *  table is never clipped. Closes on a click outside or Escape. */
export function Floating({ anchor, label, onClose, width = "w-72", children }: { anchor: React.RefObject<HTMLElement>; label: string; onClose: () => void; width?: string; children: ReactNode }) {
  const panel = useRef<HTMLDivElement>(null);
  const [place, setPlace] = useState<{ left: number; top?: number; bottom?: number }>({ left: 0, top: 0 });
  useLayoutEffect(() => {
    const rect = anchor.current?.getBoundingClientRect();
    if (!rect) return;
    const below = rect.bottom + 320 < window.innerHeight || rect.top < 320;
    setPlace({ left: Math.max(8, Math.min(rect.left, window.innerWidth - 296)), ...(below ? { top: rect.bottom + 4 } : { bottom: window.innerHeight - rect.top + 4 }) });
  }, [anchor]);
  useEffect(() => {
    const outside = (event: MouseEvent) => {
      const target = event.target as Node;
      if (!panel.current?.contains(target) && !anchor.current?.contains(target)) onClose();
    };
    const escape = (event: KeyboardEvent) => { if (event.key === "Escape") { event.stopPropagation(); onClose(); } };
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
