import { useEffect, useId, useMemo, useState } from "react";
import { cn } from "@/lib/utils";
import { bridgeApi } from "../api";
import type { UsageScreenTimeResult } from "../types";
import { axisLabelIndices, formatDayShort, niceScale, seriesPaths, type ChartGeometry } from "../usageReport";
import {
  formatDuration, formatLeverage, readScreenTimePreferences, SCREEN_TIME_PREFERENCES_EVENT, screenTimeParams, summarizeScreenTime, writeScreenTimePreferences,
  type ScreenTimeRange,
} from "../screenTime";
import { LAYOUT_CARD, UsageLoadingState, useScrub } from "./UsageLayoutParts";
import { Switch } from "./settings/kit";

// The Time tab: Screen Time for Bridge. Your time is minutes the window was
// focused and in use; agent time is minutes your chats spent working, added
// across chats. Rules are in testing/feat-usage-screen-time.md.

/** How often the tab asks again while agent history is still being counted. */
export const COUNTING_POLL_MS = 3_000;

const RANGE_LABELS: Record<ScreenTimeRange, string> = { 7: "Week", 30: "Month" };
const RANGE_TOTALS: Record<ScreenTimeRange, string> = { 7: "Last 7 days", 30: "Last 30 days" };
const CUMULATIVE: ChartGeometry = { width: 600, height: 88, plotTop: 6 };
const YOU = "bg-foreground/80";
const AGENTS = "bg-foreground/25";

// Kept across visits so reopening the tab draws at once and refetches in place.
const cache = new Map<ScreenTimeRange, UsageScreenTimeResult>();

export function clearScreenTimeCacheForTests(): void {
  cache.clear();
}

function weekday(day: string): string {
  return new Intl.DateTimeFormat("en-US", { weekday: "short", timeZone: "UTC" }).format(new Date(`${day}T12:00:00Z`));
}

function dayLabel(day: string, today: string): string {
  return day === today ? "Today" : `${weekday(day)}, ${formatDayShort(day)}`;
}

function axisHours(hours: number): string {
  return hours >= 1 && Number.isInteger(hours) ? `${hours}h` : formatDuration(hours * 3_600);
}

function Legend() {
  return <span className="inline-flex items-center gap-3 text-caption text-muted-foreground">
    <span className="inline-flex items-center gap-1.5"><span className={cn("size-2 rounded-xs", YOU)} aria-hidden="true" />You</span>
    <span className="inline-flex items-center gap-1.5"><span className={cn("size-2 rounded-xs", AGENTS)} aria-hidden="true" />Agents</span>
  </span>;
}

function DailyChart({ result, range }: { result: UsageScreenTimeResult; range: ScreenTimeRange }) {
  const days = result.days;
  const scrub = useScrub(days.length);
  const peak = Math.max(0, ...days.map(day => Math.max(day.humanSeconds, day.agentSeconds)));
  const scale = niceScale(peak / 3_600, 3);
  const maxSeconds = scale.max * 3_600;
  const height = (seconds: number) => `${maxSeconds > 0 ? (seconds / maxSeconds) * 100 : 0}%`;
  const today = result.untilDay;
  const labelled = new Set(range === 7 ? days.map((_, index) => index) : axisLabelIndices(days.length));
  const active = scrub.index !== null ? days[scrub.index] : null;
  const humanTotal = days.reduce((sum, day) => sum + day.humanSeconds, 0);
  const agentTotal = days.reduce((sum, day) => sum + day.agentSeconds, 0);

  return <section aria-label="Daily time" className={LAYOUT_CARD}>
    <div className="mb-3 flex flex-wrap items-center gap-x-4 gap-y-1">
      <h2 className="text-ui font-medium text-foreground">Daily time</h2>
      <Legend />
      <p role="status" aria-live="polite" className="ml-auto text-caption tabular-nums text-muted-foreground">
        {active
          ? <>{dayLabel(active.day, today)}: you <span className="text-foreground">{formatDuration(active.humanSeconds)}</span> · agents <span className="text-foreground">{formatDuration(active.agentSeconds)}</span></>
          : <>{RANGE_TOTALS[range]}: you <span className="text-foreground">{formatDuration(humanTotal)}</span> · agents <span className="text-foreground">{formatDuration(agentTotal)}</span></>}
      </p>
    </div>
    {peak === 0 ? <p className="py-10 text-center text-caption text-muted-foreground">No time recorded in this range.</p> : <div
      role="group"
      tabIndex={0}
      aria-label="Your time and agent time per day. Use the arrow keys to read one day."
      onKeyDown={scrub.onKeyDown}
      onBlur={scrub.clear}
      onPointerLeave={scrub.clear}
      className="grid grid-cols-[2.75rem_minmax(0,1fr)] gap-x-2 rounded-lg outline-none focus-visible:ring-2 focus-visible:ring-ring"
    >
      <div className="relative h-44" aria-hidden="true">
        {scale.ticks.map(tick => <span key={tick} className="absolute right-0 translate-y-1/2 text-caption tabular-nums text-muted-foreground" style={{ bottom: `${scale.max > 0 ? (tick / scale.max) * 100 : 0}%` }}>{axisHours(tick)}</span>)}
      </div>
      <div className="relative h-44">
        {scale.ticks.map(tick => <span key={tick} className="absolute inset-x-0 border-t border-border" style={{ bottom: `${scale.max > 0 ? (tick / scale.max) * 100 : 0}%` }} aria-hidden="true" />)}
        <div className="absolute inset-0 flex items-end">
          {days.map((day, index) => <div
            key={day.day}
            data-day={day.day}
            onPointerEnter={() => scrub.setIndex(index)}
            className={cn("flex h-full min-w-0 flex-1 items-end justify-center gap-0.5 rounded-sm px-0.5", scrub.index === index && "bg-accent/70")}
          >
            <span data-series="you" className={cn("w-full max-w-3.5 rounded-t-xs motion-safe:animate-chart-rise origin-bottom", YOU)} style={{ height: height(day.humanSeconds) }} />
            <span data-series="agents" className={cn("w-full max-w-3.5 rounded-t-xs motion-safe:animate-chart-rise origin-bottom", AGENTS)} style={{ height: height(day.agentSeconds) }} />
          </div>)}
        </div>
      </div>
      <span />
      <div className="relative mt-1.5 h-4" aria-hidden="true">
        {days.map((day, index) => labelled.has(index) && <span
          key={day.day}
          className={cn(
            "absolute top-0 whitespace-nowrap text-caption tabular-nums text-muted-foreground",
            // A week centres a label under each day; a month pins its end labels to the edges.
            range === 30 && index === 0 ? "" : range === 30 && index === days.length - 1 ? "-translate-x-full" : "-translate-x-1/2",
            day.day === today && "text-foreground",
          )}
          style={{ left: `${range === 30 && index === 0 ? 0 : range === 30 && index === days.length - 1 ? 100 : ((index + 0.5) / days.length) * 100}%` }}
        >{range === 7 ? (day.day === today ? "Today" : weekday(day.day)) : day.day === today ? "Today" : formatDayShort(day.day)}</span>)}
      </div>
    </div>}
  </section>;
}

