import { useEffect, useState, type ReactElement } from "react";
import { PreviewCard } from "@base-ui/react/preview-card";
import { ArrowRight, CircleAlert, Route } from "lucide-react";
import type { TrailDetail, TrailSummary } from "@/protocol/generated/protocol";
import { bridgeApi } from "@/api";
import { latestEvents, stageLabel, whenLabel } from "@/trails";
import { HarnessMark } from "@/components/harnessMarks";
import { Skeleton, SkeletonRegion } from "@/components/ui/skeleton";
import { KIND_ICON, StageBar, StateWord, TONE_INK } from "./TrailParts";
import { clsx as cn } from "clsx";

// A trail at a glance, from wherever a chat names its trail: the sidebar row's
// trail button and the chat header's trail chip. It opens on hover or focus
// after a beat, so passing over the sidebar never flashes one.

const LATEST = 4;
const OPEN_ITEMS = 2;

// The last detail read per trail: a second look paints at once and reads
// again behind it, the way a reload keeps the last data on screen.
const lastRead = new Map<string, TrailDetail>();

/** For tests: forget what earlier peeks read. */
export function resetTrailPeekCache() {
  lastRead.clear();
}

type Read = { detail: TrailDetail | null; failed: boolean };

/** Read the trail's stored detail (no GitHub refresh) while the card is open. */
function useTrailDetail(trailId: string, open: boolean, updatedAt: string): Read {
  const [read, setRead] = useState<Read>(() => ({ detail: lastRead.get(trailId) ?? null, failed: false }));
  useEffect(() => {
    if (!open) return;
    let live = true;
    setRead({ detail: lastRead.get(trailId) ?? null, failed: false });
    bridgeApi.readTrail(trailId)
      .then(result => {
        lastRead.set(trailId, result.trail);
        if (live) setRead({ detail: result.trail, failed: false });
      })
      .catch(() => { if (live) setRead(current => ({ detail: current.detail, failed: true })); });
    return () => { live = false; };
  }, [trailId, open, updatedAt]);
  return read;
}

