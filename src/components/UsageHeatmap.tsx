import { useMemo, useState } from "react";
import { cn } from "@/lib/utils";
import { harnessLabel } from "../utils";
import type { UsageResolution } from "../types";
import { HarnessMark, harnessChartDot } from "./harnessMarks";
import { formatPeriodLabel, formatTokens, formatUsd, type PeriodReport, type UsageMetric } from "../usageReport";

// A calendar of the window. Day windows lay out as real weeks: seven columns
// Monday to Sunday, one row per week, so the grid fills the card at any width
// and a Saturday always sits under a Saturday. The 24-hour window is one row
// of hours. Each cell is coloured by the harness that did most of that
// period's work and shaded by the period's share of the window's peak, so
// colour says who and depth says how much. The area chart above keeps its own
// scale and geometry; this borrows nothing from it.

const WEEKDAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
/** Depth steps by share of peak. Step 0 is an empty cell. */
const DEPTH = ["", "opacity-25", "opacity-45", "opacity-70", "opacity-100"];

export function heatStep(value: number, peak: number): number {
  if (value <= 0 || peak <= 0) return 0;
  const share = value / peak;
  if (share <= 0.25) return 1;
  if (share <= 0.5) return 2;
  if (share <= 0.75) return 3;
  return 4;
}

/** Monday-first weekday for a `YYYY-MM-DD` day, parsed as UTC so the local
 *  zone cannot move it across midnight. */
export function weekdayIndex(day: string): number {
  return (new Date(`${day}T00:00:00Z`).getUTCDay() + 6) % 7;
}

/** The harness that carried most of a period, or null when nothing did. */
export function dominantHarness(period: PeriodReport, metric: UsageMetric): string | null {
  const shares = metric === "cost" ? period.costByHarness : period.tokensByHarness;
  let best: string | null = null;
  let most = 0;
  for (const [harness, value] of Object.entries(shares)) {
    if (value > most) { most = value; best = harness; }
  }
  return best;
}

export interface HeatCell { period: string; value: number; harness: string | null; column: number; row: number }

export function layoutCells(periods: PeriodReport[], resolution: UsageResolution, metric: UsageMetric): { cells: HeatCell[]; columns: number; rows: number } {
  const value = (period: PeriodReport) => (metric === "cost" ? period.costMicrousd : period.tokens);
  const cell = (period: PeriodReport, column: number, row: number): HeatCell => ({ period: period.period, value: value(period), harness: dominantHarness(period, metric), column, row });
  if (resolution === "hour") {
    return { cells: periods.map((period, index) => cell(period, index, 0)), columns: periods.length, rows: 1 };
  }
  // Row 0 is the week holding the first day; earlier days of that week are
  // empty cells, so the calendar keeps its true shape.
  const first = periods[0] ? weekdayIndex(periods[0].period) : 0;
  const cells = periods.map((period, index) => cell(period, weekdayIndex(period.period), Math.floor((index + first) / 7)));
  return { cells, columns: 7, rows: cells.length ? cells[cells.length - 1].row + 1 : 0 };
}

