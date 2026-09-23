import { useId, useMemo } from "react";
import { cn } from "@/lib/utils";
import { harnessLabel } from "../utils";
import { harnessChartDot, harnessChartFill } from "./harnessMarks";
import { formatPercent, formatPeriodLabel, formatTokens, formatUsd, niceScale } from "../usageReport";
import { harnessesByMetric, harnessValue, modelValue, periodHarnessValue, stackColumns, TOKEN_KINDS, treemap } from "../usageGeometry";
import { AxisLabels, EstimateMark, formatMetric, HarnessName, HeroCaption, KIND_SHADE, LAYOUT_CARD, PartialBadge, periodNoun, useScrub, type UsageLayoutProps } from "./UsageLayoutParts";

// Layout D: discrete stacked columns instead of curves, so no bar ever draws
// a value between two real periods, beside a treemap where each model's area
// is its share of the metric. The table stays one disclosure away.

const COLUMNS = { width: 560, height: 250, left: 44, top: 8, bottom: 4 };
const TREE = { width: 360, height: 262 };
/** Depth by rank inside a harness: the largest model is the full hue. */
const RANK_OPACITY = [0.92, 0.68, 0.5, 0.38];

export function UsageMosaic({ report, window, metric, partial }: UsageLayoutProps) {
  const format = formatMetric(metric);
  const id = useId().replace(/:/g, "");
  const harnesses = harnessesByMetric(report, metric);
  const columns = useMemo(() => stackColumns(report.periods, harnesses.map(entry => entry.harness), metric), [report.periods, harnesses, metric]);
  const scale = niceScale(Math.max(0, ...columns.map(column => column.total)), 4);
  const plotWidth = COLUMNS.width - COLUMNS.left;
  const plotHeight = COLUMNS.height - COLUMNS.top - COLUMNS.bottom;
  const slot = plotWidth / Math.max(1, columns.length);
  const bar = slot * 0.64;
  const y = (value: number) => COLUMNS.top + plotHeight - (scale.max > 0 ? (value / scale.max) * plotHeight : 0);
  const scrub = useScrub(columns.length);
  const hovered = scrub.index;
  const groups = useMemo(() => treemap(report, metric, { x: 0, y: 0, w: TREE.width, h: TREE.height }), [report, metric]);
  const title = `${periodNoun(window)} ${metric === "cost" ? "cost" : "processed tokens"}`;
  const hoverLeft = hovered === null ? 0 : (COLUMNS.left + (hovered + 0.5) * slot) / COLUMNS.width;

  return <div className="space-y-4">
    <section aria-label="Summary" className="flex flex-wrap items-end gap-x-10 gap-y-4 pb-2">
      <div>
        <PartialBadge label={partial} className="mb-3" />
        <div className="text-[2.75rem] font-light leading-none tracking-[-0.04em] tabular-nums text-foreground">{format(metric === "cost" ? report.totals.costMicrousd : report.totals.processedTokens)}{metric === "cost" && <EstimateMark report={report} large />}</div>
        <p className="mt-2 text-ui text-muted-foreground">{metric === "cost"
          ? <>at API rates · <span className="font-medium tabular-nums text-foreground">{formatTokens(report.totals.processedTokens)}</span> tokens</>
          : <>tokens · <span className="font-medium tabular-nums text-foreground">{formatUsd(report.totals.costMicrousd)}</span><EstimateMark report={report} /> at API rates</>}</p>
        <HeroCaption report={report} metric={metric} className="mt-0.5" />
      </div>
      <ul className="flex flex-wrap gap-x-7 gap-y-3 pb-1" aria-label="By harness">
        {harnesses.map(entry => <li key={entry.harness}>
          <span className="flex items-center gap-1.5 text-caption text-muted-foreground"><span className={cn("size-2 rounded-full", harnessChartDot(entry.harness))} aria-hidden="true" /><HarnessName harness={entry.harness} size={11} /></span>
          <span className="mt-0.5 block text-[17px] tabular-nums text-foreground">{format(harnessValue(entry, metric))} <span className="text-caption text-muted-foreground">{formatPercent(metric === "cost" ? entry.costShare : entry.tokenShare, 0)}</span></span>
        </li>)}
      </ul>
    </section>

    <div className="grid gap-4 lg:grid-cols-[minmax(0,1.45fr)_minmax(0,1fr)]">
      <section aria-label={title} className={cn(LAYOUT_CARD, "relative min-w-0")}>
        <div className="mb-3 flex items-center justify-between gap-3">
          <h2 className="text-ui font-medium text-foreground">{title}</h2>
          <span className="text-caption text-muted-foreground">stacked by harness</span>
        </div>
        <div
          role="group"
          tabIndex={0}
          aria-label={`${title} stacked by harness. Use the arrow keys to read one ${window.resolution === "hour" ? "hour" : "day"}.`}
          onKeyDown={scrub.onKeyDown}
          onBlur={scrub.clear}
          onPointerMove={event => {
            const rect = event.currentTarget.getBoundingClientRect();
            const x = ((event.clientX - rect.left) / rect.width) * COLUMNS.width - COLUMNS.left;
            if (x < 0 || columns.length === 0) { scrub.clear(); return; }
            scrub.setIndex(Math.min(columns.length - 1, Math.floor(x / slot)));
          }}
          onPointerLeave={scrub.clear}
          className="relative rounded-lg outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <svg viewBox={`0 0 ${COLUMNS.width} ${COLUMNS.height}`} className="block h-auto w-full" role="img" aria-label={`${title} by harness`}>
            <defs>{columns.map((column, index) => <clipPath key={column.period} id={`${id}-c${index}`}><rect x={COLUMNS.left + index * slot + (slot - bar) / 2} y={y(column.total)} width={bar} height={Math.max(0, y(0) - y(column.total))} rx={Math.min(3, bar / 2)} /></clipPath>)}</defs>
            {scale.ticks.map(tick => <g key={tick}>
              <line x1={COLUMNS.left} x2={COLUMNS.width} y1={y(tick)} y2={y(tick)} className={tick === 0 ? "stroke-border" : "stroke-border/60"} />
              <text x={COLUMNS.left - 8} y={y(tick) + 4} textAnchor="end" className="fill-muted-foreground/70 text-[11px] tabular-nums">{tick === 0 ? "0" : format(tick)}</text>
            </g>)}
            {columns.map((column, index) => <g key={column.period} clipPath={`url(#${id}-c${index})`} className={cn("transition-opacity", hovered !== null && hovered !== index && "opacity-45")}>
              {column.segments.map(segment => <rect key={segment.harness} x={COLUMNS.left + index * slot + (slot - bar) / 2} y={y(segment.end)} width={bar} height={Math.max(0.6, y(segment.start) - y(segment.end) - 1.2)} className={harnessChartFill(segment.harness)} />)}
            </g>)}
          </svg>
          <AxisLabels slots count={columns.length} label={tick => formatPeriodLabel(columns[tick].period, window.resolution, window.timeZone)} className="ml-[7.86%] mt-1" />
          {hovered !== null && <div role="status" aria-live="polite" className={cn("u-glass-popover pointer-events-none absolute top-6 z-10 w-48 rounded-xl px-3 py-2 text-caption", hoverLeft > 0.62 ? "-translate-x-[calc(100%+14px)]" : "translate-x-3.5")} style={{ left: `${hoverLeft * 100}%` }}>
            <div className="mb-1 text-muted-foreground">{formatPeriodLabel(columns[hovered].period, window.resolution, window.timeZone)}</div>
            {harnesses.map(entry => <div key={entry.harness} className="flex items-center gap-2 leading-5">
              <span className={cn("size-2 rounded-full", harnessChartDot(entry.harness))} aria-hidden="true" />
              <span className="flex-1 text-muted-foreground">{harnessLabel(entry.harness)}</span>
              <span className="tabular-nums text-foreground">{format(periodHarnessValue(report.periods[hovered], entry.harness, metric))}</span>
            </div>)}
            <div className="mt-1 flex items-center justify-between border-t border-border pt-1 font-medium text-foreground"><span>Total</span><span className="tabular-nums">{format(columns[hovered].total)}</span></div>
          </div>}
        </div>
      </section>

      <section aria-label="By model" className={cn(LAYOUT_CARD, "min-w-0")}>
        <div className="mb-3 flex items-center justify-between gap-3">
          <h2 className="text-ui font-medium text-foreground">By model</h2>
          <span className="text-caption text-muted-foreground">area = {metric === "cost" ? "cost" : "tokens"}</span>
        </div>
        {groups.length === 0 ? <p className="py-10 text-center text-caption text-muted-foreground">No activity in this window.</p> : <svg viewBox={`0 0 ${TREE.width} ${TREE.height}`} className="block h-auto w-full" role="img" aria-label={`Treemap of ${metric === "cost" ? "cost" : "processed tokens"} by harness and model`}>
          {groups.flatMap(group => group.tiles.map((tile, rank) => {
            const x = tile.x + 1.5, top = tile.y + 1.5, w = Math.max(0, tile.w - 3), h = Math.max(0, tile.h - 3);
            const model = tile.item;
            return <g key={`${model.harness}:${model.model}`}>
              <rect x={x} y={top} width={w} height={h} rx={6} className={harnessChartFill(group.harness)} fillOpacity={RANK_OPACITY[Math.min(rank, RANK_OPACITY.length - 1)]}>
                <title>{`${harnessLabel(model.harness)} · ${model.model}: ${format(modelValue(model, metric))}${model.costSource === "unpriced" ? " (unpriced)" : ""}`}</title>
              </rect>
              {w > 70 && h > 40 ? <>
                <text x={x + 9} y={top + 18} className="pointer-events-none fill-foreground font-mono text-[11.5px]">{model.model.length > Math.floor(w / 7.2) ? `${model.model.slice(0, Math.max(3, Math.floor(w / 7.2) - 1))}…` : model.model}</text>
                <text x={x + 9} y={top + 34} className="pointer-events-none fill-foreground/70 text-[11.5px] tabular-nums">{format(modelValue(model, metric))}{metric === "tokens" ? ` · ${formatUsd(model.costMicrousd)}` : ""}</text>
              </> : w > 44 && h > 22 ? <text x={x + 7} y={top + 15} className="pointer-events-none fill-foreground/80 text-[10.5px] tabular-nums">{format(modelValue(model, metric))}</text> : null}
            </g>;
          }))}
        </svg>}
      </section>
    </div>

    <section aria-label="Token composition" className={cn(LAYOUT_CARD, "grid grid-cols-2 gap-0 p-0 sm:grid-cols-5")}>
      {TOKEN_KINDS.map(kind => <div key={kind.key} className="border-border px-5 py-4 sm:border-l sm:first:border-l-0">
        <div className="flex items-center gap-1.5 text-[11.5px] text-muted-foreground"><span className={cn("size-2 rounded-[2px]", KIND_SHADE[kind.key].bg)} aria-hidden="true" />{kind.label}</div>
        <div className="mt-1 text-lg tracking-tight tabular-nums text-foreground">{formatTokens(report.totals[kind.key])}</div>
        <div className="text-[11.5px] tabular-nums text-muted-foreground/70">{formatPercent(report.totals.processedTokens > 0 ? report.totals[kind.key] / report.totals.processedTokens : 0)} of processed</div>
      </div>)}
      <div className="border-border px-5 py-4 sm:border-l">
        <div className="text-[11.5px] text-muted-foreground">Cache saved</div>
        <div className="mt-1 text-lg tracking-tight tabular-nums text-foreground">{formatUsd(report.totals.cacheSavingsMicrousd)}</div>
        <div className="text-[11.5px] text-muted-foreground/70">reasoning {formatTokens(report.totals.reasoningTokens)}, inside output</div>
      </div>
    </section>
  </div>;
}
