import { CircleAlert, CircleCheck, Hand } from "lucide-react";
import { cn } from "@/lib/utils";
import type { AttentionCopy } from "../attentionCopy";
import { ToastCard, ToastOverflow, TOASTS_SHOWN } from "./ToastCard";

// Attention toasts, first in the shared top-right stack: "Bridge needs you"
// reads near the window chrome the person already glances at.

/** How long a toast stays when it does not say. */
export const ATTENTION_TOAST_TTL_MS = 16_000;

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

function AttentionCard({ toast, onOpen, onDismiss, onAction }: {
  toast: AttentionToast;
  onOpen: (toast: AttentionToast) => void;
  onDismiss: (key: string) => void;
  onAction?: (toast: AttentionToast) => void;
}) {
  return <ToastCard
    toastKey={toast.key}
    firedAt={toast.firedAt}
    durationMs={toast.durationMs === undefined ? ATTENTION_TOAST_TTL_MS : toast.durationMs}
    onDismiss={onDismiss}
    icon={<ToneIcon tone={toast.copy.tone} />}
    onOpen={() => onOpen(toast)}
    openLabel={`${toast.copy.headline} — open the chat`}
    body={<>
      <span className="block text-[13px] font-medium leading-snug text-foreground">{toast.copy.headline}</span>
      {/* A remedy needs its reason read in full, so that detail may wrap. */}
      <span className={cn("mt-0.5 block text-[12px] text-muted-foreground", toast.actionLabel ? "line-clamp-2" : "truncate")}>{toast.copy.detail}</span>
    </>}
    footer={toast.actionLabel && onAction && <button
      type="button"
      onClick={() => onAction(toast)}
      className="mt-2 h-7 rounded-md border border-border px-2 text-[12px] font-medium text-foreground transition-colors hover:bg-accent"
    >
      {toast.actionLabel}
    </button>}
  />;
}

export function AttentionToasts({ toasts, onOpen, onDismiss, onAction }: {
  toasts: AttentionToast[];
  onOpen: (toast: AttentionToast) => void;
  onDismiss: (key: string) => void;
  onAction?: (toast: AttentionToast) => void;
}) {
  if (!toasts.length) return null;
  // A card's timer starts when it is shown, so a queued one gets its full time.
  const shown = toasts.slice(-TOASTS_SHOWN);
  return <div
    role="region"
    aria-label="Attention notifications"
    className="pointer-events-none flex flex-col items-end gap-2"
  >
    {shown.map(toast => (
      <AttentionCard key={toast.key} toast={toast} onOpen={onOpen} onDismiss={onDismiss} onAction={onAction} />
    ))}
    <ToastOverflow count={toasts.length - shown.length} />
  </div>;
}