/** The card's body: a compressed trail page. Exported so it renders without the hover. */
export function TrailPeekCard({ trail, detail, failed, onOpen }: {
  trail: TrailSummary;
  detail: TrailDetail | null;
  failed: boolean;
  onOpen: () => void;
}) {
  const summary = detail?.summary.id === trail.id ? detail.summary : trail;
  const chats = summary.harnesses.reduce((sum, item) => sum + item.chats, 0);
  const latest = detail ? latestEvents(detail.events, LATEST) : [];
  const open = detail?.open.slice(0, OPEN_ITEMS) ?? [];
  const moreOpen = (detail?.open.length ?? 0) - open.length;
  return (
    <div data-trail-peek={trail.id} className="flex flex-col">
      <div className="px-3.5 pb-3 pt-3">
        <div className="flex items-center gap-1.5 text-caption text-faint">
          <Route className="size-3" aria-hidden="true" />
          <span className="flex-1">Trail</span>
          {chats > 0 && (
            <span className="flex shrink-0 items-center gap-1 text-muted-foreground">
              {summary.harnesses.map(item => <HarnessMark key={item.harness} harness={item.harness} size={11} />)}
              <span className="ml-0.5 tabular-nums">{chats} {chats === 1 ? "chat" : "chats"}</span>
            </span>
          )}
        </div>
        <p className="mt-1 line-clamp-2 text-ui font-semibold leading-snug text-foreground">{summary.title}</p>
        <div className="mt-2 flex items-center gap-2 whitespace-nowrap">
          <StateWord state={summary.state} />
          <StageBar stage={summary.stage} state={summary.state} />
          <span className="truncate text-caption text-faint">{stageLabel(summary.stage)}</span>
        </div>
        {summary.needsYouReason && (
          <p className="mt-2.5 flex items-start gap-2 rounded-lg bg-warning/10 px-2.5 py-1.5 text-caption text-foreground">
            <CircleAlert className="mt-0.5 size-3 shrink-0 text-warning" aria-hidden="true" />
            <span className="min-w-0 flex-1">{summary.needsYouReason}</span>
          </p>
        )}
      </div>

      <div className="border-t border-border px-1.5 py-1.5">
        <p className="px-2 pb-1 pt-0.5 text-caption font-medium text-faint">Latest</p>
        {!detail && !failed && (
          <SkeletonRegion label="Loading trail" className="flex flex-col">
            {[0, 1, 2].map(row => (
              <span key={row} className="flex h-7 items-center gap-2 px-2">
                <Skeleton index={row} className="size-3.5 shrink-0 rounded-full" />
                <Skeleton index={row} className={row === 1 ? "w-1/2" : "w-2/3"} />
                <Skeleton index={row} className="ml-auto h-2.5 w-8 shrink-0" />
              </span>
            ))}
          </SkeletonRegion>
        )}
        {!detail && failed && <p className="px-2 py-1.5 text-caption text-muted-foreground">Could not read this trail.</p>}
        {detail && latest.length === 0 && <p className="px-2 py-1.5 text-caption text-muted-foreground">Nothing on this trail yet.</p>}
        {latest.length > 0 && (
          <ul aria-label="Latest on this trail">
            {latest.map(event => {
              const Icon = KIND_ICON[event.kind];
              return (
                <li key={event.id} data-peek-event={event.kind} className="flex h-7 items-center gap-2 px-2">
                  <span className="flex size-3.5 shrink-0 items-center justify-center">
                    {event.harness
                      ? <HarnessMark harness={event.harness} size={12} />
                      : <Icon className={cn("size-3.5", TONE_INK[event.tone])} />}
                  </span>
                  <span className="min-w-0 flex-1 truncate text-caption text-body">{event.title}</span>
                  <time dateTime={event.at} className="shrink-0 text-micro tabular-nums text-faint">{whenLabel(event.at)}</time>
                </li>
              );
            })}
          </ul>
        )}
      </div>

      {open.length > 0 && (
        <div className="border-t border-border px-3.5 py-2.5">
          <ul aria-label="Open on this trail" className="flex flex-col gap-1.5">
            {open.map(item => (
              <li key={`${item.kind}:${item.label}`} className="flex items-center gap-2 text-caption text-body">
                <span aria-hidden="true" className={cn("size-1.5 shrink-0 rounded-full", item.kind === "failingCheck" ? "bg-destructive" : "bg-warning")} />
                <span className="min-w-0 flex-1 truncate">{item.label}</span>
              </li>
            ))}
          </ul>
          {moreOpen > 0 && <p className="mt-1.5 pl-3.5 text-caption text-faint">and {moreOpen} more</p>}
        </div>
      )}

      <div className="border-t border-border p-1.5">
        <button
          type="button"
          onClick={onOpen}
          className="flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-ui text-foreground transition-colors hover:bg-accent focus-visible:bg-accent focus-visible:outline-none"
        >
          <span className="flex-1 font-medium">Open trail</span>
          <span className="text-caption tabular-nums text-faint">{summary.memberCount} {summary.memberCount === 1 ? "item" : "items"}</span>
          <ArrowRight className="size-3.5 text-muted-foreground" aria-hidden="true" />
        </button>
      </div>
    </div>
  );
}

/**
 * Wraps a trigger (a button) so hovering or focusing it shows the trail's
 * card. The trigger keeps its own click; "Open trail" in the card calls
 * `onOpen` and closes it.
 */
export function TrailPeek({ trail, trigger, onOpen, side = "right", align = "start", delay = 350 }: {
  trail: TrailSummary;
  /** The element that opens the card: a `<button>`; it keeps its own props and click. */
  trigger: ReactElement;
  onOpen: (trailId: string) => void;
  side?: "top" | "right" | "bottom" | "left";
  align?: "start" | "center" | "end";
  delay?: number;
}) {
  const [open, setOpen] = useState(false);
  const read = useTrailDetail(trail.id, open, trail.updatedAt);
  return (
    <PreviewCard.Root open={open} onOpenChange={setOpen}>
      {/* The trigger's own click (open the trail) goes elsewhere: the card goes with it. */}
      <PreviewCard.Trigger render={trigger} delay={delay} closeDelay={150} onClick={() => setOpen(false)} />
      <PreviewCard.Portal>
        <PreviewCard.Positioner side={side} align={align} sideOffset={8} collisionPadding={8} className="z-50">
          <PreviewCard.Popup
            aria-label={`Trail ${trail.title}`}
            className="w-80 max-w-[calc(100vw-1rem)] origin-(--transform-origin) overflow-hidden rounded-xl u-glass-popover text-popover-foreground outline-none transition-[opacity,scale] duration-150 ease-out data-ending-style:scale-98 data-ending-style:opacity-0 data-starting-style:scale-98 data-starting-style:opacity-0"
          >
            <TrailPeekCard
              trail={trail}
              detail={read.detail}
              failed={read.failed}
              onOpen={() => { setOpen(false); onOpen(trail.id); }}
            />
          </PreviewCard.Popup>
        </PreviewCard.Positioner>
      </PreviewCard.Portal>
    </PreviewCard.Root>
  );
}
