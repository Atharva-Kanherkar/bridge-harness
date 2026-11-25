import type * as React from "react";
import { cn } from "@/lib/utils";

/**
 * 32x18. On is a foreground track with a background knob; there is no hue.
 *
 * Controlled only, and a plain button rather than a hidden checkbox: a setting
 * flips when the save lands, so the switch reports the requested value and
 * re-renders from its prop.
 */
export function Switch({
  checked,
  onCheckedChange,
  className,
  ...props
}: Omit<React.ComponentProps<"button">, "type" | "role" | "onChange"> & {
  checked: boolean;
  onCheckedChange: (next: boolean) => void;
}): React.ReactElement {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      data-slot="switch"
      onClick={() => onCheckedChange(!checked)}
      className={cn(
        "relative inline-flex h-4.5 w-8 shrink-0 items-center rounded-full p-0.5 outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-1 focus-visible:ring-offset-background disabled:opacity-40",
        checked ? "bg-foreground" : "bg-foreground/15",
        className,
      )}
      {...props}
    >
      <span
        data-slot="switch-thumb"
        className={cn(
          "pointer-events-none block size-3.5 rounded-full bg-background transition-transform duration-150",
          checked && "translate-x-3.5",
        )}
      />
    </button>
  );
}
