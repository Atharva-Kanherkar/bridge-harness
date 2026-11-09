import { CircleCheck, CircleX, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { ciNotificationText, type GithubCiFinishedPayload } from "../githubSurface";
import { ToastCard, ToastOverflow, TOASTS_SHOWN } from "./ToastCard";

// The CI-finished notification stack. One card per terminal rollup, dedup'd
// upstream by `ciToastKey`; the card body deep-links to the PR, the X only
// dismisses. Auto-dismissal is slow on purpose — a CI verdict is worth a
// glance, not a chase.

export const CI_TOAST_TTL_MS = 15_000;

export type CiToast = { key: string; payload: GithubCiFinishedPayload };

function CiCard({ toast, onOpen, onDismiss }: { toast: CiToast; onOpen: (toast: CiToast) => void; onDismiss: (key: string) => void }) {
  const text = ciNotificationText(toast.payload);
  const Icon = text.tone === "failure" ? CircleX : CircleCheck;
  return <ToastCard
    toastKey={toast.key}
    durationMs={CI_TOAST_TTL_MS}
    onDismiss={onDismiss}
    icon={<Icon size={16} aria-hidden="true" className={cn("mt-0.5 shrink-0", text.tone === "failure" ? "text-destructive" : "text-success")} />}
    onOpen={() => onOpen(toast)}
    openLabel={`${text.headline} — open the pull request`}
    body={<>
      <span className="block text-[13px] font-medium leading-snug text-foreground">{text.headline}</span>
      <span className="mt-0.5 block truncate text-[12px] text-muted-foreground">{text.detail}</span>
    </>}
  />;
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
  const shown = toasts.slice(-TOASTS_SHOWN);
  return <div className="pointer-events-none flex flex-col items-end gap-2">
    {hint && <div className="u-glass-popover pointer-events-auto flex max-w-[min(22rem,calc(100vw-1.5rem))] items-start gap-2 rounded-xl border border-border px-3 py-2 shadow-2xl animate-page-mount">
      <span className="min-w-0 flex-1 text-[12px] leading-relaxed text-muted-foreground">{hint}</span>
      <button type="button" onClick={onDismissHint} aria-label="Dismiss hint" className="grid size-7 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"><X size={12} aria-hidden="true" /></button>
    </div>}
    {shown.map(toast => <CiCard key={toast.key} toast={toast} onOpen={onOpen} onDismiss={onDismiss} />)}
    <ToastOverflow count={toasts.length - shown.length} />
  </div>;
}
