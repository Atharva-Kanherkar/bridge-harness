import { useEffect, useMemo, useRef, useState } from "react";
import { cn } from "@/lib/utils";
import { harnessLabel } from "../utils";
import { harnessChartDot } from "./harnessMarks";
import { heatStep } from "./UsageHeatmap";
import { formatDayShort, formatHourShort, formatPercent, formatPeriodLabel, formatTokens, formatUsd } from "../usageReport";
import { calendarWeeks, harnessesByMetric, harnessValue, modelValue, orderedRange, periodValue, scopeReport } from "../usageGeometry";
import { EstimateMark, formatMetric, HarnessName, HeroCaption, LAYOUT_CARD, PartialBadge, type UsageLayoutProps } from "./UsageLayoutParts";

// Layout E: time is the navigation. The calendar is the hero; clicking a day
// selects it, and shift-click or a drag extends the range. Everything under
// the calendar is rebuilt from the buckets in that range, so a selection is
// a real sub-total, never an interpolation. Depth is achromatic; the thin bar
// in each cell is the only colour, and it says which harnesses did the work.

const WEEKDAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
/** Cell depth by share of the window's peak; step 0 is a day with nothing in it. */
const DEPTH = ["bg-muted/40", "bg-foreground/[0.05]", "bg-foreground/[0.09]", "bg-foreground/[0.13]", "bg-foreground/[0.18]"];

