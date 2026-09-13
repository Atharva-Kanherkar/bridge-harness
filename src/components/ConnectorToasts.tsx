import { useEffect } from "react";
import { AtSign, FileText, Mail, MessageSquare, Reply, X } from "lucide-react";
import { cn } from "@/lib/utils";
import type { ConnectorToast } from "../connectorSurface";

// The connector notification stack. One card per inbox item, keyed by item so
// the harness-rendered headline *replaces* the placeholder in a toast that is
// already on screen rather than pushing a second one — a single DM is a single
// notification, at two levels of polish.
//
// Clicking the body opens the item in the dock pane. The TTL is long: a message
// addressed to you is worth a glance, not a chase.

export const CONNECTOR_TOAST_TTL_MS = 20_000;

/**
 * Per-family glyphs, with a default that is right rather than arbitrary.
 *
 * A table instead of a conditional so an unlisted family gets the generic
 * message glyph — which is true of every connector here — rather than whichever
 * icon happened to be on the else branch.
 */
const FAMILY_ICON: Record<string, typeof MessageSquare> = {
  slack: MessageSquare,
  gmail: Mail,
  linear: AtSign,
  notion: FileText,
};

function KindIcon({ family }: { family: string }) {
  const Icon = FAMILY_ICON[family] ?? MessageSquare;
  return <Icon size={14} aria-hidden="true" className="mt-0.5 shrink-0 text-muted-foreground" />;
}

function ToastCard({ toast, index, onOpen, onDismiss }: {
  toast: ConnectorToast;
  index: number;
  onOpen: (toast: ConnectorToast) => void;
  onDismiss: (key: string) => void;
}) {
  useEffect(() => {
    const timer = window.setTimeout(() => onDismiss(toast.key), CONNECTOR_TOAST_TTL_MS);
    return () => window.clearTimeout(timer);
  }, [toast.key, onDismiss]);

  return <div
    className="u-glass-popover connector-toast-in pointer-events-auto flex w-[min(23rem,calc(100vw-1.5rem))] items-start gap-2.5 rounded-xl border border-border p-3 shadow-2xl"
    style={{ "--connector-toast-delay": `${Math.min(index, 4) * 60}ms` } as React.CSSProperties}
  >
    <KindIcon family={toast.family} />
    <button
      type="button"
      onClick={() => onOpen(toast)}
      className="min-w-0 flex-1 text-left"
      aria-label={`${toast.headline} — open in the inbox`}
    >
      <span
        key={toast.headline}
        className={cn(
          "block text-[13px] font-medium leading-snug text-foreground",
          // Unsettled: the render run is still in flight, so the wording is
          // Bridge's own and says so by shimmering. Settled: it just sharpened,
          // and the lift marks the swap without a colour change.
          toast.settled ? "connector-settle" : "connector-shimmer",
        )}
      >
        {toast.headline}
      </span>
      <span className="mt-0.5 flex items-center gap-1.5 text-[11px] text-muted-foreground">
        <span className="truncate">{toast.detail}</span>
        <Reply size={10} aria-hidden="true" className="shrink-0" />
        <span className="shrink-0">Reply here</span>
      </span>
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

export function ConnectorToasts({ toasts, suppressed = false, onOpen, onDismiss }: {
  toasts: ConnectorToast[];
  /** True while the Inbox pane is the one on screen. Telling someone about a
   *  message they are already looking at is noise, and the stack sits exactly
   *  where that pane's reply box is. */
  suppressed?: boolean;
  onOpen: (toast: ConnectorToast) => void;
  onDismiss: (key: string) => void;
}) {
  if (suppressed || !toasts.length) return null;
  // Newest nearest the corner, and never more than three at once: a burst of
  // mentions should not become a wall the user has to clear.
  const shown = toasts.slice(-3);
  return <div
    role="region"
    aria-label="Connector notifications"
    className="pointer-events-none fixed bottom-3 right-3 z-30 flex flex-col items-end gap-2 sm:bottom-[18px] sm:right-[18px]"
  >
    {shown.map((toast, index) => <ToastCard key={toast.key} toast={toast} index={index} onOpen={onOpen} onDismiss={onDismiss} />)}
    {toasts.length > shown.length && <div className="pointer-events-none rounded-md bg-muted/70 px-2 py-0.5 text-[11px] text-muted-foreground">
      +{toasts.length - shown.length} more waiting
    </div>}
  </div>;
}
