import { useEffect, useRef } from "react";
import { X } from "lucide-react";
import { Alert, AlertAction, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

export const TRANSIENT_ALERT_TTL_MS = 5_000;

export function TransientAlert({ title, message, variant, onDismiss, className }: {
  title: string;
  message: string;
  variant: "warning" | "error";
  onDismiss: () => void;
  className?: string;
}) {
  const onDismissRef = useRef(onDismiss);
  onDismissRef.current = onDismiss;

  useEffect(() => {
    const timer = window.setTimeout(() => onDismissRef.current(), TRANSIENT_ALERT_TTL_MS);
    return () => window.clearTimeout(timer);
  }, [title, message]);

  return <Alert variant={variant} className={cn(
    "u-overlay fixed right-3 bottom-3 z-40 max-w-[min(32rem,calc(100vw-1.5rem))] rounded-xl bg-popover sm:right-[18px] sm:bottom-[18px]",
    className,
  )}>
    <AlertTitle>{title}</AlertTitle>
    <AlertDescription className="max-h-[40vh] overflow-y-auto text-foreground/85">{message}</AlertDescription>
    <AlertAction>
      <Button type="button" size="icon-sm" variant="ghost" aria-label="Dismiss notification" onClick={onDismiss}>
        <X size={14} aria-hidden="true" />
      </Button>
    </AlertAction>
  </Alert>;
}
