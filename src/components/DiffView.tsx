import { useEffect, useMemo, useState } from "react";
import { ChevronDown } from "lucide-react";
import { cn } from "@/lib/utils";
import { COLORIZE_DEBOUNCE_MS, colorizePatch, highlightPatch, type DiffRow, type DiffRowKind } from "./highlight";

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
 * Split rows at each hunk header, so a long patch can show its first hunk and
 * fold the rest.
 *
 * Leading `diff --git`/`index` metadata rides with the first hunk rather than
 * forming a group of its own — a fold bar reading "1 more hunk" must not be
 * counting a file header.
 */
export function splitHunks(rows: DiffRow[]): DiffRow[][] {
  const groups: DiffRow[][] = [];
  let current: DiffRow[] = [];
  for (const row of rows) {
    if (row.kind === "hunk" && current.some(candidate => candidate.kind === "hunk")) {
      groups.push(current);
      current = [];
    }
    current.push(row);
  }
  if (current.length) groups.push(current);
  return groups;
}

/**
 * A unified diff rendered in the file's own language: additions and deletions
 * keep their colour, but the code inside them is syntax-highlighted like the
 * rest of the app's code, with old/new line numbers pinned to the left.
 *
 * `foldAfterHunks` shows that many hunks and hides the rest behind a fold bar.
 * That is what lets an edit render inline by default without a 200-line patch
 * taking over the transcript — and it beats slicing the patch to a character
 * budget, which cut hunks in half and left the gutter lying about line numbers.
 */
export function PatchView({ patch, path = "", className, foldAfterHunks }: { patch: string; path?: string; className?: string; foldAfterHunks?: number }) {
  // `rows` is derived at render time, not reset by an effect: an effect only
  // runs after commit, so a naive `useEffect`-driven reset would show one
  // real paint of the *previous* patch's coloured rows under the *new*
  // patch. Comparing the cache against the current props keeps that
  // impossible — the very first render after a change already falls back to
  // the cheap, synchronous, plain-escaped structural parse.
  const [cache, setCache] = useState<{ patch: string; path: string; rows: DiffRow[] } | null>(null);
  const rows = cache && cache.patch === patch && cache.path === path ? cache.rows : highlightPatch(patch, path);

  useEffect(() => {
    let live = true;
    // Debounced: see `COLORIZE_DEBOUNCE_MS` — tool output can arrive in
    // growing chunks, and a still-growing patch shouldn't schedule a
    // tokenization pass for every intermediate length.
    const timer = window.setTimeout(() => {
      void colorizePatch(patch, path).then(colored => { if (live) setCache({ patch, path, rows: colored }); });
    }, COLORIZE_DEBOUNCE_MS);
    return () => { live = false; window.clearTimeout(timer); };
  }, [patch, path]);
  // Fragments (tool output, patches with no @@ header) have nothing to number,
  // and an empty gutter is just wasted width.
  const numbered = useMemo(() => rows.some(row => row.oldLine !== null || row.newLine !== null), [rows]);
  const hunks = useMemo(() => splitHunks(rows), [rows]);
  // Folding is reset by the patch changing, not by an effect: a growing patch
  // must not silently re-collapse what the reader just expanded, and a different
  // patch must not inherit the previous one's expansion.
  const [unfoldedFor, setUnfoldedFor] = useState<string | null>(null);
  const unfolded = unfoldedFor === patch;
  const fold = foldAfterHunks !== undefined && !unfolded && hunks.length > foldAfterHunks ? foldAfterHunks : null;
  const shown = fold === null ? rows : hunks.slice(0, fold).flat();
  const hiddenHunks = fold === null ? 0 : hunks.length - fold;
  if (!rows.length) return null;
  // A column, so a caller's `max-h-*` bounds the rows and leaves the fold bar
  // pinned below them rather than scrolling away with the code.
  return <div className={cn("stx flex flex-col overflow-hidden font-mono text-[11.5px] leading-[1.6]", className)}>
    <div className="min-h-0 flex-1 overflow-auto py-2">
      <div className="w-max min-w-full">
        {shown.map((row, index) => <DiffLine key={index} row={row} numbered={numbered} />)}
      </div>
    </div>
    {hiddenHunks > 0 && <button
      type="button"
      onClick={() => setUnfoldedFor(patch)}
      className="flex w-full items-center gap-2 bg-code px-3 py-1 text-left text-[10.5px] text-muted-foreground/70 transition-colors hover:text-muted-foreground"
    >
      <ChevronDown className="h-3 w-3 shrink-0" aria-hidden="true" />
      {hiddenHunks} more hunk{hiddenHunks === 1 ? "" : "s"} — expand
    </button>}
  </div>;
}
