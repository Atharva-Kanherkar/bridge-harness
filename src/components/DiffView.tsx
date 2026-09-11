import { useEffect, useMemo, useState } from "react";
import { ChevronDown, Quote } from "lucide-react";
import { cn } from "@/lib/utils";
import { COLORIZE_DEBOUNCE_MS, colorizePatch, highlightPatch, type DiffRow, type DiffRowKind } from "./highlight";

/** A hunk's span in the new file, read from its own @@ header. */
export type HunkRange = { start: number; end: number };

const HUNK_NEW_RANGE = /@@+ (?:-\d+(?:,\d+)? )?\+(\d+)(?:,(\d+))? @@/;

function hunkRange(row: DiffRow): HunkRange | undefined {
  const match = HUNK_NEW_RANGE.exec(row.html);
  if (!match) return undefined;
  const start = Number(match[1]);
  const count = match[2] === undefined ? 1 : Number(match[2]);
  return { start, end: start + Math.max(count, 1) - 1 };
}

/**
 * Row tint, marker glyph and marker colour for each kind of diff line.
 *
 * `edge` is what makes a run of changes read as a block: an 8% tint has no
 * boundary against the context around it, so a two-line deletion inside forty
 * lines of context used to be almost invisible. The marker column carries a
 * full-strength rule in the row's own colour instead — one continuous stripe
 * down the length of a run, which is the shape the eye actually picks up.
 */
const ROW_STYLE: Record<DiffRowKind, { tint: string; marker: string; markerClass: string; edge: string }> = {
  add: { tint: "bg-success/10", marker: "+", markerClass: "text-success", edge: "bg-success/60" },
  del: { tint: "bg-destructive/10", marker: "−", markerClass: "text-destructive", edge: "bg-destructive/60" },
  hunk: { tint: "u-diff-band text-info", marker: "", markerClass: "", edge: "bg-info/50" },
  meta: { tint: "text-muted-foreground", marker: "", markerClass: "", edge: "" },
  context: { tint: "", marker: "", markerClass: "", edge: "" },
};

function DiffLine({ row, numbered, onQuoteHunk }: { row: DiffRow; numbered: boolean; onQuoteHunk?: (range: HunkRange) => void }) {
  const style = ROW_STYLE[row.kind];
  const range = row.kind === "hunk" && onQuoteHunk ? hunkRange(row) : undefined;
  // A hunk header is a divider across the whole row, gutter included —
  // tinting only the body left the line numbers sitting in an untinted notch
  // that broke the band in half.
  const band = row.kind === "hunk";
  // The gutter stays a fixed column while the body wraps, so it has to be
  // opaque in every row kind: a translucent column lets a hunk header show
  // through. `bg-inherit` takes the row's pre-composed tint so the sticky
  // column cannot go transparent.
  const line = row.newLine ?? row.oldLine;
  return <div className={cn("group/hunk flex", band && "pt-px", style.tint || "bg-card")}>
    <span className={cn(
      "sticky left-0 z-10 flex shrink-0 select-none items-start gap-1.5 bg-inherit px-1.5",
      numbered && !band && !style.edge && "border-r border-border/60",
    )}>
      {numbered && (
        <span className="w-6 text-right text-[11px] tabular-nums text-muted-foreground/70">{line ?? ""}</span>
      )}
      <span className={cn("w-3 text-center", style.markerClass)}>
        {style.marker}
      </span>
      {style.edge && !band && <span className={cn("absolute inset-y-0 right-0 w-[2px]", style.edge)} aria-hidden="true" />}
    </span>
    <span className="min-w-0 flex-1 whitespace-pre-wrap break-words pr-3">
      <span dangerouslySetInnerHTML={{ __html: row.html }} />
      {range && <button
        type="button"
        onClick={() => onQuoteHunk?.(range)}
        aria-label={`Reference lines ${range.start}-${range.end} in the composer`}
        title="Reference this hunk in the composer"
        className="ml-2 inline-flex h-4 items-center gap-1 rounded px-1 align-middle text-[11px] text-muted-foreground opacity-0 transition-opacity hover:bg-accent hover:text-foreground focus-visible:opacity-100 group-hover/hunk:opacity-100"
      >
        <Quote size={9} strokeWidth={1.8} aria-hidden="true" />
        quote
      </button>}
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
 * rest of the app's code, with a line number pinned to the left.
 *
 * `foldAfterHunks` shows that many hunks and hides the rest behind a fold bar.
 * That is what lets an edit render inline by default without a 200-line patch
 * taking over the transcript — and it beats slicing the patch to a character
 * budget, which cut hunks in half and left the gutter lying about line numbers.
 */
export function PatchView({ patch, path = "", className, foldAfterHunks, onQuoteHunk }: { patch: string; path?: string; className?: string; foldAfterHunks?: number; onQuoteHunk?: (range: HunkRange) => void }) {
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
  return <div className={cn("stx flex flex-col overflow-hidden bg-card font-mono text-[12px] leading-[1.6]", className)}>
    <div className="min-h-0 flex-1 overflow-auto p-1">
      <div className="min-w-0 overflow-hidden rounded-sm border border-border bg-card">
        {shown.map((row, index) => <DiffLine key={index} row={row} numbered={numbered} onQuoteHunk={onQuoteHunk} />)}
      </div>
    </div>
    {hiddenHunks > 0 && <button
      type="button"
      onClick={() => setUnfoldedFor(patch)}
      className="flex w-full items-center gap-2 bg-card px-3 py-1 text-left text-[11px] text-muted-foreground transition-colors hover:text-muted-foreground"
    >
      <ChevronDown className="h-3 w-3 shrink-0" aria-hidden="true" />
      {hiddenHunks} more hunk{hiddenHunks === 1 ? "" : "s"} — expand
    </button>}
  </div>;
}