function CumulativeChart({ result, cumulative }: { result: UsageScreenTimeResult; cumulative: number[] }) {
  const gradient = `${useId().replace(/:/g, "")}-cumulative`;
  const start = Math.max(0, (cumulative[0] ?? 0) - (result.days[0]?.humanSeconds ?? 0));
  const end = cumulative[cumulative.length - 1] ?? 0;
  // Drawn from where the range began, so the growth inside it reads; both
  // ends are labelled with their real totals.
  const paths = seriesPaths(cumulative.map(value => value - start), end - start, CUMULATIVE);
  return <section aria-label="Your total time" className={LAYOUT_CARD}>
    <div className="mb-3 flex flex-wrap items-baseline gap-x-3 gap-y-1">
      <h2 className="text-ui font-medium text-foreground">Your total time</h2>
      <span className="text-caption text-muted-foreground">all time, at the end of each day</span>
      <span data-testid="screen-time-all-time" className="ml-auto text-body tabular-nums text-foreground">{formatDuration(result.allTime.humanSeconds)}</span>
    </div>
    {end === 0 ? <p className="py-6 text-center text-caption text-muted-foreground">Your total starts with the first minute you spend in Bridge.</p> : <>
      <div className="relative h-[88px]">
        <svg viewBox={`0 0 ${CUMULATIVE.width} ${CUMULATIVE.height}`} preserveAspectRatio="none" className="block h-full w-full overflow-visible text-foreground" aria-hidden="true">
          <defs><linearGradient id={gradient} x1="0" y1="0" x2="0" y2="1"><stop offset="0" stopColor="currentColor" stopOpacity={0.22} /><stop offset="1" stopColor="currentColor" stopOpacity={0.02} /></linearGradient></defs>
          <line x1={0} x2={CUMULATIVE.width} y1={CUMULATIVE.height} y2={CUMULATIVE.height} className="stroke-border" vectorEffect="non-scaling-stroke" />
          {end > start && <path d={paths.area} fill={`url(#${gradient})`} />}
          <path d={end > start ? paths.line : `M0,${CUMULATIVE.height} L${CUMULATIVE.width},${CUMULATIVE.height}`} fill="none" stroke="currentColor" strokeWidth={1.5} strokeLinejoin="round" vectorEffect="non-scaling-stroke" />
        </svg>
      </div>
      <div className="mt-1.5 flex justify-between text-caption tabular-nums text-muted-foreground">
        <span>{formatDayShort(result.sinceDay)} · {formatDuration(start)} before</span>
        <span className="text-foreground">Today · {formatDuration(end)}</span>
      </div>
    </>}
  </section>;
}