export function UsageHeatmap({ periods, resolution, timeZone, metric, className }: {
  periods: PeriodReport[];
  resolution: UsageResolution;
  timeZone: string;
  metric: UsageMetric;
  className?: string;
}) {
  const [hover, setHover] = useState<HeatCell | null>(null);
  const format = metric === "cost" ? formatUsd : formatTokens;
  const { cells, columns, rows } = useMemo(() => layoutCells(periods, resolution, metric), [periods, resolution, metric]);
  const peak = useMemo(() => Math.max(0, ...cells.map(cell => cell.value)), [cells]);
  const harnesses = useMemo(() => [...new Set(cells.map(cell => cell.harness).filter((harness): harness is string => harness != null))], [cells]);
  const hourly = resolution === "hour";
  // The label at the left of each week row: the date its first cell holds.
  const rowStarts = useMemo(() => {
    const starts = new Map<number, string>();
    for (const cell of cells) if (!starts.has(cell.row)) starts.set(cell.row, cell.period);
    return starts;
  }, [cells]);
  const cellHeight = hourly ? "h-8" : rows > 8 ? "h-5" : "h-7";

  return <figure className={cn("min-w-0", className)}>
    <div className={cn("grid gap-x-2", hourly ? "grid-cols-1" : "grid-cols-[3rem_minmax(0,1fr)]")}>
      {!hourly && <span aria-hidden="true" />}
      <div className="mb-1.5 grid gap-[3px] text-[10px] text-muted-foreground" style={{ gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))` }} aria-hidden="true">
        {hourly
          ? cells.map((cell, index) => <span key={cell.period} className="truncate text-center tabular-nums">{index % 3 === 0 ? formatPeriodLabel(cell.period, resolution, timeZone).replace(/\s?[AP]M$/, match => match.trim().toLowerCase()) : ""}</span>)
          : WEEKDAYS.map(label => <span key={label} className="text-center">{label}</span>)}
      </div>
      {!hourly && <div className="grid gap-[3px] text-[10px] tabular-nums text-muted-foreground" style={{ gridTemplateRows: `repeat(${rows}, minmax(0, 1fr))` }} aria-hidden="true">
        {Array.from({ length: rows }, (_, row) => <span key={row} className="flex items-center">{rowStarts.get(row)?.slice(5).replace("-", "/") ?? ""}</span>)}
      </div>}
      <div
        role="grid"
        aria-label={`${hourly ? "Hourly" : "Daily"} ${metric === "cost" ? "cost" : "processed tokens"} calendar`}
        className="relative grid gap-[3px]"
        style={{ gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))`, gridTemplateRows: `repeat(${rows}, auto)` }}
        onMouseLeave={() => setHover(null)}
      >
        {cells.map(cell => {
          const step = heatStep(cell.value, peak);
          return <div
            key={cell.period}
            role="gridcell"
            aria-label={`${formatPeriodLabel(cell.period, resolution, timeZone)}: ${format(cell.value)}${cell.harness ? `, mostly ${harnessLabel(cell.harness)}` : ""}`}
            data-step={step}
            data-harness={cell.harness ?? undefined}
            onMouseEnter={() => setHover(cell)}
            className={cn(cellHeight, "rounded-[4px] transition-opacity motion-safe:animate-[insight-rise_300ms_ease-out_both]", step === 0 ? "bg-muted/50" : cn(harnessChartDot(cell.harness), DEPTH[step]), hover && hover.period !== cell.period && step > 0 && "opacity-40")}
            style={{ gridColumnStart: cell.column + 1, gridRowStart: cell.row + 1, animationDelay: `${Math.min(cell.row * 7 + cell.column, 30) * 12}ms` }}
          />;
        })}
        {hover && <div role="tooltip" className={cn("u-glass-popover pointer-events-none absolute z-10 whitespace-nowrap rounded-lg px-2.5 py-1.5 text-[11px] leading-4 tabular-nums", hover.column / Math.max(1, columns - 1) > 0.6 ? "-translate-x-full" : "", hover.row === 0 ? "top-full mt-1" : "-top-1 -translate-y-full")} style={{ left: `${((hover.column + (hover.column / Math.max(1, columns - 1) > 0.6 ? 1 : 0)) / Math.max(1, columns)) * 100}%` }}>
          <div className="text-foreground">{format(hover.value)}{hover.harness && <span className="text-muted-foreground"> · mostly {harnessLabel(hover.harness)}</span>}</div>
          <div className="text-muted-foreground">{formatPeriodLabel(hover.period, resolution, timeZone)}</div>
        </div>}
      </div>
    </div>
    <figcaption className="mt-3 flex flex-wrap items-center justify-between gap-x-4 gap-y-1 text-[11px] text-muted-foreground">
      <span className="inline-flex flex-wrap items-center gap-3">
        {harnesses.map(harness => <span key={harness} className="inline-flex items-center gap-1.5"><span className={cn("size-2.5 rounded-[3px]", harnessChartDot(harness))} aria-hidden="true" /><HarnessMark harness={harness} size={11} />{harnessLabel(harness)}</span>)}
        {harnesses.length === 0 && <span>No activity in this window.</span>}
      </span>
      <span className="inline-flex items-center gap-1" aria-label={`Depth from none to ${format(peak)}`}>
        <span>less</span>
        <span className="size-2.5 rounded-[3px] bg-muted/50" aria-hidden="true" />
        {DEPTH.slice(1).map(depth => <span key={depth} className={cn("size-2.5 rounded-[3px] bg-foreground", depth)} aria-hidden="true" />)}
        <span>more</span>
      </span>
    </figcaption>
  </figure>;
}
