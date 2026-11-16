import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { ChevronRight, type LucideIcon } from "lucide-react";
import { cn } from "@/lib/utils";
import { Kbd } from "@/components/ui/kbd";

// The pane's one confirm surface for every write that is not a merge: approve,
// request changes, close, comment, labels, checkout — a sheet that rises from
// the bottom of the GitHub pane and dims only that pane, never the chat beside
// it. It names the action, says the consequence in one sentence, keeps the
// exact `gh` command behind a disclosure, and labels the button with the verb.
// The app-wide Dialog is reserved for setup flows (connecting a repository);
// no PR action opens one.

export type ActionSheetTone = "neutral" | "success" | "destructive";

export type ActionSheetProps = {
  icon: LucideIcon;
  tone?: ActionSheetTone;
  title: string;
  consequence: string;
  /** The exact `gh` command behind the "Exact command" disclosure. Actions
   *  that are not a single gh invocation (checkout) omit it. */
  command?: string;
  /** Extra, quieter context under the header (checkout's worktree note). */
  note?: string;
  requiresBody?: boolean;
  body?: string;
  onBody?: (value: string) => void;
  /** The body was already composed elsewhere — show it, don't ask for it. */
  readOnlyBody?: boolean;
  confirmLabel: string;
  destructive?: boolean;
  busy?: boolean;
  onCancel: () => void;
  onConfirm: () => void;
};

const ENTER_EASE: [number, number, number, number] = [0.34, 1.56, 0.64, 1];
const EXIT_EASE: [number, number, number, number] = [0.4, 0, 1, 1];

const BADGE_TONE: Record<ActionSheetTone, string> = {
  neutral: "bg-muted text-foreground",
  success: "bg-success/12 text-success",
  destructive: "bg-destructive/12 text-destructive",
};

/** One focusable element list for the trap, in DOM order. */
function focusables(sheet: HTMLElement): HTMLElement[] {
  return [...sheet.querySelectorAll<HTMLElement>("button, textarea, input, [href], summary, [tabindex]:not([tabindex='-1'])")]
    .filter(element => !element.hasAttribute("disabled") && element.offsetParent !== null);
}

/** Stable portal point for pane-scoped overlays. Renders an invisible anchor,
 *  resolves the pane root once (`[data-gh-pane-root]`), and portals an
 *  AnimatePresence into it — so the sheet's exit motion survives the one
 *  render where the pending action clears. */
export function ActionSheetHost({ children }: { children: ReactNode }) {
  const anchor = useRef<HTMLDivElement>(null);
  const [host, setHost] = useState<HTMLElement | null>(null);
  useEffect(() => {
    setHost(anchor.current?.closest<HTMLElement>("[data-gh-pane-root]") ?? null);
  }, []);
  return <div ref={anchor} className="hidden" aria-hidden="true">
    {host && createPortal(<AnimatePresence initial={false}>{children}</AnimatePresence>, host)}
  </div>;
}

/** The sheet surface itself. Render it inside an `ActionSheetHost` and toggle
 *  it with the pending action; the host's AnimatePresence plays the exit. */
export function ActionSheet(props: ActionSheetProps) {
  return <SheetSurface {...props} />;
}

/** The sheet itself. Nested inside an AnimatePresence so its exit motion has
 *  one render to play in; children stagger at the mockup's 40ms. */
