import { useEffect, useMemo, useRef, useState } from "react";
import { LoaderCircle, Sparkles } from "lucide-react";
import { cn } from "@/lib/utils";
import { bridgeApi } from "../api";
import { harnessLabel } from "../utils";
import type { UsageInsightsReport, UsageInsightsResult, UsageInsightTone } from "../types";
import { HarnessMark, harnessChartDot } from "./harnessMarks";
import { formatCount, formatTokens, formatUsd } from "../usageReport";

// The Insights tab: one harness turn over Bridge's own records, rendered as
// charts whose numbers Bridge computed and prose the model wrote. The two are
// kept visibly apart — figures on marks, words in cards — so a reader can
// always tell which is which. Colour follows the harness; everything else is
// the achromatic chrome. Motion is one reveal per card, never a loop.

const CARD = "u-surface rounded-2xl p-4";

const TONE: Record<UsageInsightTone, { dot: string; label: string }> = {
  neutral: { dot: "bg-muted-foreground/60", label: "Note" },
  good: { dot: "bg-success", label: "Good" },
  watch: { dot: "bg-foreground", label: "Watch" },
};

const LOADING_STEPS = ["Reading your usage ledger", "Sampling recent prompts", "Checking pull requests", "Asking your harness to write it up"];

function rise(index: number): { className: string; style: React.CSSProperties } {
  return { className: "motion-safe:animate-[insight-rise_420ms_ease-out_both]", style: { animationDelay: `${Math.min(index, 8) * 60}ms` } };
}

