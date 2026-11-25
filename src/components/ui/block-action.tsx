import * as React from "react";
import { cn } from "@/lib/utils";

/**
 * The small text action on a rendered block: Copy on a code block, Expand or
 * Zoom on a diagram. In a block header it sits inline; `floating` pins it to
 * the block's corner and shows it on hover or focus (math, Mermaid, visuals).
 * The look lives in `.code-block-copy` and `.rich-block-copy` in index.css,
 * which the Markdown renderer shares.
 */
export const BlockAction = React.forwardRef<
  HTMLButtonElement,
  Omit<React.ComponentProps<"button">, "type"> & { floating?: boolean }
>(function BlockAction({ floating = false, className, ...props }, ref) {
  return (
    <button
      ref={ref}
      type="button"
      data-slot="block-action"
      className={cn(
        floating ? "rich-block-copy" : "code-block-copy",
        "disabled:cursor-not-allowed disabled:opacity-40",
        className,
      )}
      {...props}
    />
  );
});
