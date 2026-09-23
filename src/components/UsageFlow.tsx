import { useMemo } from "react";
import { cn } from "@/lib/utils";
import { harnessLabel } from "../utils";
import { harnessChartFill } from "./harnessMarks";
import { formatPercent, formatPeriodLabel, formatTokens, formatUsd } from "../usageReport";
import { flowBandPath, flowLayout, harnessesByMetric, harnessValue, stackColumns } from "../usageGeometry";
import { AxisLabels, EstimateMark, EYEBROW, formatMetric, HeroCaption, KIND_SHADE, LAYOUT_CARD, PartialBadge, periodNoun, type UsageLayoutProps } from "./UsageLayoutParts";

// Layout C: where the tokens go. Bands run harness → model → token kind, so
// the dominant fact of most histories (nearly everything is a cache read)
// is something you see rather than a tile you read. Cost is not split by
// token kind in the ledger, so cost mode stops at the model and says so.

const WIDTH = 900;
const NODE = 10;
const TIMELINE = { width: 900, height: 72 };

export function UsageFlow({ report, window, metric, partial }: UsageLayoutProps) {
  const format = formatMetric(metric);
  const layout = useMemo(() => flowLayout(report, metric, { height: 380, padding: 10, minNode: 3 }), [report, metric]);
  const columnX = layout.columns === 3 ? [150, 450, WIDTH - 180] : [190, WIDTH - 300];
  const byId = useMemo(() => new Map(layout.nodes.map(node => [node.id, node])), [layout.nodes]);
  const harnesses = harnessesByMetric(report, metric);
  const columns = useMemo(() => stackColumns(report.periods, harnesses.map(entry => entry.harness), metric), [report.periods, harnesses, metric]);
  const peak = Math.max(0, ...columns.map(column => column.total));
  const slot = TIMELINE.width / Math.max(1, columns.length);
  const processed = report.totals.processedTokens;
  const cacheShare = processed > 0 ? report.totals.cacheReadTokens / processed : 0;
  const lead = harnesses[0];
  const leadShare = lead ? (metric === "cost" ? lead.costShare : lead.tokenShare) : 0;
  const empty = layout.nodes.length === 0;

  return <div className="space-y-6">
    <section aria-label="Summary">
      <PartialBadge label={partial} className="mb-4" />
      {empty ? <h2 className="text-[2rem] font-normal leading-tight tracking-[-0.03em] text-foreground">No activity in this window.</h2> : metric === "tokens"
        ? <h2 className="max-w-3xl text-[2rem] font-normal leading-tight tracking-[-0.03em] text-foreground">{formatPercent(cacheShare, 0)} of what you processed was a cache read. <span className="text-muted-foreground">Here is where the other {formatTokens(processed - report.totals.cacheReadTokens)} went.</span></h2>
        : <h2 className="max-w-3xl text-[2rem] font-normal leading-tight tracking-[-0.03em] text-foreground">{harnessLabel(lead!.harness)} carried {formatPercent(leadShare, 0)} of the cost. <span className="text-muted-foreground">Here is how it splits by model.</span></h2>}
      <p className="mt-3 text-[14.5px] text-muted-foreground">
        <span className="font-medium tabular-nums text-foreground">{formatTokens(processed)}</span> tokens · <span className="font-medium tabular-nums text-foreground">{formatUsd(report.totals.costMicrousd)}</span><EstimateMark report={report} /> at API rates · cache reuse saved <span className="font-medium tabular-nums text-foreground">{formatUsd(report.totals.cacheSavingsMicrousd)}</span>
      </p>
      <HeroCaption report={report} metric={metric} className="mt-1" />
    </section>

    {!empty && <section aria-label={metric === "tokens" ? "Token flow" : "Cost flow"} className={LAYOUT_CARD}>
      <div className="relative mb-2 h-4" aria-hidden="true">
        <span className={cn(EYEBROW, "absolute -translate-x-full")} style={{ left: `${((columnX[0] + NODE) / WIDTH) * 100}%` }}>Harness</span>
        <span className={cn(EYEBROW, "absolute")} style={{ left: `${(columnX[1] / WIDTH) * 100}%` }}>Model</span>
        {layout.columns === 3 && <span className={cn(EYEBROW, "absolute")} style={{ left: `${(columnX[2] / WIDTH) * 100}%` }}>Token kind</span>}
      </div>
      <svg viewBox={`0 0 ${WIDTH} ${layout.height}`} className="block h-auto w-full overflow-visible" role="img" aria-label={metric === "tokens" ? "Processed tokens flowing from harness to model to token kind" : "Cost flowing from harness to model"}>
        {layout.links.map(link => {
          const source = byId.get(link.source)!;
          const target = byId.get(link.target)!;
          return <path key={`${link.source}>${link.target}`} d={flowBandPath(link, columnX[source.column] + NODE, columnX[target.column])} className={harnessChartFill(link.harness)} fillOpacity={target.column === 2 ? 0.24 : 0.34}>
            <title>{`${source.harness ? harnessLabel(source.harness) : source.label} → ${target.kind ? target.label : target.label}: ${format(link.value)}`}</title>
          </path>;
        })}
        {layout.nodes.map(node => <rect key={node.id} x={columnX[node.column]} y={node.y} width={NODE} height={node.height} rx={2} className={node.kind ? "fill-foreground" : harnessChartFill(node.harness)} fillOpacity={node.kind ? KIND_SHADE[node.kind].opacity : 1} />)}
        {layout.nodes.filter(node => node.column === 0).map(node => <g key={`label-${node.id}`}>
          <text x={columnX[0] - 12} y={node.y + node.height / 2 - 2} textAnchor="end" className="fill-foreground text-[13.5px] font-medium">{harnessLabel(node.harness!)}</text>
          <text x={columnX[0] - 12} y={node.y + node.height / 2 + 14} textAnchor="end" className="fill-muted-foreground text-[12px] tabular-nums">{format(node.value)}</text>
        </g>)}
        {layout.nodes.filter(node => node.column === 1).map(node => <text key={`label-${node.id}`} x={columnX[1] + NODE + 8} y={node.y + node.height / 2 + 4} className="fill-foreground stroke-card font-mono text-[11.5px] [paint-order:stroke]" strokeWidth={4} strokeLinejoin="round">
          {node.label}{node.model?.costSource === "unpriced" ? " (unpriced)" : ""} <tspan className="fill-muted-foreground font-sans tabular-nums">{format(node.value)}</tspan>
        </text>)}
        {layout.nodes.filter(node => node.column === 2).map(node => node.height > 40
          ? <g key={`label-${node.id}`}>
            <text x={columnX[2] + NODE + 12} y={node.y + node.height / 2 - 4} className="fill-foreground text-[13.5px] font-medium">{node.label}</text>
            <text x={columnX[2] + NODE + 12} y={node.y + node.height / 2 + 13} className="fill-muted-foreground text-[12px] tabular-nums">{formatTokens(node.value)} · {formatPercent(processed > 0 ? node.value / processed : 0)}</text>
          </g>
          : <text key={`label-${node.id}`} x={columnX[2] + NODE + 12} y={node.y + node.height / 2 + 4} className="fill-foreground text-[12.5px]">{node.label} <tspan className="fill-muted-foreground tabular-nums">{formatTokens(node.value)} · {formatPercent(processed > 0 ? node.value / processed : 0)}</tspan></text>)}
      </svg>
      {metric === "cost" && <p className="mt-3 text-caption text-muted-foreground">Cost is recorded per model, not per token kind, so this view stops at the model. Switch to Tokens to follow each token to cache, input, or output.</p>}

      <div className="mt-5 border-t border-border pt-4">
        <div className="mb-2.5 flex items-center justify-between">
          <h3 className="text-ui font-medium text-foreground">{periodNoun(window)}</h3>
          <span className="text-caption text-muted-foreground">stacked by harness</span>
        </div>
        <svg viewBox={`0 0 ${TIMELINE.width} ${TIMELINE.height}`} preserveAspectRatio="none" className="block h-[4.5rem] w-full" role="img" aria-label={`${periodNoun(window)} ${metric === "cost" ? "cost" : "processed tokens"} stacked by harness`}>
          <line x1={0} x2={TIMELINE.width} y1={TIMELINE.height} y2={TIMELINE.height} className="stroke-border" vectorEffect="non-scaling-stroke" />
          {columns.map((column, index) => column.segments.map(segment => {
            const y0 = TIMELINE.height - (peak > 0 ? (segment.end / peak) * (TIMELINE.height - 4) : 0);
            const h = peak > 0 ? (segment.value / peak) * (TIMELINE.height - 4) : 0;
            return <rect key={`${column.period}-${segment.harness}`} x={index * slot + slot * 0.19} y={y0} width={slot * 0.62} height={Math.max(h - 1, 0.5)} className={harnessChartFill(segment.harness)} fillOpacity={0.85} />;
          }))}
        </svg>
        <AxisLabels slots count={columns.length} label={tick => formatPeriodLabel(columns[tick].period, window.resolution, window.timeZone)} className="mt-1.5" />
      </div>
      <ul className="sr-only">
        {harnesses.map(entry => <li key={entry.harness}>{harnessLabel(entry.harness)}: {format(harnessValue(entry, metric))}</li>)}
      </ul>
    </section>}
  </div>;
}
