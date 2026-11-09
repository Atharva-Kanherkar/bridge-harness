import { useEffect, useRef } from "react";
import { CircleAlert, CircleCheck, Hand, X } from "lucide-react";
import { cn } from "@/lib/utils";
import type { AttentionCopy } from "../attentionCopy";

// Attention toast stack — same glass card language as GithubToasts /
// ConnectorToasts / UpdateToast. Sits top-right so it does not fight the
// bottom-right CI and connector stacks, and so "Bridge needs you" reads near
// the window chrome the human already glances at.

/** How long a toast stays when it does not say. */
export const ATTENTION_TOAST_TTL_MS = 16_000;
/** Toasts shown at once; the rest of the queue waits its turn. */
export const ATTENTION_TOASTS_SHOWN = 3;

export type AttentionToast = {
  key: string;
  sessionId: string;
  copy: AttentionCopy;
  // Distinguishes a fresh occurrence of the same key (a chat that re-enters
  // `waiting` before its last card expired) so the TTL below can restart —
  // without this, App re-renders that only change unrelated state (e.g. the
  // git-stat poll) would pass a same-key toast through and must NOT restart it.
  firedAt?: number;
  /** A one-click remedy beside the body, such as delivering stranded input. */
  actionLabel?: string;
  /** How long it stays once shown; `null` until the person acts on it. */
  durationMs?: number | null;
};

function ToneIcon({ tone }: { tone: AttentionCopy["tone"] }) {
  if (tone === "needs-you") {
    return <Hand size={16} aria-hidden="true" className="mt-0.5 shrink-0 text-warning" />;
  }
  if (tone === "failed") {
    return <CircleAlert size={16} aria-hidden="true" className="mt-0.5 shrink-0 text-destructive" />;
  }
  return <CircleCheck size={16} aria-hidden="true" className="mt-0.5 shrink-0 text-success" />;
}

function ToastCard({ toast, onOpen, onDismiss, onAction }: {
  toast: AttentionToast;
  onOpen: (toast: AttentionToast) => void;
  onDismiss: (key: string) => void;
  onAction?: (toast: AttentionToast) => void;
}) {
  // `onDismiss` is a fresh closure on every App render (e.g. its ~5s
  // git-stat poll), which is not a reason to restart this timer — only a
  // new `key`/`firedAt` occurrence is. Read the latest callback from a ref
  // instead of depending on it directly.
  const onDismissRef = useRef(onDismiss);
  onDismissRef.current = onDismiss;
  const durationMs = toast.durationMs === undefined ? ATTENTION_TOAST_TTL_MS : toast.durationMs;
  useEffect(() => {
    if (durationMs === null) return;
    const timer = window.setTimeout(() => onDismissRef.current(toast.key), durationMs);
    return () => window.clearTimeout(timer);
  }, [toast.key, toast.firedAt, durationMs]);

  return <div className="u-glass-popover pointer-events-auto flex w-[min(22rem,calc(100vw-1.5rem))] items-start gap-2.5 rounded-xl border border-border p-3 shadow-2xl animate-page-mount">
    <ToneIcon tone={toast.copy.tone} />
    <div className="min-w-0 flex-1">
      <button
        type="button"
        onClick={() => onOpen(toast)}
        className="block w-full text-left"
        aria-label={`${toast.copy.headline} — open the chat`}
      >
        <span className="block text-[13px] font-medium leading-snug text-foreground">{toast.copy.headline}</span>
        {/* A remedy needs its reason read in full, so that detail may wrap. */}
        <span className={cn("mt-0.5 block text-[12px] text-muted-foreground", toast.actionLabel ? "line-clamp-2" : "truncate")}>{toast.copy.detail}</span>
      </button>
      {toast.actionLabel && onAction && <button
        type="button"
        onClick={() => onAction(toast)}
        className="mt-2 h-7 rounded-md border border-border px-2 text-[12px] font-medium text-foreground transition-colors hover:bg-accent"
      >
        {toast.actionLabel}
      </button>}
    </div>
    <button
      type="button"
      onClick={() => onDismiss(toast.key)}
      aria-label="Dismiss notification"
      className="grid size-7 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
    >
      <X size={13} aria-hidden="true" />
    </button>
  </div>;
}

export function AttentionToasts({ toasts, onOpen, onDismiss, onAction }: {
  toasts: AttentionToast[];
  onOpen: (toast: AttentionToast) => void;
  onDismiss: (key: string) => void;
  onAction?: (toast: AttentionToast) => void;
}) {
  if (!toasts.length) return null;
  // A card's timer starts when it is shown, so a queued one gets its full time.
  const shown = toasts.slice(-ATTENTION_TOASTS_SHOWN);
  return <div
    role="region"
    aria-label="Attention notifications"
    className="pointer-events-none flex flex-col items-end gap-2"
  >
    {shown.map(toast => (
      <ToastCard key={toast.key} toast={toast} onOpen={onOpen} onDismiss={onDismiss} onAction={onAction} />
    ))}
  </div>;
}
