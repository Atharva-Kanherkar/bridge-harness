import { useId, useMemo, useState } from "react";
import { cn } from "@/lib/utils";
import { harnessLabel } from "../utils";
import { harnessChartDot, harnessChartText } from "./harnessMarks";
import { chartY, formatPercent, formatPeriodLabel, formatTokens, formatUsd, type ChartGeometry } from "../usageReport";
import { axisFraction, harnessesByMetric, harnessValue, modelValue, periodHarnessValue, periodValue, straightPaths } from "../usageGeometry";
import { AxisLabels, EstimateMark, formatMetric, HarnessName, HeroCaption, LAYOUT_CARD, PartialBadge, periodNoun, useScrub, type UsageLayoutProps } from "./UsageLayoutParts";

// Layout B: small multiples. One strip per harness on one shared clock, so
// no series is ever drawn over another. A single rule scrubs every strip at
// once, by pointer or arrow keys, and the readout names every harness.

type StripScale = "shared" | "own";

const STRIP: ChartGeometry = { width: 600, height: 56, plotTop: 6 };
const TOTAL_STRIP: ChartGeometry = { width: 600, height: 72, plotTop: 6 };
const GRID = "grid grid-cols-[8.5rem_minmax(0,1fr)_7.5rem] items-center gap-x-4";

function Strip({ values, max, geometry, index, harness, gradientId }: { values: number[]; max: number; geometry: ChartGeometry; index: number | null; harness: string | null; gradientId: string }) {
  const paths = straightPaths(values, max, geometry);
  const tone = harness ? harnessChartText(harness) : "text-foreground";
  return <div className={cn("relative", tone)} style={{ height: geometry.height }}>
    <svg viewBox={`0 0 ${geometry.width} ${geometry.height}`} preserveAspectRatio="none" className="block h-full w-full overflow-visible" aria-hidden="true">
      <defs><linearGradient id={gradientId} x1="0" y1="0" x2="0" y2="1"><stop offset="0" stopColor="currentColor" stopOpacity={0.36} /><stop offset="1" stopColor="currentColor" stopOpacity={0.02} /></linearGradient></defs>
      <line x1={0} x2={geometry.width} y1={geometry.height} y2={geometry.height} className="stroke-border" vectorEffect="non-scaling-stroke" />
      <path d={paths.area} fill={`url(#${gradientId})`} className="motion-safe:animate-[chart-rise_600ms_ease-out_both] origin-bottom" />
      <path d={paths.line} fill="none" stroke="currentColor" strokeWidth={1.5} strokeLinejoin="round" vectorEffect="non-scaling-stroke" />
    </svg>
    {index !== null && <>
      <span className="pointer-events-none absolute inset-y-0 w-px bg-foreground/25" style={{ left: `${axisFraction(index, values.length) * 100}%` }} aria-hidden="true" />
      <span className="pointer-events-none absolute size-2 -translate-x-1/2 -translate-y-1/2 rounded-full bg-current ring-2 ring-card" style={{ left: `${axisFraction(index, values.length) * 100}%`, top: `${(chartY(values[index] ?? 0, max, geometry) / geometry.height) * 100}%` }} aria-hidden="true" />
    </>}
  </div>;
}