export function UsageCalendar({ report, summary, periods, window, metric, partial }: UsageLayoutProps) {
  const format = formatMetric(metric);
  const hourly = window.resolution === "hour";
  const drag = useRef<{ anchor: number; moved: boolean } | null>(null);
  // A selection belongs to the window it was made in: a new window would read
  // its indices as other days, so it is ignored rather than carried over.
  const windowKey = `${periods[0] ?? ""}:${periods.length}`;
  const [selection, setSelection] = useState<{ key: string; anchor: number; range: [number, number] } | null>(null);
  const active = selection?.key === windowKey ? selection : null;
  const anchor = active?.anchor ?? null;
  const range = active?.range ?? null;
  useEffect(() => {
    // Cleared after the click that follows the pointerup, so that click can tell a drag from a tap.
    const release = () => { setTimeout(() => { drag.current = null; }, 0); };
    document.addEventListener("pointerup", release);
    return () => document.removeEventListener("pointerup", release);
  }, []);

  const values = report.periods.slice(0, periods.length).map(period => periodValue(period, metric));
  const peak = Math.max(0, ...values);
  const harnesses = harnessesByMetric(report, metric);
  const scoped = useMemo(() => scopeReport(summary, periods, range), [summary, periods, range]);
  const scopedHarnesses = harnessesByMetric(scoped.report, metric);
  const scopedTotal = metric === "cost" ? scoped.report.totals.costMicrousd : scoped.report.totals.processedTokens;
  const windowTotal = metric === "cost" ? report.totals.costMicrousd : report.totals.processedTokens;
  const topModels = [...scoped.report.models].sort((a, b) => modelValue(b, metric) - modelValue(a, metric)).slice(0, 6);
  const topModel = topModels[0] ? modelValue(topModels[0], metric) : 0;
  const weeks = useMemo(() => (hourly ? [] : calendarWeeks(periods)), [hourly, periods]);
  const compact = weeks.length > 6;
  const inRange = (index: number) => range !== null && index >= range[0] && index <= range[1];
  const label = (index: number) => formatPeriodLabel(periods[index], window.resolution, window.timeZone);

  const select = (index: number, extend: boolean) => {
    const from = extend && anchor !== null ? anchor : index;
    setSelection({ key: windowKey, anchor: from, range: orderedRange(from, index) });
  };

  const cell = (index: number) => {
    const period = report.periods[index];
    const value = values[index] ?? 0;
    const step = heatStep(value, peak);
    const day = periods[index];
    const dayOfMonth = Number(day.slice(8, 10));
    const monthLabel = !hourly && (index === 0 || dayOfMonth === 1) ? formatDayShort(day).split(" ")[0] : null;
    const parts = harnesses.map(entry => ({ harness: entry.harness, value: metric === "cost" ? period.costByHarness[entry.harness] ?? 0 : period.tokensByHarness[entry.harness] ?? 0 })).filter(part => part.value > 0);
    return <button
      key={day}
      type="button"
      aria-pressed={inRange(index)}
      aria-label={`${hourly ? label(index) : formatDayShort(day)}: ${format(value)}`}
      onPointerDown={event => { if (!event.shiftKey) drag.current = { anchor: index, moved: false }; }}
      onPointerEnter={() => {
        const current = drag.current;
        if (!current || current.anchor === index) return;
        current.moved = true;
        setSelection({ key: windowKey, anchor: current.anchor, range: orderedRange(current.anchor, index) });
      }}
      onClick={event => {
        if (drag.current?.moved) { drag.current = null; return; }
        drag.current = null;
        select(index, event.shiftKey);
      }}
      className={cn(
        "relative flex select-none flex-col rounded-[10px] text-left outline-none transition-shadow focus-visible:ring-2 focus-visible:ring-ring",
        compact ? "h-11 p-1.5" : "h-[5.5rem] px-2.5 pb-2.5 pt-2",
        DEPTH[step],
        inRange(index) && "shadow-[inset_0_0_0_1.5px_var(--color-ring)]",
      )}
    >
      <span className="text-[11.5px] leading-none tabular-nums text-muted-foreground">{monthLabel && <span className="mr-1 font-medium text-foreground">{monthLabel}</span>}{hourly ? formatHourShort(day, window.timeZone) : dayOfMonth}</span>
      {!hourly && index === periods.length - 1 && <span className="absolute right-2.5 top-2.5 size-1.5 rounded-full bg-foreground" aria-hidden="true" />}
      {!compact && <span className={cn("mt-auto text-[15px] tracking-tight tabular-nums", value > 0 ? "text-foreground" : "text-muted-foreground/60")}>{value > 0 ? format(value) : "—"}</span>}
      <span className={cn("flex h-1 gap-[1.5px] overflow-hidden rounded-[2px]", compact ? "mt-auto" : "mt-1.5")} aria-hidden="true">
        {parts.map(part => <span key={part.harness} className={harnessChartDot(part.harness)} style={{ flexGrow: part.value }} />)}
      </span>
    </button>;
  };

  return <div className="space-y-4">
    <section aria-label={hourly ? "Hourly calendar" : "Daily calendar"} className={cn(LAYOUT_CARD, "p-4")}>
      {hourly ? <div className="grid grid-cols-6 gap-1.5 sm:grid-cols-12">
        {periods.map((_, index) => cell(index))}
      </div> : <div className={cn("grid gap-1.5", compact ? "grid-cols-7" : "grid-cols-[repeat(7,minmax(0,1fr))_5.5rem]")}>
        {WEEKDAYS.map(day => <div key={day} className="px-1 pb-1 text-[11px] uppercase tracking-[0.08em] text-muted-foreground/70">{day}</div>)}
        {!compact && <div className="px-2 pb-1 text-[11px] uppercase tracking-[0.08em] text-muted-foreground/70">Week</div>}
        {weeks.map((week, row) => {
          const weekTotal = week.reduce<number>((sum, slot) => sum + (slot === null ? 0 : values[slot] ?? 0), 0);
          return [
            ...week.map((slot, column) => (slot === null ? <div key={`empty-${row}-${column}`} aria-hidden="true" /> : cell(slot))),
            ...(compact ? [] : [<div key={`week-${row}`} className="flex flex-col justify-end border-l border-border pb-2.5 pl-2.5">
              <span className="text-[11px] text-muted-foreground/70">Week {row + 1}</span>
              <span className="text-ui tabular-nums text-muted-foreground">{format(weekTotal)}</span>
            </div>]),
          ];
        })}
      </div>}
      <div className="mt-3 flex flex-wrap items-center gap-x-4 gap-y-1 text-caption text-muted-foreground">
        <span className="inline-flex items-center gap-1">less{DEPTH.map(depth => <span key={depth} className={cn("size-3 rounded-[4px]", depth)} aria-hidden="true" />)}more</span>
        <span className="ml-auto">Bar = harness mix · Click {hourly ? "an hour" : "a day"}, shift-click or drag to select a range</span>
      </div>
    </section>

    <section aria-label="Selection" className={cn(LAYOUT_CARD, "grid gap-0 p-0 md:grid-cols-2")}>
      <div className="p-5">
        <div className="flex items-center gap-2.5">
          <span className={cn("rounded-full border px-2 py-0.5 text-[11.5px]", range ? "border-ring/40 text-ring" : "border-border text-muted-foreground")}>{range ? "Selected" : "Whole window"}</span>
          <span className="text-caption tabular-nums text-muted-foreground">{range ? `${label(range[0])}${range[1] !== range[0] ? ` – ${label(range[1])}` : ""}` : `${label(0)} – ${label(periods.length - 1)}`} · {scoped.periods.length} {hourly ? (scoped.periods.length === 1 ? "hour" : "hours") : scoped.periods.length === 1 ? "day" : "days"}</span>
          {range && <button type="button" onClick={() => setSelection(null)} className="ml-auto rounded-md px-2 py-0.5 text-caption text-muted-foreground hover:bg-accent hover:text-foreground">Clear</button>}
        </div>
        <PartialBadge label={partial} className="mt-4" />
        <div className="mt-4 text-[2.5rem] font-light leading-none tracking-[-0.04em] tabular-nums text-foreground">{format(scopedTotal)}{metric === "cost" && <EstimateMark report={scoped.report} large />}</div>
        <p className="mt-2 text-ui text-muted-foreground">{metric === "cost"
          ? <>at API rates · <span className="font-medium tabular-nums text-foreground">{formatTokens(scoped.report.totals.processedTokens)}</span> tokens</>
          : <>tokens · <span className="font-medium tabular-nums text-foreground">{formatUsd(scoped.report.totals.costMicrousd)}</span><EstimateMark report={scoped.report} /> at API rates</>}
          {range && windowTotal > 0 && <> · {formatPercent(scopedTotal / windowTotal, 0)} of the window</>}</p>
        <HeroCaption report={scoped.report} metric={metric} className="mt-0.5" />
        {scopedTotal > 0 && <>
          <div className="mt-5 flex h-2 gap-[3px]" aria-hidden="true">
            {scopedHarnesses.filter(entry => harnessValue(entry, metric) > 0).map(entry => <span key={entry.harness} className={cn("rounded-[2px]", harnessChartDot(entry.harness))} style={{ flexGrow: harnessValue(entry, metric) }} />)}
          </div>
          <ul className="mt-2.5 flex flex-wrap gap-x-4 gap-y-1.5 text-caption text-muted-foreground" aria-label="By harness">
            {scopedHarnesses.map(entry => <li key={entry.harness} className="inline-flex items-center gap-1.5"><span className={cn("size-2 rounded-full", harnessChartDot(entry.harness))} aria-hidden="true" /><HarnessName harness={entry.harness} size={11} /><span className="font-medium tabular-nums text-foreground">{format(harnessValue(entry, metric))}</span></li>)}
          </ul>
        </>}
      </div>
      <div className="border-t border-border p-5 md:border-l md:border-t-0">
        <h2 className="mb-2 text-ui font-medium text-foreground">Top models {range ? "in selection" : "in window"}</h2>
        {topModels.length === 0 ? <p className="py-6 text-caption text-muted-foreground">No activity in this {range ? "selection" : "window"}.</p> : <ul>
          {topModels.map(model => <li key={`${model.harness}:${model.model}`} className="grid h-8 grid-cols-[minmax(0,9rem)_minmax(0,1fr)_4.5rem_4.5rem] items-center gap-3">
            <span className="flex min-w-0 items-center gap-1.5"><span className="truncate font-mono text-caption text-foreground/85" title={`${harnessLabel(model.harness)} · ${model.model}`}>{model.model}</span>{model.costSource === "unpriced" && <span className="shrink-0 text-[11px] text-muted-foreground">unpriced</span>}</span>
            <span className="h-1.5 overflow-hidden rounded-full bg-muted"><span className={cn("block h-full rounded-full", harnessChartDot(model.harness))} style={{ width: `${topModel > 0 ? (modelValue(model, metric) / topModel) * 100 : 0}%` }} /></span>
            <span className="text-right text-ui tabular-nums text-foreground">{format(modelValue(model, metric))}</span>
            <span className="text-right text-caption tabular-nums text-muted-foreground">{metric === "cost" ? formatTokens(model.tokens) : formatUsd(model.costMicrousd)}</span>
          </li>)}
        </ul>}
      </div>
    </section>
  </div>;
}
