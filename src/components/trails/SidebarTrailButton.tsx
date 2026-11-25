import { Route } from "lucide-react";
import type { TrailSummary } from "@/protocol/generated/protocol";
import { TrailPeek } from "./TrailPeek";
import { Button } from "@/components/ui/button";

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
      trigger={<Button variant="quiet" size="icon-xs" className="hidden shrink-0"
       
        onClick={event => { event.stopPropagation(); onOpenTrail(trail.id); }}
        aria-label={`Trail: ${trail.title}`}
        data-sidebar-trail={trail.id}
       
      >
        <Route size={13} strokeWidth={1.7} aria-hidden="true" />
      </Button>}
    />
  );
}