function relative(iso: string): string {
  const minutes = Math.round((Date.now() - Date.parse(iso)) / 60_000);
  if (!Number.isFinite(minutes) || minutes < 1) return "just now";
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} h ago`;
  return `${Math.round(hours / 24)} d ago`;
}

/** Horizontal bars, one per harness, measured from a shared zero. */
function HarnessBars({ report }: { report: UsageInsightsReport }) {
  const [hover, setHover] = useState<string | null>(null);
  const peak = Math.max(1, ...report.harnesses.map(entry => entry.processedTokens));
  return <ul className="space-y-2.5" aria-label="Processed tokens by harness">
    {report.harnesses.map((entry, index) => {
      const width = (entry.processedTokens / peak) * 100;
      return <li key={entry.harness} {...rise(index)} className={cn("relative", rise(index).className)} onMouseEnter={() => setHover(entry.harness)} onMouseLeave={() => setHover(null)}>
        <div className="mb-1 flex items-baseline justify-between gap-3 text-caption">
          <span className="inline-flex min-w-0 items-center gap-1.5 text-foreground"><span className={cn("size-2 shrink-0 rounded-[3px]", harnessChartDot(entry.harness))} aria-hidden="true" /><HarnessMark harness={entry.harness} size={12} /><span className="truncate">{harnessLabel(entry.harness)}</span></span>
          <span className="shrink-0 tabular-nums text-muted-foreground">{formatTokens(entry.processedTokens)}</span>
        </div>
        <div className="h-3 w-full overflow-hidden rounded-[4px] bg-muted/60">
          <div className={cn("h-full origin-left rounded-r-[4px] motion-safe:animate-[meter-fill_600ms_ease-out_both]", harnessChartDot(entry.harness))} style={{ width: `${Math.max(width, entry.processedTokens > 0 ? 1.5 : 0)}%` }} />
        </div>
        {hover === entry.harness && <div role="tooltip" className="u-glass-popover absolute right-0 top-full z-10 mt-1 rounded-lg px-2.5 py-1.5 text-[11px] leading-4 tabular-nums">
          <div className="text-foreground">{formatUsd(entry.costMicrousd)} at API rates</div>
          <div className="text-muted-foreground">{formatCount(entry.records)} requests · {formatCount(entry.sessions)} sessions · {formatCount(entry.prompts)} prompts</div>
        </div>}
      </li>;
    })}
    {report.harnesses.length === 0 && <li className="text-caption text-muted-foreground">No usage in this window.</li>}
  </ul>;
}

/** Twenty-four columns: prompts sent per local hour. One hue; the peak is labelled. */
function HourColumns({ report }: { report: UsageInsightsReport }) {
  const [hover, setHover] = useState<number | null>(null);
  const peak = Math.max(1, ...report.hours.map(entry => entry.prompts));
  const peakHour = report.hours.reduce((best, entry) => (entry.prompts > best.prompts ? entry : best), report.hours[0] ?? { hour: 0, prompts: 0 });
  const total = report.hours.reduce((sum, entry) => sum + entry.prompts, 0);
  return <figure aria-label="Prompts by hour of day">
    <div className="relative flex h-28 items-end gap-[2px]" onMouseLeave={() => setHover(null)}>
      {report.hours.map((entry, index) => <div key={entry.hour} className="group relative flex h-full flex-1 items-end" onMouseEnter={() => setHover(entry.hour)}>
        <div className={cn("w-full origin-bottom rounded-t-[3px] motion-safe:animate-[chart-rise_500ms_ease-out_both]", entry.hour === peakHour.hour && entry.prompts > 0 ? "bg-foreground" : "bg-foreground/35 group-hover:bg-foreground/60")} style={{ height: `${Math.max((entry.prompts / peak) * 100, entry.prompts > 0 ? 3 : 1)}%`, animationDelay: `${index * 15}ms` }} />
        {hover === entry.hour && <div role="tooltip" className={cn("u-glass-popover pointer-events-none absolute bottom-full z-10 mb-1 whitespace-nowrap rounded-lg px-2 py-1 text-[11px] tabular-nums", index > 15 ? "right-0" : "left-0")}>
          <span className="text-foreground">{formatCount(entry.prompts)} prompts</span> <span className="text-muted-foreground">at {String(entry.hour).padStart(2, "0")}:00</span>
        </div>}
      </div>)}
    </div>
    <div className="mt-1 flex justify-between text-[11px] tabular-nums text-muted-foreground" aria-hidden="true"><span>00</span><span>06</span><span>12</span><span>18</span><span>23</span></div>
    <figcaption className="mt-2 text-caption text-muted-foreground">{total === 0 ? "No prompts in this window." : <>Busiest at <span className="tabular-nums text-foreground">{String(peakHour.hour).padStart(2, "0")}:00</span>, {formatCount(peakHour.prompts)} of {formatCount(total)} prompts.</>}</figcaption>
  </figure>;
}

/** Processed tokens by day as a single area, with a crosshair on hover. */
function DayArea({ report }: { report: UsageInsightsReport }) {
  const plotRef = useRef<HTMLDivElement>(null);
  const [hover, setHover] = useState<number | null>(null);
  const days = report.days;
  const width = 600;
  const height = 120;
  const peak = Math.max(1, ...days.map(entry => entry.processedTokens));
  const points = useMemo(() => days.map((entry, index) => ({
    x: days.length <= 1 ? width / 2 : (index / (days.length - 1)) * width,
    y: height - (entry.processedTokens / peak) * (height - 8) - 4,
  })), [days, peak]);
  const line = points.map((point, index) => `${index === 0 ? "M" : "L"}${point.x.toFixed(1)} ${point.y.toFixed(1)}`).join(" ");
  const area = points.length ? `${line} L${points[points.length - 1].x.toFixed(1)} ${height} L${points[0].x.toFixed(1)} ${height} Z` : "";
  const onMove = (event: React.MouseEvent<HTMLDivElement>) => {
    const rect = plotRef.current?.getBoundingClientRect();
    if (!rect || rect.width === 0 || days.length === 0) return;
    setHover(Math.round(((event.clientX - rect.left) / rect.width) * (days.length - 1)));
  };
  const hovered = hover != null && hover >= 0 && hover < days.length ? hover : null;
  return <figure aria-label="Processed tokens by day">
    <div ref={plotRef} className="relative h-32 w-full" onMouseMove={onMove} onMouseLeave={() => setHover(null)}>
      <svg viewBox={`0 0 ${width} ${height}`} preserveAspectRatio="none" className="block h-full w-full overflow-visible text-foreground" role="img" aria-label={`${days.length} days of processed tokens`}>
        <line x1={0} x2={width} y1={height} y2={height} className="stroke-border" strokeWidth={1} vectorEffect="non-scaling-stroke" />
        {area && <path d={area} fill="currentColor" className="origin-bottom opacity-10 motion-safe:animate-[chart-rise_600ms_ease-out_both]" />}
        {line && <path d={line} fill="none" stroke="currentColor" strokeWidth={2} strokeLinejoin="round" strokeLinecap="round" vectorEffect="non-scaling-stroke" />}
        {hovered != null && <>
          <line x1={points[hovered].x} x2={points[hovered].x} y1={0} y2={height} className="stroke-foreground/40" strokeWidth={1} vectorEffect="non-scaling-stroke" />
          <circle cx={points[hovered].x} cy={points[hovered].y} r={4} fill="currentColor" className="stroke-background" strokeWidth={2} vectorEffect="non-scaling-stroke" />
        </>}
      </svg>
      {hovered != null && <div role="tooltip" className={cn("u-glass-popover pointer-events-none absolute top-1 z-10 rounded-lg px-2.5 py-1.5 text-[11px] leading-4 tabular-nums", (hovered / Math.max(1, days.length - 1)) > 0.6 ? "-translate-x-[calc(100%+10px)]" : "translate-x-2.5")} style={{ left: `${(hovered / Math.max(1, days.length - 1)) * 100}%` }}>
        <div className="text-foreground">{formatTokens(days[hovered].processedTokens)} tokens</div>
        <div className="text-muted-foreground">{days[hovered].day} · {formatCount(days[hovered].prompts)} prompts</div>
      </div>}
    </div>
    <div className="mt-1 flex justify-between text-[11px] tabular-nums text-muted-foreground" aria-hidden="true">
      <span>{days[0]?.day.slice(5) ?? ""}</span><span>{days[days.length - 1]?.day.slice(5) ?? ""}</span>
    </div>
  </figure>;
}

/** Themes as one segmented bar plus the list it indexes. Ordered by share, so
 * an ink ramp by rank reads as "bigger is darker" rather than as identity. */
function Themes({ report }: { report: UsageInsightsReport }) {
  const themes = [...report.themes].sort((a, b) => b.share - a.share);
  const shades = ["bg-foreground", "bg-foreground/75", "bg-foreground/55", "bg-foreground/40", "bg-foreground/28", "bg-foreground/18"];
  if (themes.length === 0) return <p className="text-caption text-muted-foreground">No themes were found in the sampled prompts.</p>;
  return <div>
    <div className="flex h-3 w-full gap-[2px] overflow-hidden rounded-[4px]" aria-hidden="true">
      {themes.map((theme, index) => <div key={theme.label} className={cn("h-full origin-left motion-safe:animate-[meter-fill_600ms_ease-out_both]", shades[Math.min(index, shades.length - 1)])} style={{ width: `${Math.max(theme.share * 100, 1)}%`, animationDelay: `${index * 60}ms` }} />)}
    </div>
    <ul className="mt-3 space-y-2" aria-label="Prompt themes">
      {themes.map((theme, index) => <li key={theme.label} className={cn("flex items-start justify-between gap-3", rise(index + 1).className)} style={rise(index + 1).style}>
        <span className="inline-flex min-w-0 items-start gap-2">
          <span className={cn("mt-1.5 size-2 shrink-0 rounded-[3px]", shades[Math.min(index, shades.length - 1)])} aria-hidden="true" />
          <span className="min-w-0">
            <span className="block text-ui text-foreground">{theme.label}</span>
            {theme.example && <span className="block text-caption text-muted-foreground">{theme.example}</span>}
          </span>
        </span>
        <span className="shrink-0 text-ui tabular-nums text-muted-foreground">{Math.round(theme.share * 100)}%</span>
      </li>)}
    </ul>
  </div>;
}

function Skeleton({ step }: { step: number }) {
  return <div role="status" aria-live="polite" className="space-y-4">
    <div className="flex items-center gap-2 text-caption text-muted-foreground"><LoaderCircle size={13} className="animate-spin" aria-hidden="true" />{LOADING_STEPS[Math.min(step, LOADING_STEPS.length - 1)]}…</div>
    <div className="grid gap-4 lg:grid-cols-3">
      {[0, 1, 2, 3, 4, 5].map(index => <div key={index} className={cn(CARD, "h-40 bg-[linear-gradient(100deg,transparent_30%,var(--color-muted)_50%,transparent_70%)] bg-[length:200%_100%] motion-safe:animate-[insight-shimmer_1.6s_linear_infinite]")} style={{ animationDelay: `${index * 120}ms` }} />)}
    </div>
  </div>;
}

export function UsageInsights({ windowDays, onError }: { windowDays: number; onError: (message: string) => void }) {
  const [result, setResult] = useState<UsageInsightsResult | null>(null);
  const [loading, setLoading] = useState(true);
  const [step, setStep] = useState(0);

  const load = (refresh: boolean) => {
    setLoading(true);
    setStep(0);
    const ticker = window.setInterval(() => setStep(current => current + 1), refresh ? 4_000 : 1_000);
    bridgeApi.usageInsights({ windowDays, refresh })
      .then(setResult)
      .catch(error => {
        const message = error instanceof Error ? error.message : String(error);
        setResult({ status: "failed", windowDays, detail: message });
        onError(message);
      })
      .finally(() => { window.clearInterval(ticker); setLoading(false); });
  };

  // The stored report first; running the model is always an explicit click.
  useEffect(() => { load(false); }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const report = result?.report ?? null;
  const analyseButton = <button type="button" onClick={() => load(true)} disabled={loading} className="inline-flex h-8 items-center gap-2 rounded-lg border border-border px-3 text-caption text-foreground transition-colors hover:bg-accent disabled:opacity-40">
    {loading ? <LoaderCircle size={13} className="animate-spin" aria-hidden="true" /> : <Sparkles size={13} aria-hidden="true" />}{report ? "Analyse again" : "Analyse my usage"}
  </button>;

  if (loading && !report) return <Skeleton step={step} />;

  if (!report) {
    const status = result?.status ?? "empty";
    return <div className={cn(CARD, "flex flex-col items-start gap-3 py-8")}>
      <h2 className="font-display text-lg font-semibold text-foreground">{status === "unavailable" ? "Insights need a harness" : status === "failed" ? "The analysis did not finish" : "Nothing analysed yet"}</h2>
      <p className="max-w-xl text-caption text-muted-foreground">{result?.detail ?? `Bridge reads your last ${windowDays} days of usage, a sample of recent prompts, and your open pull requests, then asks your harness to write up what it sees. Nothing is stored except the report.`}</p>
      {status !== "unavailable" && analyseButton}
    </div>;
  }

  return <div className="space-y-4">
    <section className={cn(CARD, rise(0).className)} style={rise(0).style} aria-label="Summary">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0 max-w-3xl">
          <h2 className="font-display text-2xl font-semibold tracking-tight text-foreground">{report.headline}</h2>
          <p className="mt-2 text-ui leading-relaxed text-muted-foreground">{report.summary}</p>
        </div>
        {analyseButton}
      </div>
      <p className="mt-3 text-[11px] text-muted-foreground">
        {result?.harness && result?.model ? <>Written by {harnessLabel(result.harness)} · <span className="font-mono">{result.model}</span> · </> : null}
        {result?.generatedAt ? `${relative(result.generatedAt)} · ` : ""}{result?.windowDays} day window · {formatCount(report.promptsAnalysed)} prompts sampled. Figures on the charts are Bridge's own; the words are the model's.
      </p>
    </section>

    {report.highlights.length > 0 && <section className="grid gap-3 md:grid-cols-3" aria-label="Highlights">
      {report.highlights.map((item, index) => <article key={item.title} className={cn(CARD, "py-3", rise(index + 1).className)} style={rise(index + 1).style}>
        <div className="mb-1 flex items-center gap-1.5 text-[11px] text-muted-foreground"><span className={cn("size-1.5 rounded-full", TONE[item.tone].dot)} aria-hidden="true" />{TONE[item.tone].label}</div>
        <h3 className="text-ui font-medium text-foreground">{item.title}</h3>
        <p className="mt-1 text-caption leading-relaxed text-muted-foreground">{item.detail}</p>
      </article>)}
    </section>}

    <section className="grid gap-4 lg:grid-cols-2" aria-label="Charts">
      <div className={cn(CARD, rise(3).className)} style={rise(3).style}>
        <h3 className="mb-3 text-ui font-medium text-foreground">Where the tokens went</h3>
        <HarnessBars report={report} />
      </div>
      <div className={cn(CARD, rise(4).className)} style={rise(4).style}>
        <h3 className="mb-3 text-ui font-medium text-foreground">When you prompt</h3>
        <HourColumns report={report} />
      </div>
      <div className={cn(CARD, rise(5).className)} style={rise(5).style}>
        <h3 className="mb-3 text-ui font-medium text-foreground">Day by day</h3>
        <DayArea report={report} />
      </div>
      <div className={cn(CARD, rise(6).className)} style={rise(6).style}>
        <h3 className="mb-3 text-ui font-medium text-foreground">What you ask for</h3>
        <Themes report={report} />
      </div>
    </section>

    <section className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(0,1.4fr)]" aria-label="GitHub and recommendations">
      <div className={cn(CARD, rise(7).className)} style={rise(7).style}>
        <h3 className="mb-3 text-ui font-medium text-foreground">Pull requests</h3>
        {report.github ? <dl className="grid grid-cols-2 gap-3">
          {[
            ["Open", report.github.openPrs],
            ["Drafts", report.github.draftPrs],
            ["Failing checks", report.github.failingChecks],
            ["Awaiting review", report.github.awaitingReview],
          ].map(([label, value]) => <div key={label as string}>
            <dt className="text-[11px] text-muted-foreground">{label}</dt>
            <dd className="font-display text-2xl font-semibold tabular-nums text-foreground">{formatCount(value as number)}</dd>
          </div>)}
          <p className="col-span-2 text-[11px] text-muted-foreground">Across {formatCount(report.github.repositories)} {report.github.repositories === 1 ? "repository" : "repositories"} Bridge has open.</p>
        </dl> : <p className="text-caption text-muted-foreground">GitHub was not available for this run. Install and sign in to the GitHub CLI to include pull requests.</p>}
      </div>
      <div className={cn(CARD, rise(8).className)} style={rise(8).style}>
        <h3 className="mb-3 text-ui font-medium text-foreground">Try next</h3>
        <ol className="space-y-2.5" aria-label="Recommendations">
          {report.recommendations.map((item, index) => <li key={item} className="flex gap-3 text-ui text-foreground"><span className="shrink-0 font-mono text-caption tabular-nums text-muted-foreground">{String(index + 1).padStart(2, "0")}</span><span className="leading-relaxed">{item}</span></li>)}
          {report.recommendations.length === 0 && <li className="text-caption text-muted-foreground">No recommendations this time.</li>}
        </ol>
      </div>
    </section>
  </div>;
}
