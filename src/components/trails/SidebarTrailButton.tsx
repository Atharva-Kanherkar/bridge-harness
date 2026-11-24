import { Route } from "lucide-react";
import type { TrailSummary } from "@/protocol/generated/protocol";
import { TrailPeek } from "./TrailPeek";

/**
 * A sidebar chat row's trail button: hidden until the row (a `group/row`) is
 * hovered or holds focus, and kept while its card is open, because the
 * pointer leaves the row on its way into the card. Hovering it shows the
 * trail; clicking it opens the trail's page.
 */
export function SidebarTrailButton({ trail, onOpenTrail }: { trail: TrailSummary; onOpenTrail: (trailId: string) => void }) {
  return (
    <TrailPeek
      trail={trail}
      onOpen={onOpenTrail}
      side="right"
      align="start"
      trigger={<button
        type="button"
        onClick={event => { event.stopPropagation(); onOpenTrail(trail.id); }}
        aria-label={`Trail: ${trail.title}`}
        data-sidebar-trail={trail.id}
        className="hidden size-7 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:grid group-hover/row:grid group-focus-within/row:grid data-popup-open:grid data-popup-open:bg-accent data-popup-open:text-foreground"
      >
        <Route size={13} strokeWidth={1.7} aria-hidden="true" />
      </button>}
    />
  );
}