function SheetSurface({ icon: Icon, tone = "neutral", title, consequence, command, note, requiresBody, body, onBody, readOnlyBody, confirmLabel, destructive, busy, onCancel, onConfirm }: ActionSheetProps) {
  const reduced = useReducedMotion();
  const sheetRef = useRef<HTMLDivElement>(null);
  // Focus came from the trigger; it goes back there when the sheet leaves.
  const returnFocus = useRef<Element | null>(null);

  useEffect(() => {
    returnFocus.current = document.activeElement;
    const sheet = sheetRef.current;
    const focusFirst = () => {
      if (requiresBody && !readOnlyBody) sheet?.querySelector<HTMLTextAreaElement>("textarea")?.focus();
      else sheet?.querySelector<HTMLButtonElement>("button[data-confirm]")?.focus();
    };
    focusFirst();
    return () => {
      const target = returnFocus.current;
      if (target instanceof HTMLElement && target.isConnected) target.focus();
    };
    // The open body is fixed for a sheet's lifetime; a new action is a new sheet.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const sheet = sheetRef.current;
      if (!sheet) return;
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        if (!busy) onCancel();
        return;
      }
      if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
        const ready = !requiresBody || (body ?? "").trim().length > 0;
        if (ready && !busy) {
          event.preventDefault();
          onConfirm();
        }
        return;
      }
      if (event.key === "Tab") {
        const items = focusables(sheet);
        if (!items.length) return;
        const first = items[0];
        const last = items[items.length - 1];
        const active = document.activeElement;
        if (event.shiftKey && (active === first || !sheet.contains(active))) {
          event.preventDefault();
          last.focus();
        } else if (!event.shiftKey && (active === last || !sheet.contains(active))) {
          event.preventDefault();
          first.focus();
        }
      }
    };
    // Capture: the pane behind keeps its own Escape (collapsing the merge
    // step), which must not double-fire while the sheet is up.
    document.addEventListener("keydown", onKeyDown, true);
    return () => document.removeEventListener("keydown", onKeyDown, true);
  }, [busy, onCancel, onConfirm, requiresBody, body]);

  const rise = (index: number, className?: string, children?: ReactNode) => (
    <motion.div
      className={className}
      initial={reduced ? { opacity: 0 } : { opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0, transition: { duration: 0.32, ease: [0.22, 1, 0.36, 1], delay: index * 0.04 } }}
    >{children}</motion.div>
  );

  return <motion.div className="absolute inset-0 z-30" initial={false}>
    <motion.div
      aria-hidden="true"
      onClick={() => { if (!busy) onCancel(); }}
      className="absolute inset-0 bg-scrim"
      initial={{ opacity: 0 }}
      animate={{ opacity: 1, transition: { duration: 0.32, ease: [0.22, 1, 0.36, 1] } }}
      exit={{ opacity: 0, transition: { duration: 0.2, ease: EXIT_EASE } }}
    />
    <motion.div
      ref={sheetRef}
      role="dialog"
      aria-modal="true"
      aria-label={title}
      className="absolute bottom-2 left-2 right-2 rounded-2xl border border-border-card bg-popover p-4 shadow-[0_16px_40px_-18px_rgb(0_0_0/0.5),0_2px_8px_-4px_rgb(0_0_0/0.22)]"
      initial={reduced ? { opacity: 0 } : { y: "calc(100% + 16px)" }}
      animate={
        reduced
          ? { opacity: 1, transition: { duration: 0.12 } }
          : { y: 0, transition: { duration: 0.38, ease: ENTER_EASE } }
      }
      exit={reduced
        ? { opacity: 0, transition: { duration: 0.12 } }
        : { y: "calc(100% + 16px)", transition: { duration: 0.2, ease: EXIT_EASE } }}
    >
      <span aria-hidden="true" className="mx-auto mb-3 block h-1 w-8 rounded-full bg-border-card" />
      {rise(0, "flex items-start gap-3", <>
        <span className={cn("grid size-[34px] shrink-0 place-items-center rounded-[10px]", BADGE_TONE[tone])}>
          <Icon size={17} strokeWidth={1.7} aria-hidden="true" />
        </span>
        <span className="min-w-0">
          <b className="block text-[14px] font-semibold leading-snug text-foreground">{title}</b>
          <p className="mt-0.5 text-[12px] leading-[1.5] text-muted-foreground">{consequence}</p>
          {note && <p className="mt-0.5 text-[12px] leading-[1.5] text-muted-foreground">{note}</p>}
        </span>
      </>)}
      {requiresBody && (readOnlyBody
        ? rise(1, "mt-3", <p
            aria-label="Comment body"
            className="max-h-40 overflow-y-auto whitespace-pre-wrap rounded-[10px] border border-border bg-card p-3 text-[12.5px] leading-relaxed text-foreground"
          >{body}</p>)
        : rise(1, "mt-3", <textarea
            value={body}
            onChange={event => onBody?.(event.target.value)}
            disabled={busy}
            aria-label="Comment body"
            placeholder="Write a comment…"
            className="block min-h-24 w-full resize-none rounded-[10px] border border-input bg-background p-2.5 text-[12.5px] leading-[1.55] text-foreground outline-none placeholder:text-muted-foreground focus-visible:border-ring disabled:opacity-45"
          />))}
      {command && rise(2, "group mt-3 rounded-lg bg-muted text-[11px] text-muted-foreground", <details className="group">
        <summary className="flex cursor-pointer list-none select-none items-center gap-1.5 px-2.5 py-1.5 transition-colors hover:text-foreground">
          <ChevronRight size={11} aria-hidden="true" className="shrink-0 transition-transform duration-200 group-open:rotate-90" />
          Exact command
        </summary>
        <code className="block break-all pb-2 pl-[26px] pr-2.5 font-mono text-[11px] text-foreground/85">{command}</code>
      </details>)}
      {rise(3, "mt-3.5 flex items-center gap-2", <>
        <button
          type="button"
          onClick={() => { if (!busy) onCancel(); }}
          disabled={busy}
          className="inline-flex min-h-7 items-center rounded-md px-2.5 text-[12px] font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-50"
        >
          Cancel
        </button>
        <Kbd className="border border-border bg-transparent text-[10.5px]">esc</Kbd>
        <span className="ml-auto" />
        <Kbd className="border border-border bg-transparent text-[10.5px]">⌘ ↵</Kbd>
        <button
          type="button"
          data-confirm=""
          onClick={onConfirm}
          disabled={busy || (requiresBody && !readOnlyBody && !(body ?? "").trim())}
          className={cn(
            "inline-flex min-h-7 items-center rounded-md px-3 text-[12px] font-medium transition-colors active:scale-[0.98] disabled:opacity-50",
            destructive ? "bg-destructive text-destructive-foreground hover:bg-destructive/90" : "bg-primary text-primary-foreground hover:bg-primary/90",
          )}
        >
          {confirmLabel}
        </button>
      </>)}
    </motion.div>
  </motion.div>;
}
