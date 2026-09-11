import { ArrowUp, GripVertical, LayoutGrid, Maximize2, SquareArrowOutUpRight } from "lucide-react";
import TranscriptEntry from "./Transcript";
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

/*
 * Mission Control: every active chat at once, each tile the real conversation with its own
 * composer. `step` reveals tiles in order, then streams each tile's entries, so the grid
 * fills the way it does when a fleet spins up.
 */
export default function MissionGrid({ tiles, step }: { tiles: Tile[]; step: number }) {
  const visible = Math.min(tiles.length, Math.max(1, step));
  const streamed = Math.max(0, step - tiles.length);

  return (
    <section className="flex min-h-0 min-w-0 flex-col">
      <div className="flex h-11 shrink-0 items-center gap-2 border-b border-border px-4 sm:px-6">
        <LayoutGrid size={15} className="shrink-0 text-muted-foreground" aria-hidden="true" />
        <h3 className="text-[13px] font-semibold leading-4 text-foreground">Mission Control</h3>
        <span className="rounded bg-muted px-1.5 py-0.5 font-mono text-[10.5px] text-foreground">{visible} live</span>
        <span className="ml-auto inline-flex h-7 items-center rounded-md border border-border bg-card px-2 text-[12px] text-muted-foreground">Show all</span>
      </div>

      <div className="grid min-h-0 flex-1 grid-cols-2 gap-2 overflow-hidden p-2 max-sm:grid-cols-1">
        {tiles.slice(0, visible).map((tile, index) => (
          <article
            key={tile.title}
            className={`flex min-h-0 min-w-0 animate-entry-in flex-col overflow-hidden rounded-md border bg-background motion-reduce:animate-none ${
              index === 0 ? "border-ring/65" : "border-border"
            }`}
          >
            <header className={`flex h-9 shrink-0 items-center gap-1.5 border-b border-border px-1.5 ${index === 0 ? "bg-accent" : "bg-background"}`}>
              <GripVertical size={12} className="shrink-0 text-muted-foreground/70" aria-hidden="true" />
              <span className={`size-1.5 shrink-0 rounded-full ${dot[tile.tone]} ${tile.tone === "success" ? "motion-safe:animate-pulse" : ""}`} aria-hidden="true" />
              <span className="min-w-0 flex-1 truncate text-xs font-medium text-foreground">{tile.title}</span>
              <span className={`shrink-0 text-[10px] uppercase tracking-[0.06em] ${ink[tile.tone]}`}>{tile.status}</span>
              <span className="hidden shrink-0 items-center gap-1 text-[11px] text-muted-foreground lg:inline-flex">
                <HarnessMark harness={tile.harness} size={12} />
                {harnessLabel[tile.harness]} · {tile.repo}
              </span>
              <span className="shrink-0 text-[11px] tabular-nums text-faint">{tile.elapsed}</span>
              <SquareArrowOutUpRight size={12} className="shrink-0 text-muted-foreground" aria-hidden="true" />
              <Maximize2 size={12} className="shrink-0 text-muted-foreground" aria-hidden="true" />
            </header>

            <div className="flex min-h-0 flex-1 flex-col justify-end gap-2.5 overflow-hidden px-3 py-2.5">
              {tile.lines.slice(0, Math.max(1, Math.min(tile.lines.length, streamed - index + 1))).map((entry, i) => (
                <div key={i} className="animate-entry-in motion-reduce:animate-none">
                  <TranscriptEntry entry={entry} />
                </div>
              ))}
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
        ))}
      </div>
    </section>
  );
}
