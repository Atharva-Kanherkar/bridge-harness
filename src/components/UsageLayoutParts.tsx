import { useCallback, useState } from "react";
import { cn } from "@/lib/utils";
import { harnessLabel } from "../utils";
import type { UsageSummaryResult } from "../types";
import { HarnessMark, harnessChartFill, harnessChartStroke } from "./harnessMarks";
import { costSourceLabel, formatCount, formatPercent, formatTokens, formatUsd, type UsageMetric, type UsageReport, type UsageWindow, type UsageWindowDays } from "../usageReport";
import { axisFraction, evenTicks, TOKEN_KINDS, type TokenKind } from "../usageGeometry";

// Pieces every Usage layout shares, so the honesty rules are written once:
// the partial badge, the `API estimate` caption, the `~` on a partly unpriced
// cost, reasoning stated inside output, and keyboard scrubbing for charts.

export interface UsageLayoutProps {
  report: UsageReport;
  summary: UsageSummaryResult;
  periods: string[];
  window: UsageWindow;
  windowDays: UsageWindowDays;
  metric: UsageMetric;
  /** Why the headline is not final yet, or null when it is. */
  partial: string | null;
}

export const LAYOUT_CARD = "u-surface rounded-2xl p-5";
export const EYEBROW = "text-[11px] font-medium uppercase tracking-[0.08em] text-muted-foreground";

/** Achromatic ramp for the four parts of processed tokens: colour stays reserved for harnesses. */
export const KIND_SHADE: Record<TokenKind, { bg: string; opacity: number }> = {
  cacheReadTokens: { bg: "bg-foreground/80", opacity: 0.8 },
  cacheWriteTokens: { bg: "bg-foreground/50", opacity: 0.5 },
  uncachedInputTokens: { bg: "bg-foreground/30", opacity: 0.3 },
  outputTokens: { bg: "bg-foreground/20", opacity: 0.2 },
};

export function formatMetric(metric: UsageMetric): (value: number) => string {
  return metric === "cost" ? formatUsd : formatTokens;
}

export function windowTitle(days: UsageWindowDays): string {
  return days === 1 ? "Last 24 hours" : `Last ${days} days`;
}

export function periodNoun(window: UsageWindow): string {
  return window.resolution === "hour" ? "Hourly" : "Daily";
}

export function PartialBadge({ label, className }: { label: string | null; className?: string }) {
  if (!label) return null;
  return <span role="status" className={cn("inline-flex items-center gap-1.5 rounded-full border border-border bg-muted/40 px-2 py-1 text-[11px] text-muted-foreground", className)}>
    <span className="size-1.5 rounded-full bg-muted-foreground/50" aria-hidden="true" />Partial total · {label}
  </span>;
}

/** `~` after a cost whose window holds unpriced records: the figure is a floor, not a total. */
export function EstimateMark({ report, large }: { report: UsageReport; large?: boolean }) {
  if (report.costSource !== "unpriced") return null;
  return <span className={cn("font-normal text-muted-foreground", large && "ml-1 align-top text-[0.42em] tracking-normal")} title="Partly unpriced: some records have no known rate">~</span>;
}

/** Request count and, in cost mode, `API estimate` plus the window's weakest provenance. */
export function HeroCaption({ report, metric, className }: { report: UsageReport; metric: UsageMetric; className?: string }) {
  return <p className={cn("text-caption tabular-nums text-muted-foreground", className)}>
    {formatCount(report.totals.records)} requests{metric === "cost" ? ` · API estimate · ${costSourceLabel(report.costSource)}` : " · processed tokens"}
  </p>;
}

export function HarnessName({ harness, size = 13, className }: { harness: string; size?: number; className?: string }) {
  return <span className={cn("inline-flex min-w-0 items-center gap-2", className)}>
    <HarnessMark harness={harness} size={size} /><span className="truncate">{harnessLabel(harness)}</span>
  </span>;
}

/** A straight-segment trend scaled to its own peak. Decorative: the row it sits in carries the numbers. */
export function Sparkline({ values, harness, area, className }: { values: readonly number[]; harness?: string; area?: boolean; className?: string }) {
  const peak = Math.max(0, ...values);
  const width = 100;
  const height = 24;
  const points = values.map((value, index) => `${values.length <= 1 ? 0 : (index / (values.length - 1)) * width},${peak > 0 ? height - 1 - (value / peak) * (height - 3) : height - 1}`);
  const line = points.length ? `M${points.join(" L")}` : "";
  const stroke = harness ? harnessChartStroke(harness) : "stroke-foreground";
  return <svg viewBox={`0 0 ${width} ${height}`} preserveAspectRatio="none" className={cn("block h-5 w-24 overflow-visible", className)} aria-hidden="true">
    {area && line && <path d={`${line} L${width},${height} L0,${height} Z`} className={cn(harness ? harnessChartFill(harness) : "fill-foreground", "opacity-15")} />}
    {line && <path d={line} fill="none" className={stroke} strokeWidth={1.25} strokeLinejoin="round" vectorEffect="non-scaling-stroke" />}
  </svg>;
}

