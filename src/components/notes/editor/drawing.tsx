// A ```excalidraw fence in a page: the drawing, drawn in place, and
// Excalidraw full screen to change it. People and agents edit the same scene;
// agents through `notes_draw`, which the core describes back to them.

import { lazy, Suspense, useMemo, useState } from "react";
import { PenLine } from "lucide-react";
import { useDarkTheme } from "../../Markdown";
import { DRAWING_FENCE, parseScene } from "./drawingScene";

const DrawingPreview = lazy(() => import("./drawingCanvas").then(module => ({ default: module.DrawingPreview })));
const DrawingEditor = lazy(() => import("./drawingCanvas").then(module => ({ default: module.DrawingEditor })));

export const isDrawingFence = (lang: string) => lang.trim().toLowerCase() === DRAWING_FENCE;

/** Whether a body reads as a drawing; one that does not shows as code. */
export const readsAsDrawing = (body: string) => parseScene(body) !== null;

export function DrawingBlock({ body, title, onSave }: { body: string; title?: string; onSave: (body: string) => void }) {
  const scene = useMemo(() => parseScene(body), [body]);
  const dark = useDarkTheme();
  const [editing, setEditing] = useState(false);
  if (!scene) return null;
  const count = scene.elements.filter(element => !element.isDeleted && !(element.type === "text" && element.containerId)).length;
  const open = (event: React.MouseEvent) => {
    event.preventDefault();
    event.stopPropagation();
    setEditing(true);
  };
  return <figure contentEditable={false} data-drawing="" className="group/drawing relative m-0 overflow-hidden rounded-lg border border-border bg-card">
    <button type="button" aria-label={count ? "Edit the drawing" : "Start drawing"} onMouseDown={open} className="block w-full cursor-pointer text-left">
      {count
        ? <Suspense fallback={<div className="h-40" />}><DrawingPreview scene={scene} dark={dark} /></Suspense>
        : <span className="flex h-40 flex-col items-center justify-center gap-2 text-ui text-muted-foreground"><PenLine size={18} aria-hidden="true" />Empty drawing. Click to draw.</span>}
    </button>
    {count > 0 && <span className="pointer-events-none absolute right-2 top-2 inline-flex h-7 items-center gap-1.5 rounded-full bg-muted px-2.5 text-[12px] text-muted-foreground opacity-0 transition-opacity group-hover/drawing:opacity-100">
      <PenLine size={13} aria-hidden="true" />Edit drawing
    </span>}
    {editing && <Suspense fallback={null}>
      <DrawingEditor body={body} dark={dark} title={title} onSave={onSave} onClose={() => setEditing(false)} />
    </Suspense>}
  </figure>;
}
