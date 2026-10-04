import { ArrowUp, GripVertical, LayoutGrid, Maximize2, SquareArrowOutUpRight } from "lucide-react";
import TranscriptEntry, { entryTicks } from "./Transcript";
import HarnessMark from "./HarnessMark";
import { harnessLabel, type Tile, type Tone } from "../../content/appScenes";

const dot: Record<Tone, string> = {
  success: "bg-success",
  warning: "bg-warning",
  info: "bg-info",
  destructive: "bg-destructive",
  faint: "bg-faint",
};

const ink: Record<Tone, string> = {
  success: "text-success",
  warning: "text-warning",
  info: "text-info",
  destructive: "text-destructive",
  faint: "text-muted-foreground",
};

const GAP = 5;

/** When each line of a tile starts, staggered per tile so the grid never moves in lockstep. */
function schedule(tile: Tile, index: number) {
  let at = index * 7;
  return tile.lines.map(line => {
    const start = at;
    at += entryTicks(line) + GAP;
    return { start, ticks: entryTicks(line) };
  });
}

/** Ticks until every tile has finished streaming. */
export function missionTicks(tiles: Tile[]) {
  return Math.max(0, ...tiles.map((tile, i) => {
    const last = schedule(tile, i).at(-1);
    return last ? last.start + last.ticks : 0;
  }));
}

/*
 * Mission Control: every active chat at once, each tile the real conversation with its own
 * composer. All tiles stream together on their own schedules, so the grid reads as a fleet
 * working rather than a slideshow.
 */
export default function MissionGrid({ tiles, t }: { tiles: Tile[]; t: number }) {
  const visible = tiles.length;

  return (
    <section className="flex min-h-0 min-w-0 flex-col">
      <div className="flex h-11 shrink-0 items-center gap-2 border-b border-border px-4 sm:px-6">
        <LayoutGrid size={15} className="shrink-0 text-muted-foreground" aria-hidden="true" />
        <h3 className="text-[13px] font-semibold leading-4 text-foreground">Mission Control</h3>
        <span className="rounded bg-muted px-1.5 py-0.5 font-mono text-[10.5px] text-foreground">{visible} live</span>
        <span className="ml-auto inline-flex h-7 items-center rounded-md border border-border bg-card px-2 text-[12px] text-muted-foreground">Show all</span>
      </div>

      <div className="grid min-h-0 flex-1 grid-cols-2 gap-2 overflow-hidden p-2 max-sm:grid-cols-1">
        {tiles.map((tile, index) => {
          const lines = schedule(tile, index);
          const busy = lines.some(line => t >= line.start && t < line.start + line.ticks);
          return (
          <article
            key={tile.title}
            className={`flex min-h-0 min-w-0 animate-entry-in flex-col overflow-hidden rounded-md border bg-background motion-reduce:animate-none ${
              index === 0 ? "border-ring/65" : "border-border"
            }`}
          >
            <header className={`flex h-9 shrink-0 items-center gap-1.5 border-b border-border px-1.5 ${index === 0 ? "bg-accent" : "bg-background"}`}>
              <GripVertical size={12} className="shrink-0 text-muted-foreground/70" aria-hidden="true" />
              <span className={`size-1.5 shrink-0 rounded-full ${dot[tile.tone]} ${busy || tile.tone === "warning" ? "motion-safe:animate-pulse" : ""}`} aria-hidden="true" />
              <span className="min-w-0 flex-1 truncate text-xs font-medium text-foreground">{tile.title}</span>
              <span className={`shrink-0 text-[10px] uppercase tracking-[0.06em] ${ink[tile.tone]}`}>{busy && tile.tone === "success" ? "streaming" : tile.status}</span>
              <span className="hidden shrink-0 items-center gap-1 text-[11px] text-muted-foreground lg:inline-flex">
                <HarnessMark harness={tile.harness} size={12} />
                {harnessLabel[tile.harness]} · {tile.repo}
              </span>
              <span className="shrink-0 text-[11px] tabular-nums text-faint">{tile.elapsed}</span>
              <SquareArrowOutUpRight size={12} className="shrink-0 text-muted-foreground" aria-hidden="true" />
              <Maximize2 size={12} className="shrink-0 text-muted-foreground" aria-hidden="true" />
            </header>

            <div className="flex min-h-0 flex-1 flex-col justify-end gap-2.5 overflow-hidden px-3 py-2.5">
              {tile.lines.map((entry, i) =>
                t < lines[i].start ? null : (
                  <div key={i} className="animate-entry-in motion-reduce:animate-none">
                    <TranscriptEntry entry={entry} p={Math.min(1, (t - lines[i].start) / lines[i].ticks)} />
                  </div>
                ),
              )}
            </div>

            <div className="shrink-0 px-2 pb-2">
              <div className="flex items-center gap-2 rounded-lg border border-border-card bg-card px-2.5 py-1.5">
                <span className="min-w-0 flex-1 truncate text-[12px] text-muted-foreground">Steer {tile.title}</span>
                <span className="grid size-5 shrink-0 place-items-center rounded-full bg-primary text-primary-foreground">
                  <ArrowUp size={11} aria-hidden="true" />
                </span>
              </div>
            </div>
          </article>
          );
        })}
      </div>
    </section>
  );
}