export function UsageStrips({ report, window, metric, partial }: UsageLayoutProps) {
  const [scale, setScale] = useState<StripScale>("shared");
  const format = formatMetric(metric);
  const id = useId().replace(/:/g, "");
  const count = report.periods.length;
  const scrub = useScrub(count);
  const harnesses = harnessesByMetric(report, metric);
  const series = useMemo(() => harnesses.map(entry => ({ harness: entry.harness, values: report.periods.map(period => periodHarnessValue(period, entry.harness, metric)) })), [harnesses, report.periods, metric]);
  const totals = useMemo(() => report.periods.map(period => periodValue(period, metric)), [report.periods, metric]);
  const sharedPeak = Math.max(0, ...series.flatMap(entry => entry.values));
  const totalPeak = Math.max(0, ...totals);
  const models = [...report.models].sort((a, b) => modelValue(b, metric) - modelValue(a, metric));
  const topModel = models[0] ? modelValue(models[0], metric) : 0;
  const cacheShare = report.totals.processedTokens > 0 ? report.totals.cacheReadTokens / report.totals.processedTokens : 0;
  const index = scrub.index;
  const readoutRight = index !== null && axisFraction(index, count) > 0.62;
  const title = `${periodNoun(window)} ${metric === "cost" ? "cost" : "processed tokens"}`;

  return <div className="space-y-5">
    <section aria-label="Summary" className="flex flex-wrap items-end justify-between gap-6">
      <div>
        <PartialBadge label={partial} className="mb-3" />
        <div className="text-[3.5rem] font-light leading-none tracking-[-0.045em] tabular-nums text-foreground">{format(metric === "cost" ? report.totals.costMicrousd : report.totals.processedTokens)}{metric === "cost" && <EstimateMark report={report} large />}</div>
        <p className="mt-2.5 text-[14px] text-muted-foreground">{metric === "cost"
          ? <>at API rates · <span className="font-medium tabular-nums text-foreground">{formatTokens(report.totals.processedTokens)}</span> tokens processed</>
          : <>tokens processed · <span className="font-medium tabular-nums text-foreground">{formatUsd(report.totals.costMicrousd)}</span><EstimateMark report={report} /> at API rates</>}</p>
        <HeroCaption report={report} metric={metric} className="mt-1" />
      </div>
      <dl className="flex divide-x divide-border">
        {[
          ["Requests", report.totals.records.toLocaleString("en-US")],
          ["From cache", formatPercent(cacheShare)],
          ["Output", formatTokens(report.totals.outputTokens)],
          ["Cache saved", formatUsd(report.totals.cacheSavingsMicrousd)],
        ].map(([label, value]) => <div key={label} className="px-5 first:pl-0 last:pr-0">
          <dt className="text-[11.5px] text-muted-foreground">{label}</dt>
          <dd className="mt-1 text-[17px] tracking-tight tabular-nums text-foreground">{value}</dd>
        </div>)}
      </dl>
    </section>

    <section aria-label={title} className={LAYOUT_CARD}>
      <div className="mb-2 flex items-center gap-3">
        <h2 className="text-ui font-medium text-foreground">{title}</h2>
        <div role="radiogroup" aria-label="Strip scale" className="u-segmented ml-auto">
          {(["shared", "own"] as const).map(option => <button key={option} type="button" role="radio" aria-checked={scale === option} data-active={scale === option} className="u-segmented-item" onClick={() => setScale(option)}>{option === "shared" ? "Shared scale" : "Own scale"}</button>)}
        </div>
      </div>
      {count === 0 || harnesses.length === 0 ? <p className="py-10 text-center text-caption text-muted-foreground">No activity in this window.</p> : <div
        role="group"
        tabIndex={0}
        aria-label={`${title} by harness. Use the arrow keys to read one ${window.resolution === "hour" ? "hour" : "day"}.`}
        onKeyDown={scrub.onKeyDown}
        onBlur={scrub.clear}
        className="relative rounded-lg outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <div className={cn(GRID, "py-1")}>
          <span className="text-ui font-medium text-foreground">All harnesses</span>
          <div onPointerMove={scrub.fromPointer} onPointerLeave={scrub.clear} className="relative">
            <Strip values={totals} max={totalPeak} geometry={TOTAL_STRIP} index={index} harness={null} gradientId={`${id}-all`} />
            {index !== null && <div role="status" aria-live="polite" className={cn("u-glass-popover pointer-events-none absolute top-1 z-10 w-52 rounded-xl px-3 py-2 text-caption", readoutRight ? "-translate-x-[calc(100%+12px)]" : "translate-x-3")} style={{ left: `${axisFraction(index, count) * 100}%` }}>
              <div className="mb-1 text-muted-foreground">{formatPeriodLabel(report.periods[index].period, window.resolution, window.timeZone)}</div>
              {series.map(entry => <div key={entry.harness} className="flex items-center gap-2 leading-5">
                <span className={cn("size-2 rounded-full", harnessChartDot(entry.harness))} aria-hidden="true" />
                <span className="flex-1 text-muted-foreground">{harnessLabel(entry.harness)}</span>
                <span className="tabular-nums text-foreground">{format(entry.values[index] ?? 0)}</span>
              </div>)}
              <div className="mt-1 flex items-center justify-between border-t border-border pt-1 font-medium text-foreground"><span>Total</span><span className="tabular-nums">{format(totals[index] ?? 0)}</span></div>
            </div>}
          </div>
          <span className="text-right"><span className="block text-[14px] tabular-nums text-foreground">{format(metric === "cost" ? report.totals.costMicrousd : report.totals.processedTokens)}</span><span className="block text-[11.5px] tabular-nums text-muted-foreground">{metric === "cost" ? `${formatTokens(report.totals.processedTokens)} tokens` : formatUsd(report.totals.costMicrousd)}</span></span>
        </div>
        {series.map(entry => {
          const harness = harnesses.find(item => item.harness === entry.harness)!;
          const own = Math.max(0, ...entry.values);
          return <div key={entry.harness} className={cn(GRID, "border-t border-border py-1")}>
            <span className="flex items-center gap-2 text-ui text-foreground"><span className={cn("size-2 shrink-0 rounded-full", harnessChartDot(entry.harness))} aria-hidden="true" /><HarnessName harness={entry.harness} size={12} /></span>
            <div onPointerMove={scrub.fromPointer} onPointerLeave={scrub.clear}>
              <Strip values={entry.values} max={scale === "shared" ? sharedPeak : own} geometry={STRIP} index={index} harness={entry.harness} gradientId={`${id}-${entry.harness}`} />
            </div>
            <span className="text-right"><span className="block text-[14px] tabular-nums text-foreground">{format(harnessValue(harness, metric))}</span><span className="block text-[11.5px] tabular-nums text-muted-foreground">{formatPercent(metric === "cost" ? harness.costShare : harness.tokenShare)} · {metric === "cost" ? formatTokens(harness.processedTokens) : formatUsd(harness.costMicrousd)}</span></span>
          </div>;
        })}
        <div className={cn(GRID, "pt-1.5")} aria-hidden="true">
          <span />
          <AxisLabels count={count} label={tick => formatPeriodLabel(report.periods[tick].period, window.resolution, window.timeZone)} />
          <span />
        </div>
      </div>}
    </section>

    {models.length > 0 && <section aria-label="By model" className={LAYOUT_CARD}>
      <div className="mb-3 flex items-center justify-between gap-3">
        <h2 className="text-ui font-medium text-foreground">By model</h2>
        <span className="text-caption text-muted-foreground">{metric === "cost" ? "share of cost" : "share of processed tokens"}</span>
      </div>
      <ul className="grid gap-x-10 md:grid-cols-2">
        {models.map(model => <li key={`${model.harness}:${model.model}`} className="grid h-8 grid-cols-[minmax(0,9rem)_minmax(0,1fr)_4.5rem_4.5rem] items-center gap-3">
          <span className="flex min-w-0 items-center gap-1.5"><span className="truncate font-mono text-caption text-foreground/85">{model.model}</span>{model.costSource === "unpriced" && <span className="shrink-0 text-[11px] text-muted-foreground">unpriced</span>}</span>
          <span className="h-1.5 overflow-hidden rounded-full bg-muted"><span className={cn("block h-full rounded-full", harnessChartDot(model.harness))} style={{ width: `${topModel > 0 ? (modelValue(model, metric) / topModel) * 100 : 0}%` }} /></span>
          <span className="text-right text-ui tabular-nums text-foreground">{format(modelValue(model, metric))}</span>
          <span className="text-right text-caption tabular-nums text-muted-foreground">{metric === "cost" ? formatTokens(model.tokens) : formatUsd(model.costMicrousd)}</span>
        </li>)}
      </ul>
    </section>}
  </div>;
}
