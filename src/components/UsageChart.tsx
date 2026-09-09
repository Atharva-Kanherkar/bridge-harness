import { useMemo, useRef, useState } from "react";
import { cn } from "@/lib/utils";
import { harnessLabel } from "../utils";
import { HarnessMark } from "./harnessMarks";
import type { UsageResolution } from "../types";
import { axisLabelIndices, CHART_GEOMETRY, chartX, chartY, formatPeriodLabel, formatTokens, formatUsd, hoverIndex, niceScale, seriesPaths, type ChartSeries, type UsageMetric } from "../usageReport";

// Layered monotone areas, one per harness, each measured from zero. The
// series wear a fixed achromatic ramp by rank — the chrome stays graphite,
// and a harness's own tint is reserved for its mark in the legend.
const FILL_RAMP = ["fill-foreground/70", "fill-foreground/45", "fill-foreground/28", "fill-foreground/16", "fill-foreground/10"];
const STROKE_RAMP = ["stroke-foreground", "stroke-foreground/70", "stroke-foreground/50", "stroke-foreground/35", "stroke-foreground/25"];
const DOT_RAMP = ["bg-foreground", "bg-foreground/70", "bg-foreground/50", "bg-foreground/35", "bg-foreground/25"];
const TICK_COUNT = 4;

export function seriesDotClass(rank: number): string {
  return DOT_RAMP[Math.min(rank, DOT_RAMP.length - 1)];
}

export function UsageChart({ series, periods, resolution, timeZone, metric, className }: {
  series: ChartSeries[];
  periods: string[];
  resolution: UsageResolution;
  timeZone: string;
  metric: UsageMetric;
  className?: string;
}) {
  const plotRef = useRef<HTMLDivElement>(null);
  const [hover, setHover] = useState<number | null>(null);
  const format = metric === "tokens" ? formatTokens : formatUsd;
  const { width, height } = CHART_GEOMETRY;

  // The scale tops out at the largest single harness-period, not the sum:
  // layered series each measure from zero, so a combined peak would leave
  // the plot permanently half empty.
  const peak = useMemo(() => Math.max(0, ...series.flatMap(entry => entry.values)), [series]);
  const scale = useMemo(() => niceScale(peak, TICK_COUNT), [peak]);
  const paths = useMemo(() => series.map(entry => seriesPaths(entry.values, scale.max)), [series, scale.max]);
  const labels = axisLabelIndices(periods.length);
  const count = periods.length;

  const onMove = (event: React.MouseEvent<HTMLDivElement>) => {
    const rect = plotRef.current?.getBoundingClientRect();
    if (!rect || rect.width === 0 || count === 0) return;
    setHover(hoverIndex((event.clientX - rect.left) / rect.width, count));
  };

  const hovered = hover !== null && hover < count ? hover : null;
  const hoverTotal = hovered === null ? 0 : series.reduce((sum, entry) => sum + (entry.values[hovered] ?? 0), 0);
  const tooltipLeft = hovered === null || count <= 1 ? 0 : (hovered / (count - 1)) * 100;

  return <figure className={cn("min-w-0", className)}>
    <div className="flex gap-2">
      <div className="relative w-14 shrink-0 text-right text-[11px] tabular-nums text-muted-foreground" aria-hidden="true">
        {scale.ticks.map(tick => <span key={tick} className="absolute right-0 -translate-y-1/2" style={{ top: `${(chartY(tick, scale.max) / height) * 100}%` }}>{tick === 0 ? "0" : format(tick)}</span>)}
      </div>
      <div ref={plotRef} className="relative h-56 min-w-0 flex-1" onMouseMove={onMove} onMouseLeave={() => setHover(null)}>
        <svg viewBox={`0 0 ${width} ${height}`} preserveAspectRatio="none" className="block h-full w-full overflow-visible" role="img" aria-label={`${resolution === "hour" ? "Hourly" : "Daily"} ${metric === "cost" ? "cost" : "processed tokens"} by harness`}>
          {scale.ticks.map(tick => <line key={tick} x1={0} x2={width} y1={chartY(tick, scale.max)} y2={chartY(tick, scale.max)} className="stroke-border" strokeWidth={1} vectorEffect="non-scaling-stroke" />)}
          {/* Heavier series first so the lighter one is not buried; all fills, then all strokes, so no series covers another's line. */}
          {paths.map((path, index) => <path key={`fill-${series[index].harness}`} d={path.area} className={cn(FILL_RAMP[Math.min(index, FILL_RAMP.length - 1)], "stroke-none")} data-series={series[index].harness} />)}
          {paths.map((path, index) => <path key={`line-${series[index].harness}`} d={path.line} fill="none" className={STROKE_RAMP[Math.min(index, STROKE_RAMP.length - 1)]} strokeWidth={1.75} strokeLinejoin="round" vectorEffect="non-scaling-stroke" />)}
          {hovered !== null && <line x1={chartX(hovered, count)} x2={chartX(hovered, count)} y1={0} y2={height} className="stroke-foreground/50" strokeWidth={1} vectorEffect="non-scaling-stroke" />}
        </svg>
        {hovered !== null && <div role="tooltip" className={cn("u-glass-popover pointer-events-none absolute top-2 z-10 min-w-40 rounded-xl px-3 py-2 text-caption", tooltipLeft > 60 ? "-translate-x-[calc(100%+12px)]" : "translate-x-3")} style={{ left: `${tooltipLeft}%` }}>
          <div className="mb-1 font-medium text-foreground">{formatPeriodLabel(periods[hovered], resolution, timeZone)}</div>
          {series.map((entry, index) => <div key={entry.harness} className="flex items-center justify-between gap-4 text-muted-foreground">
            <span className="inline-flex items-center gap-1.5"><span className={cn("size-2 rounded-[3px]", seriesDotClass(index))} /><HarnessMark harness={entry.harness} size={11} />{harnessLabel(entry.harness)}</span>
            <span className="tabular-nums text-foreground">{format(entry.values[hovered] ?? 0)}</span>
          </div>)}
          <div className="mt-1 flex items-center justify-between gap-4 border-t border-border pt-1 font-medium text-foreground"><span>Total</span><span className="tabular-nums">{format(hoverTotal)}</span></div>
        </div>}
      </div>
    </div>
    <div className="mt-1.5 flex justify-between pl-16 text-[11px] tabular-nums text-muted-foreground" aria-hidden="true">
      {labels.map(index => <span key={index}>{formatPeriodLabel(periods[index], resolution, timeZone)}</span>)}
    </div>
  </figure>;
}
