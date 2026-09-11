import { useMemo, useState } from "react";
import { cn } from "@/lib/utils";
import { harnessLabel } from "../utils";
import type { UsageResolution } from "../types";
import { HarnessMark, harnessChartDot } from "./harnessMarks";
import { formatPeriodLabel, formatTokens, formatUsd, type PeriodReport, type UsageMetric } from "../usageReport";

// A compact calendar of the window, GitHub-style: small square cells, one
// column per week and one row per weekday (Monday at the top), left-aligned
// so it never stretches. The 24-hour window is one row of hours. Each cell is
// coloured by the harness that did most of that period's work and shaded by
// the period's share of the window's peak, so colour says who and depth says
// how much. The area chart above keeps its own scale and geometry.

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
  // Column 0 is the week holding the first day; earlier days of that week are
  // empty cells, so the calendar keeps its true shape.
  const first = periods[0] ? weekdayIndex(periods[0].period) : 0;
  const cells = periods.map((period, index) => cell(period, Math.floor((index + first) / 7), weekdayIndex(period.period)));
  return { cells, columns: cells.length ? cells[cells.length - 1].column + 1 : 0, rows: 7 };
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
  // Month labels above the column where each month starts.
  const monthStarts = useMemo(() => {
    const starts = new Map<number, string>();
    let last = "";
    for (const cell of cells) {
      const month = cell.period.slice(0, 7);
      if (month !== last && !starts.has(cell.column)) { starts.set(cell.column, new Date(`${cell.period.slice(0, 10)}T00:00:00Z`).toLocaleString("en-US", { month: "short", timeZone: "UTC" })); last = month; }
    }
    return starts;
  }, [cells]);

  return <figure className={cn("min-w-0", className)}>
    <div className="flex items-start gap-2 overflow-x-auto">
      {!hourly && <div className="mt-[1.1rem] grid shrink-0 gap-[2px] text-[10px] leading-none text-muted-foreground" style={{ gridTemplateRows: `repeat(7, 0.75rem)` }} aria-hidden="true">
        {WEEKDAYS.map((label, index) => <span key={label} className="flex items-center">{index % 2 === 0 ? label : ""}</span>)}
      </div>}
      <div className="w-fit">
        <div className="mb-1 grid gap-[2px] text-[10px] leading-none text-muted-foreground" style={{ gridTemplateColumns: `repeat(${columns}, 0.75rem)`, height: "0.75rem" }} aria-hidden="true">
          {Array.from({ length: columns }, (_, column) => <span key={column} className="whitespace-nowrap">{hourly ? (column % 6 === 0 ? formatPeriodLabel(cells[column].period, resolution, timeZone).replace(/\s?[AP]M$/, match => match.trim().toLowerCase()) : "") : monthStarts.get(column) ?? ""}</span>)}
        </div>
        <div
          role="grid"
          aria-label={`${hourly ? "Hourly" : "Daily"} ${metric === "cost" ? "cost" : "processed tokens"} calendar`}
          className="relative grid gap-[2px]"
          style={{ gridTemplateColumns: `repeat(${columns}, 0.75rem)`, gridTemplateRows: `repeat(${rows}, 0.75rem)` }}
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
              className={cn("size-3 rounded-[3px] transition-opacity motion-safe:animate-[insight-rise_300ms_ease-out_both]", step === 0 ? "bg-muted/60" : cn(harnessChartDot(cell.harness), DEPTH[step]), hover && hover.period !== cell.period && step > 0 && "opacity-40")}
              style={{ gridColumnStart: cell.column + 1, gridRowStart: cell.row + 1, animationDelay: `${Math.min(cell.column, 20) * 15}ms` }}
            />;
          })}
          {hover && <div role="tooltip" className={cn("u-glass-popover pointer-events-none absolute z-10 whitespace-nowrap rounded-lg px-2.5 py-1.5 text-[11px] leading-4 tabular-nums", hover.column / Math.max(1, columns - 1) > 0.6 ? "-translate-x-full" : "", "top-full mt-1.5")} style={{ left: `${((hover.column + (hover.column / Math.max(1, columns - 1) > 0.6 ? 1 : 0)) / Math.max(1, columns)) * 100}%` }}>
            <div className="text-foreground">{format(hover.value)}{hover.harness && <span className="text-muted-foreground"> · mostly {harnessLabel(hover.harness)}</span>}</div>
            <div className="text-muted-foreground">{formatPeriodLabel(hover.period, resolution, timeZone)}</div>
          </div>}
        </div>
      </div>
    </div>
    <figcaption className="mt-3 flex flex-wrap items-center gap-x-4 gap-y-1 text-[11px] text-muted-foreground">
      <span className="inline-flex flex-wrap items-center gap-3">
        {harnesses.map(harness => <span key={harness} className="inline-flex items-center gap-1.5"><span className={cn("size-2.5 rounded-[3px]", harnessChartDot(harness))} aria-hidden="true" /><HarnessMark harness={harness} size={11} />{harnessLabel(harness)}</span>)}
        {harnesses.length === 0 && <span>No activity in this window.</span>}
      </span>
      <span className="ml-auto inline-flex items-center gap-1" aria-label={`Depth from none to ${format(peak)}`}>
        <span>less</span>
        <span className="size-2.5 rounded-[3px] bg-muted/50" aria-hidden="true" />
        {DEPTH.slice(1).map(depth => <span key={depth} className={cn("size-2.5 rounded-[3px] bg-foreground", depth)} aria-hidden="true" />)}
        <span>more</span>
      </span>
    </figcaption>
  </figure>;
}
