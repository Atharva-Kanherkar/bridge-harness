import { Route } from "lucide-react";
import { ScreenHeading } from "@/components/ui/screen";
import { Skeleton, SkeletonRegion } from "@/components/ui/skeleton";
import { clsx as cn } from "clsx";
import { TrailScroll } from "./TrailParts";

// Placeholders shaped like the Trails screens, from the skeleton kit. They
// load with App (not with the lazy Trails chunk) so the Suspense fallback can
// use them too.

const TITLE_WIDTHS = ["w-3/5", "w-2/5", "w-1/2", "w-2/3"] as const;
const META_WIDTHS = ["w-1/3", "w-1/4", "w-2/5", "w-1/5"] as const;

/** Rows shaped like a trail list row: title and last event, marks, stage, state. */
export function TrailListSkeleton({ rows = 4 }: { rows?: number }) {
  return (
    <SkeletonRegion label="Loading trails" className="flex flex-col">
      {Array.from({ length: rows }, (_, row) => (
        <div key={row} data-skeleton-row="" className="flex items-center gap-4 px-4 py-3">
          <span className="flex min-w-0 flex-1 flex-col gap-2">
            <Skeleton index={row} className={TITLE_WIDTHS[row % TITLE_WIDTHS.length]} />
            <Skeleton index={row} className={cn("h-2.5", META_WIDTHS[row % META_WIDTHS.length])} />
          </span>
          <Skeleton index={row} className="hidden size-3.5 rounded-full sm:block" />
          <span className="hidden w-36 flex-col gap-1.5 md:flex">
            <Skeleton index={row} className="h-1.5 w-[5.75rem] rounded-full" />
            <Skeleton index={row} className="h-2.5 w-12" />
          </span>
          <Skeleton index={row} className="h-2.5 w-16" />
        </div>
      ))}
    </SkeletonRegion>
  );
}

/** A day label and event rows shaped like the timeline: dot, title, meta line, time. */
export function TrailTimelineSkeleton({ rows = 5 }: { rows?: number }) {
  return (
    <SkeletonRegion label="Loading trail" className="flex flex-col">
      <Skeleton className="mb-4 ml-10 h-2.5 w-28" />
      {Array.from({ length: rows }, (_, row) => (
        <div key={row} data-skeleton-row="" className="flex gap-3 pb-6">
          <Skeleton index={row} className="size-7 shrink-0 rounded-full" />
          <span className="flex min-w-0 flex-1 flex-col gap-2 pt-1.5">
            <span className="flex items-center gap-3">
              <Skeleton index={row} className={cn("h-3", TITLE_WIDTHS[row % TITLE_WIDTHS.length])} />
              <Skeleton index={row} className="ml-auto h-2.5 w-9 shrink-0" />
              <span className="size-5 shrink-0" />
            </span>
            <Skeleton index={row} className={cn("h-2.5", META_WIDTHS[row % META_WIDTHS.length])} />
          </span>
        </div>
      ))}
    </SkeletonRegion>
  );
}

/** Lines shaped like the brief's items: a glyph and a label. */
export function TrailBriefSkeleton({ rows = 3 }: { rows?: number }) {
  return (
    <SkeletonRegion label="Loading the brief" className="flex flex-col gap-3">
      {Array.from({ length: rows }, (_, row) => (
        <span key={row} className="flex items-center gap-2">
          <Skeleton index={row} className="size-3 shrink-0 rounded" />
          <Skeleton index={row} className={TITLE_WIDTHS[(row + 1) % TITLE_WIDTHS.length]} />
        </span>
      ))}
    </SkeletonRegion>
  );
}

/** The rail's cards before the detail arrives. */
export function TrailRailSkeleton() {
  return (
    <div aria-hidden="true" className="space-y-3">
      <div className="rounded-xl bg-card p-4">
        <Skeleton className="mb-4 h-2.5 w-32" />
        <span className="flex flex-col gap-3">
          {[0, 1, 2].map(row => (
            <span key={row} className="flex items-center gap-2">
              <Skeleton index={row} className="size-3 shrink-0 rounded" />
              <Skeleton index={row} className={TITLE_WIDTHS[(row + 1) % TITLE_WIDTHS.length]} />
            </span>
          ))}
        </span>
      </div>
      <div className="rounded-xl bg-card p-4">
        <Skeleton className="mb-4 h-2.5 w-12" />
        <Skeleton index={1} className="w-1/2" />
      </div>
    </div>
  );
}

/** The trail page's header before anything about the trail is known. */
export function TrailHeaderSkeleton() {
  return (
    <span aria-hidden="true" className="flex flex-col gap-3">
      <Skeleton className="h-6 w-2/5" />
      <span className="flex gap-2">
        <Skeleton className="h-6 w-20 rounded-md" />
        <Skeleton className="h-6 w-28 rounded-md" />
        <Skeleton className="h-6 w-16 rounded-md" />
      </span>
    </span>
  );
}

/** What Trails shows while its code loads: the list, or a trail's page. */
export function TrailsScreenSkeleton({ trailOpen }: { trailOpen: boolean }) {
  if (!trailOpen) {
    return (
      <TrailScroll title="Trails">
        <div className="mx-auto w-full max-w-page px-5 pb-12 pt-6 sm:px-8">
          <ScreenHeading title="Trails" />
          <TrailListSkeleton />
        </div>
      </TrailScroll>
    );
  }
  return (
    <TrailScroll wide title="Trails">
      <div className="mx-auto w-full max-w-page-wide px-5 pb-12 pt-6 sm:px-8">
        <p className="mb-2 flex items-center gap-1 text-caption text-muted-foreground"><Route className="size-3.5" aria-hidden="true" />Trails</p>
        <div className="mb-6"><TrailHeaderSkeleton /></div>
        <div className="flex flex-col gap-8 lg:flex-row">
          <div className="min-w-0 flex-1"><TrailTimelineSkeleton /></div>
          <div className="w-full shrink-0 lg:w-72"><TrailRailSkeleton /></div>
        </div>
      </div>
    </TrailScroll>
  );
}
