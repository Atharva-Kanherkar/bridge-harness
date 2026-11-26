import { cva, type VariantProps } from "class-variance-authority";
import * as React from "react";
import { cn } from "@/lib/utils";

/**
 * A pressable cell in a chart or calendar: a muted tile that deepens on
 * hover and is ringed while pressed (selected). The caller draws the data
 * inside it.
 */
export const dataCellVariants = cva(
  "group relative flex select-none flex-col overflow-hidden rounded-lg text-left outline-none transition-[background-color,box-shadow] focus-visible:ring-2 focus-visible:ring-ring aria-pressed:shadow-[inset_0_0_0_1.5px_var(--color-ring)]",
  {
    defaultVariants: { size: "default", filled: false },
    variants: {
      size: {
        compact: "h-9 px-1.5 pt-1",
        default: "h-19 px-2.5 pb-2 pt-2",
      },
      // Whether the cell holds a value; an empty one recedes.
      filled: {
        true: "bg-muted/50 hover:bg-muted/80",
        false: "bg-muted/20 hover:bg-muted/40",
      },
    },
  },
);

export const DataCell = React.forwardRef<
  HTMLButtonElement,
  Omit<React.ComponentProps<"button">, "type"> & VariantProps<typeof dataCellVariants>
>(function DataCell({ className, size, filled, ...props }, ref) {
  return (
    <button
      ref={ref}
      type="button"
      className={cn(dataCellVariants({ size, filled }), className)}
      data-slot="data-cell"
      {...props}
    />
  );
});
