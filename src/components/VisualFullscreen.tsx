import { useState, type ReactNode } from "react";
import { Dialog, DialogPopup } from "@/components/ui/dialog";
import { Maximize2, Minimize2, Minus, Plus } from "lucide-react";

/**
 * A diagram laid out to fit a chat column is often unreadable — a wide Mermaid
 * flowchart shrinks its labels to a few pixels. The overlay gives the same
 * figure the whole window and a zoom control, without touching how the block
 * renders in the transcript.
 */

export const ZOOM_MIN = 50;
export const ZOOM_MAX = 300;
export const ZOOM_STEP = 25;
export const ZOOM_DEFAULT = 100;

/** One step on the zoom ladder, clamped so the controls never dead-end. */
export function nextZoom(current: number, direction: "in" | "out"): number {
  const delta = direction === "in" ? ZOOM_STEP : -ZOOM_STEP;
  return Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, current + delta));
}

/** The inline affordance, styled like the block's other hover actions. */
export function ExpandButton({ onClick }: { onClick: () => void }) {
  return (
    <button
      type="button"
      className="rich-block-copy"
      onClick={onClick}
      aria-label="Fullscreen"
      title="Fullscreen"
    >
      <Maximize2 size={12} aria-hidden="true" />
      Expand
    </button>
  );
}

export interface VisualFullscreenProps {
  open: boolean;
  onClose: () => void;
  /** Short lowercase kind shown in the overlay's header, e.g. `diagram`. */
  label: string;
  children: ReactNode;
}

export function VisualFullscreen({ open, onClose, label, children }: VisualFullscreenProps) {
  const [zoom, setZoom] = useState(ZOOM_DEFAULT);

  // Reopening always starts from fit-the-window, not whatever the last look
  // left behind — the reader's mental model of "open it big" does not change
  // because they once zoomed a different diagram.
  const close = () => {
    setZoom(ZOOM_DEFAULT);
    onClose();
  };

  if (!open) return null;

  return (
    <Dialog open onOpenChange={next => { if (!next) close(); }}>
      <DialogPopup
        showCloseButton={false}
        aria-label={`${label} fullscreen`}
        className="h-[86dvh] w-[calc(100vw-4rem)] max-w-[calc(100vw-4rem)] overflow-hidden"
      >
        <div className="code-block-header">
          <span className="code-block-lang">{label}</span>
          <div className="flex items-center gap-[0.15rem]">
            <button
              type="button"
              className="code-block-copy disabled:cursor-not-allowed disabled:opacity-40"
              onClick={() => setZoom(current => nextZoom(current, "out"))}
              disabled={zoom <= ZOOM_MIN}
              aria-label="Zoom out"
              title="Zoom out"
            >
              <Minus size={12} aria-hidden="true" />
            </button>
            <span className="min-w-[3.4em] text-center text-[11px] tabular-nums text-muted-foreground" aria-live="polite">
              {zoom}%
            </span>
            <button
              type="button"
              className="code-block-copy disabled:cursor-not-allowed disabled:opacity-40"
              onClick={() => setZoom(current => nextZoom(current, "in"))}
              disabled={zoom >= ZOOM_MAX}
              aria-label="Zoom in"
              title="Zoom in"
            >
              <Plus size={12} aria-hidden="true" />
            </button>
            <button
              type="button"
              className="code-block-copy"
              onClick={close}
              aria-label="Exit fullscreen"
              title="Exit fullscreen (Esc)"
            >
              <Minimize2 size={12} aria-hidden="true" />
              Close
            </button>
          </div>
        </div>
        <div className="flex-1 overflow-auto bg-background p-[1.25rem]">
          {/* Zoom is expressed as the canvas width, so the overlay's own width
              is the 100% baseline and everything past it scrolls instead of
              clipping. `visual-fullscreen-canvas` turns that width into an SVG
              width; see src/index.css. */}
          <div className="visual-fullscreen-canvas mx-auto" style={{ width: `${zoom}%` }}>
            {children}
          </div>
        </div>
      </DialogPopup>
    </Dialog>
  );
}
