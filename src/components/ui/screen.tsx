import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

/** Shared page rhythm for desktop destinations. Toolbars stay outside the
 * scroll region; a title describes the destination, not a second navigation. */
export const SCREEN_CONTENT = "mx-auto w-full max-w-page px-5 pb-12 pt-6 sm:px-8";

export function ScreenHeading({ title, description, action, id, className }: {
  title: string;
  description?: ReactNode;
  action?: ReactNode;
  id?: string;
  className?: string;
}) {
  // The title block keeps a real basis: with `flex-1` its basis is zero, so a
  // crowded header never wraps and the actions squeeze the title to nothing
  // instead. Past the basis the actions drop under the title, and wrap among
  // themselves when even a full row cannot hold them.
  return <header className={cn("mb-5 flex flex-wrap items-center gap-3", className)}>
    <div className={cn("min-w-0 grow", description ? "basis-80" : "basis-40")}>
      <h1 id={id} className="font-display text-title font-semibold tracking-tight text-foreground">{title}</h1>
      {description && <p className="mt-1 text-ui leading-relaxed text-muted-foreground">{description}</p>}
    </div>
    {action && <div className="flex max-w-full min-w-0 flex-wrap items-center gap-2">{action}</div>}
  </header>;
}