export function UsageScreenTime({ onError, refreshKey = 0 }: { onError: (message: string) => void; refreshKey?: number }) {
  const [range, setRange] = useState<ScreenTimeRange>(7);
  const [preferences, setPreferences] = useState(() => readScreenTimePreferences());
  const [shown, setShown] = useState<{ range: ScreenTimeRange; result: UsageScreenTimeResult } | null>(() => {
    const cached = cache.get(7);
    return cached ? { range: 7, result: cached } : null;
  });
  const [loading, setLoading] = useState(true);
  const [poll, setPoll] = useState(0);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    const cached = cache.get(range);
    if (cached) setShown({ range, result: cached });
    bridgeApi.screenTime(screenTimeParams(range))
      .then(result => {
        if (cancelled) return;
        cache.set(range, result);
        setShown({ range, result });
      })
      .catch(error => { if (!cancelled) onError(error instanceof Error ? error.message : String(error)); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [range, refreshKey, poll, onError]);

  const counting = shown?.result.agentHistory === "counting";
  useEffect(() => {
    if (!counting) return;
    const timer = setTimeout(() => setPoll(tick => tick + 1), COUNTING_POLL_MS);
    return () => clearTimeout(timer);
  }, [counting, shown]);

  useEffect(() => {
    const sync = () => setPreferences(readScreenTimePreferences());
    window.addEventListener(SCREEN_TIME_PREFERENCES_EVENT, sync);
    return () => window.removeEventListener(SCREEN_TIME_PREFERENCES_EVENT, sync);
  }, []);

  const result = shown?.result ?? null;
  const summary = useMemo(() => result ? summarizeScreenTime(result) : null, [result]);
  const stale = shown !== null && shown.range !== range;
  const shownRange = shown?.range ?? range;
  const setTracking = (track: boolean) => {
    writeScreenTimePreferences({ track });
    setPreferences({ track });
  };

  return <div className="space-y-5">
    <div className="flex flex-wrap items-center gap-3">
      <div role="radiogroup" aria-label="Range" className="u-segmented">
        {([7, 30] as const).map(option => <button key={option} type="button" role="radio" aria-checked={range === option} data-active={range === option} className="u-segmented-item" onClick={() => setRange(option)}>{RANGE_LABELS[option]}</button>)}
      </div>
      {result && !stale && <span className="text-caption tabular-nums text-muted-foreground">{formatDayShort(result.sinceDay)} to {formatDayShort(result.untilDay)}</span>}
      <label className="ml-auto inline-flex items-center gap-2 text-caption text-muted-foreground">
        Track my time
        <Switch checked={preferences.track} onChange={setTracking} label="Track my time" />
      </label>
    </div>

    {result && <ul aria-label="Screen Time notes" className="space-y-1 text-caption text-muted-foreground empty:hidden">
      {!preferences.track && <li>Your time is not being recorded. Agent time still counts.</li>}
      {preferences.track && !result.allTime.humanSinceDay && <li>Bridge counts your time from now on. Agent time includes your earlier chats.</li>}
      {counting && <li role="status">Still counting agent time from before Screen Time. The figures grow until it finishes.</li>}
    </ul>}

    {!result || !summary ? loading ? <UsageLoadingState /> : null : <div aria-busy={loading} className={cn("space-y-5 transition-opacity", stale && "opacity-50")}>
      <section aria-label="Summary" className="flex flex-wrap items-end justify-between gap-6">
        <div>
          <div data-testid="screen-time-today" className="text-5xl font-light leading-none tracking-display tabular-nums text-foreground">{formatDuration(summary.today.humanSeconds)}</div>
          <p className="mt-2.5 text-body text-muted-foreground">in Bridge today · agents worked <span className="font-medium tabular-nums text-foreground">{formatDuration(summary.today.agentSeconds)}</span></p>
        </div>
        <dl className="flex flex-wrap divide-x divide-border">
          {[
            { label: "Daily average", value: formatDuration(summary.dailyAverageSeconds), detail: "your time" },
            { label: RANGE_TOTALS[shownRange], value: formatDuration(summary.humanSeconds), detail: "your time" },
            { label: "All time", value: formatDuration(result.allTime.humanSeconds), detail: result.allTime.humanSinceDay ? `since ${formatDayShort(result.allTime.humanSinceDay)}` : "starting now" },
            { label: "Agent time", value: formatDuration(summary.agentSeconds), detail: summary.leverage !== null ? `${formatLeverage(summary.leverage)} yours` : RANGE_TOTALS[shownRange].toLowerCase() },
          ].map(tile => <div key={tile.label} className="px-5 first:pl-0 last:pr-0">
            <dt className="text-caption text-muted-foreground">{tile.label}</dt>
            <dd className="mt-1 text-lg tracking-tight tabular-nums text-foreground">{tile.value}</dd>
            <dd className="text-caption tabular-nums text-muted-foreground">{tile.detail}</dd>
          </div>)}
        </dl>
      </section>
      <DailyChart result={result} range={shownRange} />
      <CumulativeChart result={result} cumulative={summary.cumulative} />
      <p className="text-caption text-muted-foreground">Your time counts whole minutes the Bridge window was in front and in use, pausing after 5 idle minutes. Agent time adds up every chat and worker, so agents can outpace the clock. Both stay on this Mac.</p>
    </div>}
  </div>;
}
