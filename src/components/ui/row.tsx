import { mergeProps } from "@base-ui/react/merge-props";
import { useRender } from "@base-ui/react/use-render";
import { cva, type VariantProps } from "class-variance-authority";
import * as React from "react";
import { cn } from "@/lib/utils";

/**
 * A pressable row: list items, menu-like choices, tree nodes, anything that
 * fills its container's width and is clicked as a whole. A row owns its hover,
 * focus, selected and disabled states; the caller lays out what is inside it.
 *
 * Use Button for a control that sits beside other content, RowButton for one
 * that is the line.
 */
export const rowVariants = cva(
  "relative flex w-full min-w-0 cursor-default items-center text-left outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset disabled:pointer-events-none disabled:opacity-40 aria-disabled:pointer-events-none aria-disabled:opacity-40",
  {
    defaultVariants: { size: "default", tone: "default", shape: "rounded" },
    variants: {
      size: {
        sm: "min-h-7 gap-1.5 px-1.5 py-0.5 text-caption",
        default: "min-h-8 gap-2 px-2 py-1 text-ui",
        lg: "min-h-10 gap-2.5 px-2.5 py-1.5 text-ui",
        // A clickable card or panel row.
        card: "min-h-14 gap-3 px-4 py-3.5 text-ui",
        // The caller sets the padding: cards and multi-line rows.
        bare: "gap-2 text-ui",
      },
      tone: {
        default:
          "text-foreground hover:bg-accent data-[active=true]:bg-accent aria-selected:bg-accent aria-pressed:bg-accent aria-expanded:bg-accent aria-[current=true]:bg-accent",
        muted:
          "text-muted-foreground hover:bg-accent hover:text-foreground data-[active=true]:bg-accent data-[active=true]:text-foreground aria-selected:bg-accent aria-selected:text-foreground aria-pressed:bg-accent aria-pressed:text-foreground aria-[current=true]:bg-accent aria-[current=true]:text-foreground",
        // Selection is drawn with the selection token rather than the hover fill.
        selection:
          "text-foreground hover:bg-accent data-[active=true]:bg-selection data-[active=true]:text-selection-foreground data-[active=true]:hover:bg-selection aria-selected:bg-selection aria-selected:text-selection-foreground aria-selected:hover:bg-selection",
        // Navigation: muted at rest, the selection fill on the current page.
        nav: "text-muted-foreground hover:bg-accent hover:text-foreground data-[active=true]:bg-selection data-[active=true]:font-medium data-[active=true]:text-selection-foreground data-[active=true]:hover:bg-selection",
        // A vertical tab list: the selected tab is raised like a segmented item.
        tab: "text-foreground hover:bg-accent aria-selected:bg-card aria-selected:shadow-control aria-selected:hover:bg-card",
        // A choice card: bordered, and marked when checked or pressed.
        choice: "border border-border text-foreground hover:border-foreground/25 hover:bg-accent aria-checked:border-info/60 aria-checked:bg-info/10 aria-checked:hover:bg-info/10 aria-pressed:border-info/60 aria-pressed:bg-info/10",
        // A raised card that fills on hover.
        surface: "u-surface text-foreground hover:bg-accent",
        // No fill at all; the content changes instead (a disclosure, a heading).
        plain: "text-muted-foreground hover:text-foreground",
      },
      shape: {
        rounded: "rounded-md",
        card: "rounded-lg",
        panel: "rounded-2xl",
        flush: "rounded-none",
      },
    },
  },
);

export interface RowButtonProps extends useRender.ComponentProps<"button"> {
  size?: VariantProps<typeof rowVariants>["size"];
  tone?: VariantProps<typeof rowVariants>["tone"];
  shape?: VariantProps<typeof rowVariants>["shape"];
  /** The row is the current or selected one. */
  active?: boolean;
}

export const RowButton = React.forwardRef<HTMLButtonElement, RowButtonProps>(
  function RowButton(
    { className, size, tone, shape, active, render, ...props },
    forwardedRef,
  ): React.ReactElement {
    const defaultProps = {
      className: cn(rowVariants({ className, size, tone, shape })),
      "data-active": active ? "true" : undefined,
      "data-slot": "row",
      type: render ? undefined : ("button" as const),
    };
    return useRender({
      defaultTagName: "button",
      props: mergeProps<"button">(defaultProps, props),
      ref: forwardedRef,
      render,
    }) as React.ReactElement;
  },
);
