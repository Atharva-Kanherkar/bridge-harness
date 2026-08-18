import type { ReactNode } from "react";
import { X } from "lucide-react";
import { cn } from "@/lib/utils";

/**
 * The chrome the two create dialogs share: scrim, floating panel, and a header
 * carrying an icon, a title, a one-line subtitle, and a close button.
 *
 * Only presentation lives here. Focus management and Escape handling stay with
 * each dialog because they differ: the workspace dialog focuses its name field
 * and always closes on Escape, while the orchestrator dialog focuses an action
 * and ignores Escape while a worktree is being created.
 */
export type CreateDialogShellProps = {
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

export function CreateDialogShell({
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
}: CreateDialogShellProps) {
  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center overflow-y-auto bg-scrim p-4 pt-[6vh] backdrop-blur-md sm:pt-[10vh]"
      onClick={dismissOnScrim ? event => { if (event.target === event.currentTarget) onClose(); } : undefined}
      onKeyDown={dismissOnScrim ? event => { if (event.key === "Escape") onClose(); } : undefined}
      role="dialog"
      aria-modal="true"
      aria-labelledby={titleId}
    >
      <div className="u-overlay-strong animate-page-enter flex max-h-[90dvh] w-full max-w-md flex-col overflow-hidden rounded-3xl">
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
      </div>
    </div>
  );
}
