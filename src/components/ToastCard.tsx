import { useEffect, useRef, type CSSProperties, type ReactNode } from "react";
import { X } from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { RowButton } from "@/components/ui/row";

// The one card every toast draws: glass, an icon, a body that opens what it is
// about, an optional action area, and the X. It also owns the timer, so every
// kind expires the same way.

/** Toasts of one kind shown at once; the rest are counted, not stacked. */
export const TOASTS_SHOWN = 3;

export function ToastCard({
  toastKey,
  firedAt,
  durationMs,
  onDismiss,
  icon,
  body,
  openLabel,
  onOpen,
  footer,
  entrance = "animate-page-mount",
  className,
  style,
}: {
  toastKey: string;
  /** A fresh occurrence of the same key restarts the timer. */
  firedAt?: number;
  /** How long it stays once shown; `null` until the person dismisses it. */
  durationMs: number | null;
  onDismiss: (key: string) => void;
  icon: ReactNode;
  body: ReactNode;
  /** The body button's accessible name, "<headline> — <where it goes>". */
  openLabel?: string;
  /** Without it the body is plain text. */
  onOpen?: () => void;
  footer?: ReactNode;
  /** The class that animates it in. */
  entrance?: string;
  className?: string;
  style?: CSSProperties;
}) {
  // Parents recreate `onDismiss` on every render (App's git poll renders every
  // few seconds), which is not a reason to restart the timer. Only a new key
  // or occurrence is.
  const onDismissRef = useRef(onDismiss);
  onDismissRef.current = onDismiss;
  useEffect(() => {
    if (durationMs === null) return;
    const timer = window.setTimeout(() => onDismissRef.current(toastKey), durationMs);
    return () => window.clearTimeout(timer);
  }, [toastKey, firedAt, durationMs]);

  return <div
    className={cn("u-glass-popover pointer-events-auto flex w-[min(22rem,calc(100vw-1.5rem))] items-start gap-2.5 rounded-xl border border-border p-3 shadow-2xl", entrance, className)}
    style={style}
  >
    {icon}
    <div className="min-w-0 flex-1">
      {onOpen
        ? <RowButton size="bare" tone="default" shape="card" onClick={onOpen} aria-label={openLabel}><span className="block w-full">{body}</span></RowButton>
        : body}
      {footer}
    </div>
    <Button variant="quiet" size="icon-xs" className="shrink-0"
      onClick={() => onDismiss(toastKey)}
      aria-label="Dismiss notification"
    >
      <X size={13} aria-hidden="true" />
    </Button>
  </div>;
}

/** The count of toasts of one kind waiting behind the shown ones. */
export function ToastOverflow({ count }: { count: number }) {
  if (count <= 0) return null;
  return <div className="pointer-events-none rounded-md bg-muted/70 px-2 py-0.5 text-caption text-muted-foreground">
    +{count} more waiting
  </div>;
}
