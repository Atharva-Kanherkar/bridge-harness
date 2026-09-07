import { useEffect } from "react";
import { CircleCheck, CircleX, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { ciNotificationText, type GithubCiFinishedPayload } from "../githubSurface";

// The CI-finished notification stack. One card per terminal rollup, dedup'd
// upstream by `ciToastKey`; the card body deep-links to the PR, the X only
// dismisses. Auto-dismissal is slow on purpose — a CI verdict is worth a
// glance, not a chase.

export const CI_TOAST_TTL_MS = 15_000;

export type CiToast = { key: string; payload: GithubCiFinishedPayload };

function ToastCard({ toast, onOpen, onDismiss }: { toast: CiToast; onOpen: (toast: CiToast) => void; onDismiss: (key: string) => void }) {
  const text = ciNotificationText(toast.payload);
  useEffect(() => {
    const timer = window.setTimeout(() => onDismiss(toast.key), CI_TOAST_TTL_MS);
    return () => window.clearTimeout(timer);
  }, [toast.key, onDismiss]);
  const Icon = text.tone === "failure" ? CircleX : CircleCheck;
  return <div className="u-glass-popover pointer-events-auto flex w-[min(22rem,calc(100vw-1.5rem))] items-start gap-2.5 rounded-xl border border-border p-3 shadow-2xl animate-page-mount">
    <Icon size={16} aria-hidden="true" className={cn("mt-0.5 shrink-0", text.tone === "failure" ? "text-destructive" : "text-success")} />
    <button type="button" onClick={() => onOpen(toast)} className="min-w-0 flex-1 text-left" aria-label={`${text.headline} — open the pull request`}>
      <span className="block text-[12.5px] font-medium leading-snug text-foreground">{text.headline}</span>
      <span className="mt-0.5 block truncate text-[11.5px] text-muted-foreground">{text.detail}</span>
    </button>
    <button type="button" onClick={() => onDismiss(toast.key)} aria-label="Dismiss notification" className="shrink-0 rounded-md p-0.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">
      <X size={13} aria-hidden="true" />
    </button>
  </div>;
}

export function GithubToasts({ toasts, hint, onOpen, onDismiss, onDismissHint }: {
  toasts: CiToast[];
  /** The jump-to-diff fallback line ("branch isn’t checked out here"). */
  hint?: string;
  onOpen: (toast: CiToast) => void;
  onDismiss: (key: string) => void;
  onDismissHint: () => void;
}) {
  if (!toasts.length && !hint) return null;
  return <div className="pointer-events-none fixed bottom-3 right-3 z-30 flex flex-col items-end gap-2 sm:bottom-[18px] sm:right-[18px]">
    {hint && <div className="u-glass-popover pointer-events-auto flex max-w-[min(22rem,calc(100vw-1.5rem))] items-start gap-2 rounded-xl border border-border px-3 py-2 shadow-2xl animate-page-mount">
      <span className="min-w-0 flex-1 text-[11.5px] leading-relaxed text-muted-foreground">{hint}</span>
      <button type="button" onClick={onDismissHint} aria-label="Dismiss hint" className="shrink-0 rounded-md p-0.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"><X size={12} aria-hidden="true" /></button>
    </div>}
    {toasts.map(toast => <ToastCard key={toast.key} toast={toast} onOpen={onOpen} onDismiss={onDismiss} />)}
  </div>;
}