/** Where the processed tokens went, plus cache savings and reasoning stated as part of output. */
export function TokenComposition({ report, className }: { report: UsageReport; className?: string }) {
  const total = report.totals.processedTokens;
  return <section aria-label="Token composition" className={className}>
    <h2 className={EYEBROW}>Where the tokens went</h2>
    <div className="mt-3.5 flex h-2 gap-0.5" role="img" aria-label={TOKEN_KINDS.map(kind => `${kind.label} ${formatTokens(report.totals[kind.key])}`).join(", ")}>
      {total > 0 ? TOKEN_KINDS.filter(kind => report.totals[kind.key] > 0).map(kind => <span key={kind.key} className={cn("min-w-1 rounded-[2px]", KIND_SHADE[kind.key].bg)} style={{ flexGrow: report.totals[kind.key] }} />) : <span className="flex-1 rounded-[2px] bg-muted" />}
    </div>
    <dl className="mt-3 grid grid-cols-2 gap-x-6 gap-y-2 sm:grid-cols-4">
      {TOKEN_KINDS.map(kind => <div key={kind.key} className="flex items-baseline gap-2 text-ui">
        <span className={cn("size-2 shrink-0 translate-y-[-1px] rounded-[2px]", KIND_SHADE[kind.key].bg)} aria-hidden="true" />
        <dt className="text-muted-foreground">{kind.label}</dt>
        <dd className="font-medium tabular-nums text-foreground">{formatTokens(report.totals[kind.key])}</dd>
        <dd className="text-caption tabular-nums text-muted-foreground/70">{formatPercent(total > 0 ? report.totals[kind.key] / total : 0)}</dd>
      </div>)}
    </dl>
    <p className="mt-4 text-ui text-muted-foreground">
      Cache reuse saved <span className="font-medium tabular-nums text-foreground">{formatUsd(report.totals.cacheSavingsMicrousd)}</span> compared with paying uncached input for the same tokens. Reasoning ({formatTokens(report.totals.reasoningTokens)}) is counted inside output, never added to it.
    </p>
  </section>;
}

/** Hover or arrow-key position along a dense period axis. */
export function useScrub(count: number) {
  const [index, setIndex] = useState<number | null>(null);
  const clamp = useCallback((value: number) => Math.min(count - 1, Math.max(0, value)), [count]);
  const fromPointer = useCallback((event: React.PointerEvent<HTMLElement> | React.MouseEvent<HTMLElement>) => {
    const rect = event.currentTarget.getBoundingClientRect();
    if (rect.width === 0 || count === 0) return;
    const fraction = (event.clientX - rect.left) / rect.width;
    setIndex(count <= 1 ? 0 : clamp(Math.round(fraction * (count - 1))));
  }, [count, clamp]);
  const onKeyDown = useCallback((event: React.KeyboardEvent<HTMLElement>) => {
    if (count === 0) return;
    const moves: Record<string, (current: number) => number> = {
      ArrowLeft: current => current - 1,
      ArrowRight: current => current + 1,
      Home: () => 0,
      End: () => count - 1,
    };
    const move = moves[event.key];
    if (!move) return;
    event.preventDefault();
    setIndex(current => clamp(move(current ?? count - 1)));
  }, [count, clamp]);
  const active = index !== null && index < count ? index : null;
  return { index: active, setIndex, fromPointer, onKeyDown, clear: () => setIndex(null) };
}

/** Evenly spaced period labels. `slots` centres them under bars; otherwise they sit on the points of a line. */
export function AxisLabels({ count, label, slots, className }: { count: number; label: (index: number) => string; slots?: boolean; className?: string }) {
  return <div className={cn("relative h-4 text-[11px] tabular-nums text-muted-foreground/70", className)} aria-hidden="true">
    {evenTicks(count).map(tick => {
      if (count === 1) return <span key={tick} className="absolute left-1/2 -translate-x-1/2 whitespace-nowrap">{label(tick)}</span>;
      const first = tick === 0;
      const last = tick === count - 1;
      const left = first ? 0 : last ? 1 : slots ? (tick + 0.5) / count : axisFraction(tick, count);
      return <span key={tick} className={cn("absolute whitespace-nowrap", first ? "" : last ? "-translate-x-full" : "-translate-x-1/2")} style={{ left: `${left * 100}%` }}>{label(tick)}</span>;
    })}
  </div>;
}
