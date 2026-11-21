// What the window shows while the settings chunk loads: the rail and a page in
// the shapes they are about to take, so opening Settings does not flash a
// centered line of text and then jump into a two-column layout.

import { Skeleton, SkeletonRegion, SkeletonRows } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";
import { SETTINGS_COLUMN } from "./layout";

export function SettingsScreenSkeleton() {
  return <SkeletonRegion label="Opening settings" className="flex h-full min-h-0 flex-col md:flex-row">
    <div className="u-vibrancy-sidebar hidden w-60 shrink-0 flex-col gap-1.5 border-r border-sidebar-border bg-sidebar px-3 pt-14 md:flex">
      <Skeleton className="mb-3 h-7 w-full rounded-lg" />
      {Array.from({ length: 8 }, (_, index) => <Skeleton key={index} index={index} className="h-6 w-4/5" />)}
    </div>
    <div className="min-w-0 flex-1 md:pt-11">
      <div className={cn("space-y-6", SETTINGS_COLUMN)}>
        <div className="space-y-2"><Skeleton className="h-7 w-40" /><Skeleton index={1} className="w-72 max-w-full" /></div>
        <SkeletonRows rows={4} trailing />
      </div>
    </div>
  </SkeletonRegion>;
}
