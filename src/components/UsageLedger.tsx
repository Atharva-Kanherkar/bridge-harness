import { cn } from "@/lib/utils";
import { harnessLabel } from "../utils";
import { harnessChartDot } from "./harnessMarks";
import { formatCount, formatPercent, formatTokens, formatUsd } from "../usageReport";
import { harnessesByMetric, harnessValue, modelSeries, modelsOf } from "../usageGeometry";
import { EstimateMark, EYEBROW, formatMetric, HarnessName, HeroCaption, PartialBadge, Sparkline, TokenComposition, windowTitle, type UsageLayoutProps } from "./UsageLayoutParts";

// Layout A: an editorial receipt. Typography carries the hierarchy instead of
// cards: one large figure, a sentence that states the scale, a single ribbon
// of harness shares, then every model itemised with a leader to its amount.

const ROW = "grid grid-cols-[minmax(0,1fr)_5.5rem_6.5rem_7rem] items-center gap-x-2";

export function UsageLedger({ report, windowDays, metric, partial }: UsageLayoutProps) {
  const format = formatMetric(metric);
  const harnesses = harnessesByMetric(report, metric);
  const metricTotal = metric === "cost" ? report.totals.costMicrousd : report.totals.processedTokens;
  const periodTotals = report.periods.map(period => (metric === "cost" ? period.costMicrousd : period.tokens));
  const count = report.harnesses.length;

  return <div className="space-y-12">
    <section aria-label="Summary">
      <PartialBadge label={partial} className="mb-4" />
      <div className={EYEBROW}>{windowTitle(windowDays)}</div>
      <div className="mt-3 flex flex-wrap items-baseline gap-x-4 gap-y-1">
        <span className="text-[5.5rem] font-extralight leading-none tracking-[-0.05em] tabular-nums text-foreground">{format(metricTotal)}{metric === "cost" && <EstimateMark report={report} large />}</span>
        <span className="text-lg text-muted-foreground">{metric === "cost" ? "at API rates" : "tokens processed"}</span>
      </div>
      <p className="mt-4 max-w-2xl text-[17px] leading-relaxed text-muted-foreground">
        across <span className="font-medium text-foreground">{count} {count === 1 ? "harness" : "harnesses"}</span> and <span className="font-medium tabular-nums text-foreground">{formatCount(report.totals.records)}</span> requests.{" "}
        {metric === "tokens"
          ? <>At API rates that is <span className="font-medium tabular-nums text-foreground">{formatUsd(report.totals.costMicrousd)}</span><EstimateMark report={report} />. Your subscriptions bill separately.</>
          : <><span className="font-medium tabular-nums text-foreground">{formatTokens(report.totals.processedTokens)}</span> tokens processed. Not money spent: subscriptions bill separately.</>}
      </p>
      <HeroCaption report={report} metric={metric} className="mt-2" />
    </section>

    {harnesses.length > 0 && <section aria-label="By harness">
      <div className="flex h-2.5 gap-[3px]" role="img" aria-label={`Share of ${metric === "cost" ? "cost" : "processed tokens"}: ${harnesses.map(entry => `${harnessLabel(entry.harness)} ${formatPercent(metric === "cost" ? entry.costShare : entry.tokenShare)}`).join(", ")}`}>
        {harnesses.filter(entry => harnessValue(entry, metric) > 0).map(entry => <span key={entry.harness} className={cn("rounded-[3px]", harnessChartDot(entry.harness))} style={{ flexGrow: harnessValue(entry, metric) }} />)}
      </div>
      <ul className="mt-4 grid grid-cols-2 gap-x-6 gap-y-4 md:grid-cols-4">
        {harnesses.map(entry => <li key={entry.harness}>
          <span className="flex items-center gap-2 text-ui text-muted-foreground"><span className={cn("size-2 shrink-0 rounded-full", harnessChartDot(entry.harness))} aria-hidden="true" /><HarnessName harness={entry.harness} size={12} /></span>
          <span className="mt-1.5 block text-2xl tracking-tight tabular-nums text-foreground">{format(harnessValue(entry, metric))}</span>
          <span className="block text-caption tabular-nums text-muted-foreground">{formatPercent(metric === "cost" ? entry.costShare : entry.tokenShare)} · {metric === "cost" ? `${formatTokens(entry.processedTokens)} tokens` : formatUsd(entry.costMicrousd)}</span>
        </li>)}
      </ul>
    </section>}

    <section aria-label="Itemised by model">
      <div className={cn(ROW, "border-b border-border pb-2.5")}>
        <h2 className={EYEBROW}>Itemised by model</h2>
        <span className={cn(EYEBROW, "text-right")}>Tokens</span>
        <span className={cn(EYEBROW, "text-right")}>Cost</span>
        <span className={cn(EYEBROW, "text-right")}>Trend</span>
      </div>
      {harnesses.length === 0 && <p className="py-8 text-center text-caption text-muted-foreground">No activity in this window.</p>}
      {harnesses.map(entry => <div key={entry.harness} className="border-b border-border/60 py-2">
        <div className={cn(ROW, "h-9")}>
          <span className="flex min-w-0 items-center gap-2.5 text-ui font-medium text-foreground"><span className={cn("size-2 shrink-0 rounded-full", harnessChartDot(entry.harness))} aria-hidden="true" /><HarnessName harness={entry.harness} /></span>
          <span className="text-right text-ui tabular-nums text-foreground">{formatTokens(entry.processedTokens)}</span>
          <span className="text-right text-ui tabular-nums text-foreground">{formatUsd(entry.costMicrousd)}</span>
          <span className="flex justify-end"><Sparkline values={report.periods.map(period => (metric === "cost" ? period.costByHarness[entry.harness] : period.tokensByHarness[entry.harness]) ?? 0)} harness={entry.harness} area /></span>
        </div>
        {modelsOf(report, entry.harness, metric).map(model => <div key={model.model} className={cn(ROW, "h-7 pl-[1.125rem]")}>
          <span className="flex min-w-0 items-center">
            <span className="truncate font-mono text-caption text-foreground/80">{model.model}</span>
            {model.costSource === "unpriced" && <span className="ml-2 shrink-0 text-[11px] text-muted-foreground">unpriced</span>}
            <span className="mx-3 h-px min-w-4 flex-1 translate-y-1 border-b border-dotted border-border" aria-hidden="true" />
          </span>
          <span className="text-right text-ui tabular-nums text-muted-foreground">{formatTokens(model.tokens)}</span>
          <span className="text-right text-ui tabular-nums text-muted-foreground">{formatUsd(model.costMicrousd)}</span>
          <span className="flex justify-end"><Sparkline values={modelSeries(model, metric)} harness={entry.harness} className="h-4" /></span>
        </div>)}
      </div>)}
      {harnesses.length > 0 && <div className={cn(ROW, "h-12 border-t border-border text-[14px] font-medium text-foreground")}>
        <span>Total</span>
        <span className="text-right tabular-nums">{formatTokens(report.totals.processedTokens)}</span>
        <span className="text-right tabular-nums">{formatUsd(report.totals.costMicrousd)}<EstimateMark report={report} /></span>
        <span className="flex justify-end"><Sparkline values={periodTotals} area /></span>
      </div>}
    </section>

    <TokenComposition report={report} />
  </div>;
}
