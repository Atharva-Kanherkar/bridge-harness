import type { ReactNode } from "react";
import { useEffect } from "react";
import { AnimatePresence, motion, usePresence } from "framer-motion";
import { X } from "lucide-react";
import { cn } from "@/lib/utils";
import { MOTION_DURATION, useMotionTransition } from "../motion";

/**
 * The chrome the two create dialogs share: scrim, floating panel, and a header
 * carrying an icon, a title, a one-line subtitle, and a close button.
 *
 * Only presentation lives here. Focus management and Escape handling stay with
 * each dialog because they differ: the workspace dialog focuses its name field
 * and always closes on Escape, while the orchestrator dialog focuses an action
 * and ignores Escape while a worktree is being created.
 *
 * The shell also owns the open/closed *boundary*. It used to be each dialog's
 * job — `if (!open) return null` — which meant every dialog animated in and then
 * vanished on the same render it closed, because React had already unmounted the
 * tree before any exit could play. Holding the boundary in one place gives every
 * dialog built on the shell an exit for free.
 */
export type CreateDialogShellProps = {
  /** Whether the dialog is showing. The shell keeps the tree mounted while it
   *  animates out, so callers must pass this instead of returning `null`. */
  open: boolean;
  /** Id of the heading, wired to the dialog's `aria-labelledby`. */
  titleId: string;
  icon: ReactNode;
  title: string;
  subtitle: string;
  /** Extra classes for the subtitle, e.g. `truncate` for a workspace name. */
  subtitleClassName?: string;
  closeLabel: string;
  closeDisabled?: boolean;
  onClose: () => void;
  /** When true, a click on the scrim or an Escape inside it closes the dialog. */
  dismissOnScrim?: boolean;
  children: ReactNode;
};

type DialogSurfaceProps = Omit<CreateDialogShellProps, "open">;

export function CreateDialogShell({ open, ...surface }: CreateDialogShellProps) {
  return (
    <AnimatePresence>
      {open && <DialogSurface {...surface} />}
    </AnimatePresence>
  );
}

function DialogSurface({
  titleId,
  icon,
  title,
  subtitle,
  subtitleClassName,
  closeLabel,
  closeDisabled,
  onClose,
  dismissOnScrim = false,
  children,
}: DialogSurfaceProps) {
  const transition = useMotionTransition(MOTION_DURATION.overlay);
  // Once the exit starts the dialog is already logically gone: drop it from the
  // accessibility tree and from hit-testing, so a stray click or focus event in
  // the ~260ms the fade takes cannot act on a dialog the user just closed.
  const [isPresent, safeToRemove] = usePresence();
  // Consuming `usePresence` makes this component responsible for signalling its
  // own completion; AnimatePresence still holds the tree until the exit
  // animations below have finished before honoring it.
  useEffect(() => {
    if (!isPresent) safeToRemove();
  }, [isPresent, safeToRemove]);
  return (
    <motion.div
      className={cn(
        "fixed inset-0 z-50 flex items-start justify-center overflow-y-auto bg-scrim p-4 pt-[6vh] backdrop-blur-md sm:pt-[10vh]",
        !isPresent && "pointer-events-none",
      )}
      onClick={dismissOnScrim ? event => { if (event.target === event.currentTarget) onClose(); } : undefined}
      onKeyDown={dismissOnScrim ? event => { if (event.key === "Escape") onClose(); } : undefined}
      role="dialog"
      aria-modal="true"
      aria-labelledby={titleId}
      aria-hidden={isPresent ? undefined : true}
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={transition}
    >
      {/* The panel travels a little further than the scrim fades, so the
          dialog reads as leaving rather than merely dimming. */}
      <motion.div
        className="u-overlay-strong flex max-h-[90dvh] w-full max-w-md flex-col overflow-hidden rounded-3xl"
        initial={{ opacity: 0, y: 10, scale: 0.985 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        exit={{ opacity: 0, y: 6, scale: 0.99 }}
        transition={transition}
      >
        <div className="flex shrink-0 items-center justify-between gap-3 border-b border-border px-5 py-4">
          <div className="flex min-w-0 items-center gap-3">
            <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-[12px] border border-border bg-muted">
              {icon}
            </span>
            <div className="min-w-0">
              <h2 id={titleId} className="font-display text-base font-semibold text-foreground">
                {title}
              </h2>
              <p className={cn("mt-0.5 text-[13px] text-muted-foreground", subtitleClassName)}>
                {subtitle}
              </p>
            </div>
          </div>
          <button
            type="button"
            onClick={onClose}
            disabled={closeDisabled}
            className="shrink-0 rounded-xl p-2 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-40"
            aria-label={closeLabel}
          >
            <X className="h-4 w-4" strokeWidth={2} aria-hidden="true" />
          </button>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto p-5">{children}</div>
      </motion.div>
    </motion.div>
  );
}
