import { useEffect, useMemo, useState } from "react";
import { cn } from "@/lib/utils";
import { colorizePatch, highlightPatch, type DiffRow, type DiffRowKind } from "./highlight";

/** Row tint, marker glyph and marker colour for each kind of diff line. */
const ROW_STYLE: Record<DiffRowKind, { tint: string; marker: string; markerClass: string }> = {
  add: { tint: "bg-success/8", marker: "+", markerClass: "text-success" },
  del: { tint: "bg-destructive/8", marker: "−", markerClass: "text-destructive" },
  hunk: { tint: "bg-info/8 text-info", marker: "", markerClass: "" },
  meta: { tint: "text-muted-foreground/55", marker: "", markerClass: "" },
  context: { tint: "", marker: "", markerClass: "" },
};

function DiffLine({ row, numbered }: { row: DiffRow; numbered: boolean }) {
  const style = ROW_STYLE[row.kind];
  // The gutter is pinned so the numbers and the +/− marker survive a
  // horizontal scroll through a long line.
  return <div className="flex">
    <span className="sticky left-0 z-10 flex shrink-0 select-none bg-code">
      {numbered && <>
        <span className="w-9 px-1.5 text-right text-[10.5px] tabular-nums text-muted-foreground/40">{row.oldLine ?? ""}</span>
        <span className="w-9 px-1.5 text-right text-[10.5px] tabular-nums text-muted-foreground/40">{row.newLine ?? ""}</span>
      </>}
      <span className={cn("w-3.5 pl-1 text-left", style.markerClass)}>{style.marker}</span>
    </span>
    <span className={cn("flex-1 whitespace-pre pl-1.5 pr-3", style.tint)}>
      <span dangerouslySetInnerHTML={{ __html: row.html }} />
    </span>
  </div>;
}

/**
 * A unified diff rendered in the file's own language: additions and deletions
 * keep their colour, but the code inside them is syntax-highlighted like the
 * rest of the app's code, with old/new line numbers pinned to the left.
 */
export function PatchView({ patch, path = "", className }: { patch: string; path?: string; className?: string }) {
  // Rows lay out immediately with plain, escaped bodies; the file's grammar
  // is a dynamic import, so colour arrives a frame or two later (same shape
  // as `Markdown.tsx`'s `CodeBlock`) rather than blocking the gutter and
  // row kinds on it.
  const [rows, setRows] = useState(() => highlightPatch(patch, path));
  useEffect(() => {
    setRows(highlightPatch(patch, path));
    let live = true;
    void colorizePatch(patch, path).then(colored => { if (live) setRows(colored); });
    return () => { live = false; };
  }, [patch, path]);
  // Fragments (tool output, patches with no @@ header) have nothing to number,
  // and an empty gutter is just wasted width.
  const numbered = useMemo(() => rows.some(row => row.oldLine !== null || row.newLine !== null), [rows]);
  if (!rows.length) return null;
  return <div className={cn("stx overflow-auto py-2 font-mono text-[11.5px] leading-[1.6]", className)}>
    <div className="w-max min-w-full">
      {rows.map((row, index) => <DiffLine key={index} row={row} numbered={numbered} />)}
    </div>
  </div>;
}
