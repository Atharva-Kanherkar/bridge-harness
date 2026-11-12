// The small vocabulary Notes is drawn with: a popover anchored to its trigger,
// option rows with a check for the chosen one, quiet pill actions, and a
// status glyph. Popovers render inline (absolutely positioned inside their
// anchor) so they scroll with the page and stay inside its accessibility tree.

import { useEffect, useLayoutEffect, useRef, type ReactNode, type RefObject } from "react";
import { Check } from "lucide-react";
// clsx rather than cn: tailwind-merge does not know the text-ui and
// text-caption sizes, and drops them beside a text color.
import { clsx } from "clsx";

export function Popover({ open, onClose, label, align = "start", side = "bottom", width = "w-72", children }: {
  open: boolean;
  onClose: () => void;
  label: string;
  align?: "start" | "end";
  /** Open upward for a trigger that sits at the bottom of the window. */
  side?: "top" | "bottom";
  width?: string;
  children: ReactNode;
}) {
  const panel = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const outside = (event: MouseEvent) => {
      const anchor = panel.current?.parentElement;
      if (anchor && !anchor.contains(event.target as Node)) onClose();
    };
    const escape = (event: KeyboardEvent) => { if (event.key === "Escape") { event.stopPropagation(); onClose(); } };
    document.addEventListener("mousedown", outside);
    document.addEventListener("keydown", escape);
    return () => { document.removeEventListener("mousedown", outside); document.removeEventListener("keydown", escape); };
  }, [open, onClose]);
  if (!open) return null;
  return <div
    ref={panel}
    role="dialog"
    aria-label={label}
    className={clsx("absolute z-30 rounded-xl bg-popover p-1.5 text-popover-foreground shadow-xl ring-1 ring-border", width, side === "top" ? "bottom-full mb-1.5" : "top-full mt-1.5", align === "end" ? "right-0" : "left-0")}
  >
    {children}
  </div>;
}

export function MenuLabel({ children }: { children: ReactNode }) {
  return <p className="m-0 px-2.5 pb-1 pt-2 text-[11px] font-medium text-muted-foreground first:pt-1">{children}</p>;
}

export function MenuDivider() {
  return <div role="separator" className="mx-1 my-1.5 h-px bg-border" />;
}

export function OptionList({ label, children, className }: { label: string; children: ReactNode; className?: string }) {
  return <div role="listbox" aria-label={label} className={clsx("flex flex-col", className)}>{children}</div>;
}

export function Option({ selected, onSelect, leading, hint, children }: {
  selected: boolean;
  onSelect: () => void;
  leading?: ReactNode;
  hint?: string;
  children: ReactNode;
}) {
  return <button
    type="button"
    role="option"
    aria-selected={selected}
    onClick={onSelect}
    className="flex min-h-8 w-full items-center gap-2.5 rounded-lg px-2.5 text-left text-[13px] text-foreground outline-none transition-colors hover:bg-accent focus-visible:bg-accent"
  >
    {leading && <span className="grid w-4 shrink-0 place-items-center">{leading}</span>}
    <span data-option-label className="min-w-0 flex-1 truncate">{children}</span>
    {hint && <span className="shrink-0 text-[11px] text-muted-foreground">{hint}</span>}
    <Check size={14} strokeWidth={2} aria-hidden="true" className={clsx("shrink-0", selected ? "text-foreground" : "invisible")} />
  </button>;
}

/** A quiet text action. `active` keeps it lit while its popover is open. */
export function Pill({ children, active = false, className, ...props }: React.ButtonHTMLAttributes<HTMLButtonElement> & { active?: boolean }) {
  return <button
    type="button"
    {...props}
    className={clsx(
      "inline-flex h-8 shrink-0 items-center gap-1.5 rounded-full px-3 text-[13px] transition-colors disabled:pointer-events-none disabled:opacity-50",
      active ? "bg-accent text-foreground" : "text-muted-foreground hover:bg-accent hover:text-foreground",
      className,
    )}
  >{children}</button>;
}

/** The one filled action in a popover: ink on paper, paper on ink. */
export function PrimaryButton({ children, className, ...props }: React.ButtonHTMLAttributes<HTMLButtonElement>) {
  return <button
    type="button"
    {...props}
    className={clsx("inline-flex h-8 shrink-0 items-center rounded-full bg-primary px-3.5 text-[13px] font-medium text-primary-foreground transition-opacity hover:opacity-90 disabled:opacity-40", className)}
  >{children}</button>;
}

/** Open is a ring, doing is half full, done is a check; inbox and archived are dashed. */
export function StatusGlyph({ state, size = 14 }: { state: string; size?: number }) {
  const tone = state === "done" ? "text-success" : state === "doing" ? "text-warning" : "text-muted-foreground";
  return <svg width={size} height={size} viewBox="0 0 16 16" aria-hidden="true" className={clsx("shrink-0", tone)}>
    {state === "done"
      ? <><circle cx="8" cy="8" r="7" fill="currentColor" /><path d="M5 8.2 7.1 10.2 11 6" fill="none" stroke="var(--color-background)" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" /></>
      : <>
        <circle cx="8" cy="8" r="6.25" fill="none" stroke="currentColor" strokeWidth="1.5" strokeDasharray={state === "inbox" || state === "archived" ? "2.2 2.2" : undefined} />
        {state === "doing" && <path d="M8 3.5a4.5 4.5 0 0 1 0 9z" fill="currentColor" />}
      </>}
  </svg>;
}

/** A textarea that grows with its text instead of scrolling inside itself. */
export function useAutoGrow(ref: RefObject<HTMLTextAreaElement>, value: string, active = true) {
  useLayoutEffect(() => {
    const box = ref.current;
    if (!box || !active) return;
    box.style.height = "auto";
    box.style.height = `${box.scrollHeight}px`;
  }, [ref, value, active]);
}
