// The meter's readings, shared by the menu-bar panel and the chat's usage
// card so both show the same thing the same way: a ring gauge per provider
// for the window closest to biting, then one bar per quota window with its
// reset and its pace in words. Series colour follows the harness.
import { useEffect, useMemo, useState } from "react";
import { cn } from "@/lib/utils";
import { formatReset, windowLabel, type RateWindow, type UsageProvider, type UsageSnapshot } from "../../usage";
import { pacePhrase, paceVisible, paceWeekly } from "../../meter";
import { HarnessMark, harnessChartDot, harnessChartText } from "../harnessMarks";
import { harnessLabel } from "../../utils";

export const PROVIDER_ORDER: UsageProvider[] = ["codex", "claude", "cursor", "opencode"];

export function providerSnapshots(usage: Partial<Record<UsageProvider, UsageSnapshot>>): Array<{ provider: UsageProvider; snapshot: UsageSnapshot }> {
  return PROVIDER_ORDER.filter(provider => usage[provider]).map(provider => ({ provider, snapshot: usage[provider]! }));
}

const clampPercent = (value: number) => Math.min(100, Math.max(0, value));

/** The window a provider is closest to exhausting. */
export function headlineWindow(windows: RateWindow[]): RateWindow | undefined {
  return windows.reduce<RateWindow | undefined>((worst, window) => (worst == null || window.usedPercent > worst.usedPercent ? window : worst), undefined);
}

/** Countdowns and pace go stale against a frozen clock, so tick while shown. */
export function useMeterClock(): number {
  const [nowMs, setNowMs] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNowMs(Date.now()), 30_000);
    return () => window.clearInterval(timer);
  }, []);
  return nowMs;
}

/** A ring gauge: the headline window's usage as an arc in the harness colour. */
export function RingGauge({ provider, used, label }: { provider: UsageProvider; used: number; label: string }) {
  // Draw from empty on mount so the arc sweeps to its value once, then rests.
  const [drawn, setDrawn] = useState(false);
  useEffect(() => { const frame = window.requestAnimationFrame(() => setDrawn(true)); return () => window.cancelAnimationFrame(frame); }, []);
  const radius = 17;
  const circumference = 2 * Math.PI * radius;
  const offset = circumference * (1 - (drawn ? clampPercent(used) : 0) / 100);
  return <figure aria-label={`${harnessLabel(provider)} gauge`} className="flex min-w-0 flex-1 flex-col items-center gap-1">
    <svg viewBox="0 0 44 44" className={cn("size-14", harnessChartText(provider))} role="img" aria-label={`${Math.round(used)} percent of the ${label} window used`}>
      <circle cx="22" cy="22" r={radius} fill="none" className="stroke-border" strokeWidth={3} />
      <circle cx="22" cy="22" r={radius} fill="none" stroke="currentColor" strokeWidth={3} strokeLinecap="round" strokeDasharray={circumference} strokeDashoffset={offset} transform="rotate(-90 22 22)" className="transition-[stroke-dashoffset] duration-700 ease-out" />
      <text x="22" y="22" textAnchor="middle" dominantBaseline="central" className="fill-foreground font-sans text-[11px] font-semibold tabular-nums">{Math.round(used)}%</text>
    </svg>
    <figcaption className="flex items-center gap-1 text-[11px] text-muted-foreground">
      <HarnessMark harness={provider} size={10} />
      <span className="truncate">{harnessLabel(provider)} · {label}</span>
    </figcaption>
  </figure>;
}

export function WindowRow({ provider, window, nowMs }: { provider: UsageProvider; window: RateWindow; nowMs: number }) {
  const [expanded, setExpanded] = useState(false);
  const pace = useMemo(() => paceWeekly(window, nowMs), [window, nowMs]);
  const showPace = pace != null && paceVisible(window, nowMs);
  const reset = window.fresh ? "fresh window" : window.resetsInSeconds != null ? formatReset(window.resetsInSeconds) : window.resetsLabel;
  const used = clampPercent(window.usedPercent);
  const name = window.label || windowLabel(window.id, window.windowMinutes);
  return <div className="py-1.5">
    <div className="flex items-baseline justify-between gap-2 text-caption">
      <span className="font-medium text-foreground">{name}</span>
      <span className="shrink-0 tabular-nums text-muted-foreground">{Math.round(used)}% used</span>
    </div>
    <button type="button" aria-expanded={expanded} aria-controls={`meter-window-${provider}-${window.id}`} onClick={() => setExpanded(value => !value)} className="mt-1 block h-1.5 w-full overflow-hidden rounded-full bg-muted text-left outline-none transition-shadow focus-visible:ring-2 focus-visible:ring-ring">
      <span className={cn("block h-full origin-left rounded-full motion-safe:animate-[meter-fill_600ms_ease-out]", harnessChartDot(provider))} style={{ width: `${used}%` }} />
    </button>
    <div className="mt-1 flex flex-wrap gap-x-3 gap-y-0.5 text-[11px] text-muted-foreground">
      {reset && <span className="tabular-nums">{reset}</span>}
      {pace && showPace && <span>{pacePhrase(pace)}</span>}
    </div>
    {expanded && <p id={`meter-window-${provider}-${window.id}`} className="mt-1 text-[11px] text-muted-foreground">{Math.round(used)}% of this window is used{reset ? `. ${reset === "fresh window" ? "Nothing spent since it reset" : reset}.` : "."}</p>}
  </div>;
}

/** Gauges, then one section per live provider. Renders nothing for an empty map. */
export function MeterReadings({ usage, nowMs }: { usage: Partial<Record<UsageProvider, UsageSnapshot>>; nowMs: number }) {
  const live = providerSnapshots(usage);
  const gauges = live.map(({ provider, snapshot }) => ({ provider, window: headlineWindow(snapshot.windows) })).filter((entry): entry is { provider: UsageProvider; window: RateWindow } => entry.window != null);
  return <>
    {gauges.length > 0 && <section aria-label="Gauges" className="flex items-start justify-around gap-2 border-b border-border py-2.5">
      {gauges.map(({ provider, window }) => <RingGauge key={provider} provider={provider} used={window.usedPercent} label={window.label || windowLabel(window.id, window.windowMinutes)} />)}
    </section>}
    {live.map(({ provider, snapshot }) => <section key={provider} aria-label={`${harnessLabel(provider)} usage`} className="border-b border-border py-1.5 last:border-0">
      <div className="flex items-center gap-1.5 text-ui">
        <HarnessMark harness={provider} size={12} />
        <span className="font-medium text-foreground">{harnessLabel(provider)}</span>
        {snapshot.planType && <span className="text-[11px] text-muted-foreground">{snapshot.planType}</span>}
        <span className="ml-auto text-[11px] text-muted-foreground">{snapshot.source.charAt(0).toUpperCase() + snapshot.source.slice(1)}</span>
      </div>
      {snapshot.windows.length === 0 && <p className="py-1 text-[11px] text-muted-foreground">Connected — no quota windows reported.</p>}
      {snapshot.windows.map(window => <WindowRow key={window.id} provider={provider} window={window} nowMs={nowMs} />)}
    </section>)}
  </>;
}
