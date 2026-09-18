import { useEffect } from "react";
import { CircleAlert, CircleCheck, Hand, X } from "lucide-react";
import { cn } from "@/lib/utils";
import type { AttentionCopy } from "../attentionCopy";

// Attention toast stack — same glass card language as GithubToasts /
// ConnectorToasts / UpdateToast. Sits top-right so it does not fight the
// bottom-right CI and connector stacks, and so "Bridge needs you" reads near
// the window chrome the human already glances at.

export const ATTENTION_TOAST_TTL_MS = 16_000;

export type AttentionToast = {
  key: string;
  sessionId: string;
  copy: AttentionCopy;
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

function ToastCard({ toast, onOpen, onDismiss }: {
  toast: AttentionToast;
  onOpen: (toast: AttentionToast) => void;
  onDismiss: (key: string) => void;
}) {
  useEffect(() => {
    const timer = window.setTimeout(() => onDismiss(toast.key), ATTENTION_TOAST_TTL_MS);
    return () => window.clearTimeout(timer);
  }, [toast.key, onDismiss]);

  return <div className="u-glass-popover pointer-events-auto flex w-[min(22rem,calc(100vw-1.5rem))] items-start gap-2.5 rounded-xl border border-border p-3 shadow-2xl animate-page-mount">
    <ToneIcon tone={toast.copy.tone} />
    <button
      type="button"
      onClick={() => onOpen(toast)}
      className="min-w-0 flex-1 text-left"
      aria-label={`${toast.copy.headline} — open the chat`}
    >
      <span className="block text-[13px] font-medium leading-snug text-foreground">{toast.copy.headline}</span>
      <span className="mt-0.5 block truncate text-[12px] text-muted-foreground">{toast.copy.detail}</span>
    </button>
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

export function AttentionToasts({ toasts, onOpen, onDismiss }: {
  toasts: AttentionToast[];
  onOpen: (toast: AttentionToast) => void;
  onDismiss: (key: string) => void;
}) {
  if (!toasts.length) return null;
  const shown = toasts.slice(-3);
  return <div
    role="region"
    aria-label="Attention notifications"
    className={cn(
      "pointer-events-none fixed top-3 right-3 z-30 flex flex-col items-end gap-2",
      "sm:top-[18px] sm:right-[18px]",
    )}
  >
    {shown.map(toast => (
      <ToastCard key={toast.key} toast={toast} onOpen={onOpen} onDismiss={onDismiss} />
    ))}
  </div>;
}
