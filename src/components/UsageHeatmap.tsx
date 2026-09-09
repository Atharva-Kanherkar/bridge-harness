import { useMemo, useState } from "react";
import { cn } from "@/lib/utils";
import type { UsageResolution } from "../types";
import { formatPeriodLabel, formatTokens, formatUsd, type PeriodReport, type UsageMetric } from "../usageReport";

// A calendar heatmap of the window: one cell per period, ink darkening with
// the metric. Sequential, one hue (the page's ink), five steps from empty to
// peak; identity comes from position, so no legend swatches beyond the ramp.
// Day windows lay out as weeks (rows Monday to Sunday, one column per week);
// the 24-hour window is one row of hours. Sits beside the area chart and
// borrows nothing from it: that chart keeps its own scale and geometry.

const STEPS = ["bg-muted/50", "bg-foreground/20", "bg-foreground/40", "bg-foreground/65", "bg-foreground/90"];
const WEEKDAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/** Which of the five steps a value falls in, against the window's peak. */
export function heatStep(value: number, peak: number): number {
  if (value <= 0 || peak <= 0) return 0;
  const share = value / peak;
  if (share <= 0.25) return 1;
  if (share <= 0.5) return 2;
  if (share <= 0.75) return 3;
  return 4;
}

/** Monday-first weekday index for a `YYYY-MM-DD` day, parsed as UTC so the
 *  local zone cannot move it across midnight. */
export function weekdayIndex(day: string): number {
  const date = new Date(`${day}T00:00:00Z`);
  return (date.getUTCDay() + 6) % 7;
}

interface Cell { period: string; value: number; column: number; row: number }

export function layoutCells(periods: PeriodReport[], resolution: UsageResolution, metric: UsageMetric): { cells: Cell[]; columns: number; rows: number } {
  const value = (period: PeriodReport) => (metric === "cost" ? period.costMicrousd : period.tokens);
  if (resolution === "hour") {
    return { cells: periods.map((period, index) => ({ period: period.period, value: value(period), column: index, row: 0 })), columns: periods.length, rows: 1 };
  }
  // Column 0 holds the week containing the first day; days before it in that
  // week are simply empty cells.
  const first = periods[0] ? weekdayIndex(periods[0].period) : 0;
  const cells = periods.map((period, index) => ({ period: period.period, value: value(period), column: Math.floor((index + first) / 7), row: weekdayIndex(period.period) }));
  return { cells, columns: cells.length ? cells[cells.length - 1].column + 1 : 0, rows: 7 };
}

export function UsageHeatmap({ periods, resolution, timeZone, metric, className }: {
  periods: PeriodReport[];
  resolution: UsageResolution;
  timeZone: string;
  metric: UsageMetric;
  className?: string;
}) {
  const [hover, setHover] = useState<Cell | null>(null);
  const format = metric === "cost" ? formatUsd : formatTokens;
  const { cells, columns, rows } = useMemo(() => layoutCells(periods, resolution, metric), [periods, resolution, metric]);
  const peak = useMemo(() => Math.max(0, ...cells.map(cell => cell.value)), [cells]);
  const hourly = resolution === "hour";
  return <figure className={cn("min-w-0", className)}>
    <div className="flex gap-2">
      {!hourly && <div className="grid shrink-0 grid-rows-7 gap-[2px] text-[10px] leading-none text-muted-foreground" aria-hidden="true">
        {WEEKDAYS.map((label, index) => <span key={label} className="flex h-4 items-center">{index % 2 === 0 ? label : ""}</span>)}
      </div>}
      <div
        role="grid"
        aria-label={`${hourly ? "Hourly" : "Daily"} ${metric === "cost" ? "cost" : "processed tokens"} heatmap`}
        className="relative grid w-fit max-w-full gap-[2px] overflow-x-auto"
        style={{ gridTemplateRows: `repeat(${rows}, 1rem)`, gridTemplateColumns: `repeat(${columns}, 1rem)` }}
        onMouseLeave={() => setHover(null)}
      >
        {cells.map(cell => <div
          key={cell.period}
          role="gridcell"
          aria-label={`${formatPeriodLabel(cell.period, resolution, timeZone)}: ${format(cell.value)}`}
          data-step={heatStep(cell.value, peak)}
          onMouseEnter={() => setHover(cell)}
          className={cn("size-4 rounded-[4px] transition-colors motion-safe:animate-[insight-rise_300ms_ease-out_both]", STEPS[heatStep(cell.value, peak)], hover && hover.period !== cell.period && "opacity-70")}
          style={{ gridColumnStart: cell.column + 1, gridRowStart: cell.row + 1, animationDelay: `${Math.min(cell.column, 12) * 20}ms` }}
        />)}
        {hover && <div role="tooltip" className={cn("u-glass-popover pointer-events-none absolute -top-1 z-10 -translate-y-full whitespace-nowrap rounded-lg px-2.5 py-1.5 text-[11px] leading-4 tabular-nums", hover.column / Math.max(1, columns - 1) > 0.6 ? "-translate-x-full" : "")} style={{ left: `${((hover.column + (hover.column / Math.max(1, columns - 1) > 0.6 ? 1 : 0)) / Math.max(1, columns)) * 100}%` }}>
          <div className="text-foreground">{format(hover.value)}</div>
          <div className="text-muted-foreground">{formatPeriodLabel(hover.period, resolution, timeZone)}</div>
        </div>}
      </div>
    </div>
    <figcaption className="mt-2 flex items-center justify-between text-[11px] text-muted-foreground">
      <span>{hourly ? "Each cell is an hour" : "Each cell is a day, weeks left to right"}</span>
      <span className="inline-flex items-center gap-1" aria-label={`Scale from none to ${format(peak)}`}>
        <span>none</span>
        {STEPS.map(step => <span key={step} className={cn("size-2.5 rounded-[3px]", step)} aria-hidden="true" />)}
        <span className="tabular-nums">{format(peak)}</span>
      </span>
    </figcaption>
  </figure>;
}
