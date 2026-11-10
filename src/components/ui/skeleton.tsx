import type React from "react";
import { cn } from "@/lib/utils";

// The one way Bridge draws content that has not arrived yet. Placeholders are
// shaped like what replaces them, fade in only after a beat (so a fast load
// never flashes), and carry one travelling highlight that staggers row by row.
// The look lives in `.skeleton-shimmer` and `.animate-skeleton-in` in
// `src/index.css`; reduced motion leaves static bars.
//
// Skeletons are for a first load only. An action in flight spins inside the
// control that started it, and a reload keeps the last data on screen.

// Literal classes so Tailwind generates them: 90ms per row, capped so a long
// list does not leave its last rows waiting.
const STAGGER = [
  "[animation-delay:0ms]",
  "[animation-delay:90ms]",
  "[animation-delay:180ms]",
  "[animation-delay:270ms]",
  "[animation-delay:360ms]",
  "[animation-delay:450ms]",
  "[animation-delay:540ms]",
  "[animation-delay:630ms]",
] as const;

/** The stagger delay class for the row at `index`. */
export function skeletonDelay(index = 0): string {
  return STAGGER[Math.min(Math.max(index, 0), STAGGER.length - 1)];
}

const TITLE_WIDTHS = ["w-3/5", "w-2/5", "w-1/2", "w-3/4", "w-2/5", "w-2/3"] as const;
const META_WIDTHS = ["w-1/4", "w-1/3", "w-1/5", "w-2/5", "w-1/4", "w-1/3"] as const;
const LINE_WIDTHS = ["w-full", "w-11/12", "w-10/12", "w-full"] as const;

const LEAD = {
  dot: "size-3.5 rounded-full",
  icon: "size-7 rounded-lg",
  tile: "size-9 rounded-lg",
} as const;

/** One placeholder shape. Decorative: the region around it carries the label. */
export function Skeleton({ className, index }: { className?: string; index?: number }): React.ReactElement {
  return <span aria-hidden="true" className={cn("skeleton-shimmer block h-3 rounded", skeletonDelay(index), className)} />;
}

/** The status region every skeleton sits in: named for assistive tech, busy, and faded in after a beat. */
export function SkeletonRegion({ label, className, children }: { label: string; className?: string; children: React.ReactNode }): React.ReactElement {
  // The label goes first: a `divide-y` region rules every child but the last,
  // and a clipped sr-only span is the one child whose rule nobody sees.
  return <div role="status" aria-busy="true" aria-label={label} className={cn("animate-skeleton-in", className)}>
    <span className="sr-only">{label}</span>
    {children}
  </div>;
}

/** Prose bars with a ragged last line. Not a region on its own; put it inside one. */
export function SkeletonText({ lines = 3, className, index = 0 }: { lines?: number; className?: string; index?: number }): React.ReactElement {
  return <span aria-hidden="true" className={cn("flex flex-col gap-2.5", className)}>
    {Array.from({ length: lines }, (_, line) => <Skeleton
      key={line}
      index={index}
      className={line === lines - 1 && lines > 1 ? "w-3/5" : LINE_WIDTHS[line % LINE_WIDTHS.length]}
    />)}
  </span>;
}

/**
 * Placeholder list rows. Framed, it matches a `SettingsGroup` card; unframed,
 * it drops into a popover, a pane, or an existing `SettingsGroup`.
 */
export function SkeletonRows({ label, rows = 4, lead, meta = true, trailing = false, framed = true, className, rowClassName }: {
  /** Names the wait. Omit it only inside a `SkeletonRegion` that already does. */
  label?: string;
  rows?: number;
  /** The shape before the text: a 15px status glyph, a 28px row icon, or a 36px app tile. */
  lead?: keyof typeof LEAD;
  /** A second, shorter line under the title. */
  meta?: boolean;
  /** A control-sized block at the end of the row. */
  trailing?: boolean;
  framed?: boolean;
  className?: string;
  rowClassName?: string;
}): React.ReactElement {
  const frame = cn("divide-y divide-border", framed && "overflow-hidden rounded-xl border border-border-card bg-card", className);
  const items = Array.from({ length: rows }, (_, row) => <div key={row} data-skeleton-row="" className={cn("flex min-h-11 items-center gap-3 px-3.5 py-2.5", rowClassName)}>
    {lead && <Skeleton index={row} className={cn("shrink-0", LEAD[lead])} />}
    <span className="flex min-w-0 flex-1 flex-col gap-2">
      <Skeleton index={row} className={TITLE_WIDTHS[row % TITLE_WIDTHS.length]} />
      {meta && <Skeleton index={row} className={cn("h-2.5", META_WIDTHS[row % META_WIDTHS.length])} />}
    </span>
    {trailing && <Skeleton index={row} className="h-6 w-14 shrink-0 rounded-md" />}
  </div>);
  return label === undefined
    ? <div aria-hidden="true" className={frame}>{items}</div>
    : <SkeletonRegion label={label} className={frame}>{items}</SkeletonRegion>;
}

/** Placeholder cards in a grid, for dashboards and galleries. */
export function SkeletonCards({ label, count = 3, className, cardClassName }: {
  label: string;
  count?: number;
  className?: string;
  cardClassName?: string;
}): React.ReactElement {
  return <SkeletonRegion label={label} className={cn("grid gap-4 sm:grid-cols-2 lg:grid-cols-3", className)}>
    {Array.from({ length: count }, (_, card) => <Skeleton key={card} index={card} className={cn("h-40 rounded-xl", cardClassName)} />)}
  </SkeletonRegion>;
}
