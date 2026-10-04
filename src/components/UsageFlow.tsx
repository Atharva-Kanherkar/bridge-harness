import { useMemo } from "react";
import { cn } from "@/lib/utils";
import { harnessLabel } from "../utils";
import { harnessChartFill } from "./harnessMarks";
import { formatPercent, formatPeriodLabel, formatTokens, formatUsd } from "../usageReport";
import { flowBandPath, flowLayout, harnessesByMetric, harnessValue, modelsOf, modelValue, spreadLabels, stackColumns, type FlowNode } from "../usageGeometry";
import { AxisLabels, EstimateMark, EYEBROW, formatMetric, HeroCaption, KIND_SHADE, LAYOUT_CARD, PartialBadge, periodNoun, type UsageLayoutProps } from "./UsageLayoutParts";

// Layout C: where the tokens go. Bands run harness → model → token kind, so
// the dominant fact of most histories (nearly everything is a cache read)
// is something you see rather than a tile you read. Cost is not split by
// token kind in the ledger, so cost mode stops at the model and says so.

const WIDTH = 900;
const NODE = 10;
const TIMELINE = { width: 900, height: 72 };
/** Models drawn per harness before the rest share one `N more` band. */
const MAX_MODELS = 4;
const TOKEN_KIND_ROWS = 4;
/** Vertical room each label needs: two lines for harnesses and big kinds, one for models. */
const HARNESS_GAP = 36;
const MODEL_GAP = 19;
const KIND_GAP = 34;

/** A short elbow from a node's edge to its label when the label had to move to clear a neighbour. */
function Leader({ x, from, to, side }: { x: number; from: number; to: number; side: "left" | "right" }) {
  if (Math.abs(from - to) < 3) return null;
  const reach = side === "left" ? -7 : 7;
  return <path d={`M${x},${from} L${x + reach},${to}`} className="fill-none stroke-muted-foreground/40" strokeWidth={1} />;
}

export function UsageFlow({ report, window, metric, partial }: UsageLayoutProps) {
  const format = formatMetric(metric);
  // The drawing grows with the number of labels it has to carry, so a long
  // model list or many harnesses never stack their labels on top of each other.
  const layout = useMemo(() => {
    let harnessCount = 0;
    let modelCount = 0;
    for (const entry of harnessesByMetric(report, metric)) {
      if (harnessValue(entry, metric) <= 0) continue;
      harnessCount += 1;
      modelCount += Math.min(MAX_MODELS, modelsOf(report, entry.harness, metric).filter(model => modelValue(model, metric) > 0).length);
    }
    const height = Math.max(320, harnessCount * (HARNESS_GAP + 12), modelCount * (MODEL_GAP + 5), TOKEN_KIND_ROWS * KIND_GAP + 40);
    return flowLayout(report, metric, { height, padding: 10, minNode: 3, maxModels: MAX_MODELS });
  }, [report, metric]);
  const columnNodes = (column: number) => layout.nodes.filter(node => node.column === column);
  const place = (nodes: FlowNode[], gap: number, edge: number) => spreadLabels(nodes.map(node => node.y + node.height / 2), gap, edge, layout.height - edge);
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
            <title>{`${source.column === 0 ? harnessLabel(source.harness!) : source.label} → ${target.label}: ${format(link.value)}`}</title>
          </path>;
        })}
        {layout.nodes.map(node => <rect key={node.id} x={columnX[node.column]} y={node.y} width={NODE} height={node.height} rx={2} className={node.kind ? "fill-foreground" : harnessChartFill(node.harness)} fillOpacity={node.kind ? KIND_SHADE[node.kind].opacity : 1} />)}
        {(() => {
          const nodes = columnNodes(0);
          const ys = place(nodes, HARNESS_GAP, 16);
          return nodes.map((node, index) => <g key={`label-${node.id}`}>
            <Leader x={columnX[0] - 2} from={node.y + node.height / 2} to={ys[index]} side="left" />
            <text x={columnX[0] - 12} y={ys[index] - 3} textAnchor="end" className="fill-foreground text-[13.5px] font-medium">{harnessLabel(node.harness!)}</text>
            <text x={columnX[0] - 12} y={ys[index] + 13} textAnchor="end" className="fill-muted-foreground text-[12px] tabular-nums">{format(node.value)}</text>
          </g>);
        })()}
        {(() => {
          const nodes = columnNodes(1);
          const ys = place(nodes, MODEL_GAP, 8);
          return nodes.map((node, index) => <g key={`label-${node.id}`}>
            <Leader x={columnX[1] + NODE + 1} from={node.y + node.height / 2} to={ys[index]} side="right" />
            <text x={columnX[1] + NODE + 8} y={ys[index] + 4} className={cn("stroke-card text-[11.5px] [paint-order:stroke]", node.model ? "fill-foreground font-mono" : "fill-muted-foreground")} strokeWidth={4} strokeLinejoin="round">
              {node.label}{node.model?.costSource === "unpriced" ? " (unpriced)" : ""} <tspan className="fill-muted-foreground font-sans tabular-nums">{format(node.value)}</tspan>
            </text>
          </g>);
        })()}
        {(() => {
          const nodes = columnNodes(2);
          const ys = place(nodes, KIND_GAP, 16);
          return nodes.map((node, index) => <g key={`label-${node.id}`}>
            <Leader x={columnX[2] + NODE + 1} from={node.y + node.height / 2} to={ys[index]} side="right" />
            <text x={columnX[2] + NODE + 12} y={ys[index] - 3} className="fill-foreground text-[13px] font-medium">{node.label}</text>
            <text x={columnX[2] + NODE + 12} y={ys[index] + 13} className="fill-muted-foreground text-[12px] tabular-nums">{formatTokens(node.value)} · {formatPercent(processed > 0 ? node.value / processed : 0)}</text>
          </g>);
        })()}
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
