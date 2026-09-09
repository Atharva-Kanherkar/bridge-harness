// The menu-bar meter popover: CodexBar's menu card (`docs/ui.md`) rendered in
// Bridge's chrome. Two readings per provider — a ring gauge for the window
// closest to biting, and a pace chart that draws the even-rate line against
// what has really been spent — then one bar per quota window with its reset.
// Pace math is the shared `src/meter.ts` port; styling is Tailwind utilities
// plus `.u-glass-popover` only. Series colour follows the harness.
import { useEffect, useMemo, useRef, useState } from "react";
import { Gauge, RefreshCw, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { formatReset, windowLabel, type RateWindow, type UsageProvider, type UsageSnapshot } from "../../usage";
import { pacePhrase, paceVisible, paceWeekly, type MeterPace } from "../../meter";
import type { MeterRegistry } from "../../types";
import { HarnessMark, harnessChartDot, harnessChartText } from "../harnessMarks";
import { harnessLabel } from "../../utils";

const PROVIDER_ORDER: UsageProvider[] = ["codex", "claude", "cursor", "opencode"];

function providerSnapshots(usage: Partial<Record<UsageProvider, UsageSnapshot>>): Array<{ provider: UsageProvider; snapshot: UsageSnapshot }> {
  return PROVIDER_ORDER.filter(provider => usage[provider]).map(provider => ({ provider, snapshot: usage[provider]! }));
}

const clampPercent = (value: number) => Math.min(100, Math.max(0, value));

/** The window a provider is closest to exhausting. */
function headlineWindow(windows: RateWindow[]): RateWindow | undefined {
  return windows.reduce<RateWindow | undefined>((worst, window) => (worst == null || window.usedPercent > worst.usedPercent ? window : worst), undefined);
}

/** A ring gauge: the headline window's usage as an arc in the harness colour. */
function RingGauge({ provider, used, label }: { provider: UsageProvider; used: number; label: string }) {
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

/** Even-rate pace, drawn: the diagonal is what an evenly spent window looks
 * like, the filled line is what has really been spent, and the light line
 * beyond the dot is where the current rate lands by reset. Hover reads both. */
function PaceChart({ provider, window, pace, nowMs }: { provider: UsageProvider; window: RateWindow; pace: MeterPace; nowMs: number }) {
  const [hover, setHover] = useState<number | null>(null);
  const plotRef = useRef<HTMLDivElement>(null);
  const width = 100;
  const height = 36;
  const elapsedFraction = clampPercent(pace.expectedUsedPercent) / 100;
  const actual = clampPercent(pace.actualUsedPercent);
  const x = elapsedFraction * width;
  const y = height - (actual / 100) * height;
  // Projection at the current average rate, clipped at the top of the plot.
  const projectedAtReset = elapsedFraction > 0 ? Math.min(100, actual / elapsedFraction) : actual;
  const projectedX = elapsedFraction > 0 && projectedAtReset >= 100 ? (100 / (actual / elapsedFraction)) * width : width;
  const projectedY = height - (projectedAtReset / 100) * height;
  const reset = window.resetsInSeconds != null ? formatReset(window.resetsInSeconds) : undefined;
  const hoverFraction = hover == null ? null : Math.min(1, Math.max(0, hover));
  const hoverExpected = hoverFraction == null ? null : Math.round(hoverFraction * 100);
  const hoverActual = hoverFraction == null ? null : hoverFraction <= elapsedFraction
    ? Math.round(elapsedFraction > 0 ? (actual * hoverFraction) / elapsedFraction : 0)
    : null;
  const onMove = (event: React.MouseEvent<HTMLDivElement>) => {
    const rect = plotRef.current?.getBoundingClientRect();
    if (!rect || rect.width === 0) return;
    setHover((event.clientX - rect.left) / rect.width);
  };
  return <figure aria-label={`${harnessLabel(provider)} ${window.label || windowLabel(window.id, window.windowMinutes)} pace`} className="mt-2">
    <div ref={plotRef} className="relative h-14 w-full" onMouseMove={onMove} onMouseLeave={() => setHover(null)}>
      <svg viewBox={`0 0 ${width} ${height}`} preserveAspectRatio="none" className={cn("block h-full w-full overflow-visible", harnessChartText(provider))} role="img" aria-label={`${Math.round(actual)} percent used with ${Math.round(pace.expectedUsedPercent)} percent of the window elapsed`}>
        <line x1={0} y1={height} x2={width} y2={height} className="stroke-border" strokeWidth={1} vectorEffect="non-scaling-stroke" />
        {/* Even pace: spend arrives at 100% exactly at reset. */}
        <line x1={0} y1={height} x2={width} y2={0} className="stroke-muted-foreground/40" strokeWidth={1} vectorEffect="non-scaling-stroke" />
        {/* Spent so far, as a wash under the line. */}
        <path d={`M0 ${height} L${x} ${y} L${x} ${height} Z`} fill="currentColor" className="origin-bottom opacity-15 motion-safe:animate-[chart-rise_600ms_ease-out]" />
        <line x1={0} y1={height} x2={x} y2={y} stroke="currentColor" strokeWidth={2} strokeLinecap="round" vectorEffect="non-scaling-stroke" />
        {x < projectedX && <line x1={x} y1={y} x2={projectedX} y2={projectedY} stroke="currentColor" strokeWidth={1} strokeLinecap="round" className="opacity-40" vectorEffect="non-scaling-stroke" />}
        <circle cx={x} cy={y} r={2.2} fill="currentColor" className="stroke-background" strokeWidth={1} vectorEffect="non-scaling-stroke" />
        {hoverFraction != null && <line x1={hoverFraction * width} x2={hoverFraction * width} y1={0} y2={height} className="stroke-foreground/40" strokeWidth={1} vectorEffect="non-scaling-stroke" />}
      </svg>
      {hoverFraction != null && <div role="tooltip" className={cn("u-glass-popover pointer-events-none absolute top-0 z-10 rounded-lg px-2 py-1 text-[11px] leading-4 tabular-nums", hoverFraction > 0.55 ? "-translate-x-[calc(100%+8px)]" : "translate-x-2")} style={{ left: `${hoverFraction * 100}%` }}>
        <div className="text-muted-foreground">{hoverExpected}% of window</div>
        <div className="text-foreground">{hoverActual == null ? `even pace ${hoverExpected}%` : `spent ${hoverActual}% · even pace ${hoverExpected}%`}</div>
      </div>}
    </div>
    <figcaption className="mt-1 flex justify-between text-[11px] text-muted-foreground">
      <span>{pacePhrase(pace)}</span>
      {reset && <span className="tabular-nums">{reset}</span>}
    </figcaption>
  </figure>;
}

function WindowRow({ provider, window, nowMs, showPaceChart }: { provider: UsageProvider; window: RateWindow; nowMs: number; showPaceChart: boolean }) {
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
    <button type="button" aria-expanded={expanded} aria-controls={`meter-window-${window.id}`} onClick={() => setExpanded(value => !value)} className="mt-1 block h-1.5 w-full overflow-hidden rounded-full bg-muted text-left outline-none transition-shadow focus-visible:ring-2 focus-visible:ring-ring">
      <span className={cn("block h-full origin-left rounded-full motion-safe:animate-[meter-fill_600ms_ease-out]", harnessChartDot(provider))} style={{ width: `${used}%` }} />
    </button>
    {!showPaceChart && <div className="mt-1 flex flex-wrap gap-x-3 gap-y-0.5 text-[11px] text-muted-foreground">
      {reset && <span className="tabular-nums">{reset}</span>}
      {pace && showPace && <span>{pacePhrase(pace)}</span>}
    </div>}
    {showPaceChart && pace && showPace ? <PaceChart provider={provider} window={window} pace={pace} nowMs={nowMs} /> : showPaceChart && reset && <div className="mt-1 text-[11px] text-muted-foreground tabular-nums">{reset}</div>}
    {expanded && <p id={`meter-window-${window.id}`} className="mt-1 text-[11px] text-muted-foreground">{Math.round(used)}% of this window is used{reset ? `. ${reset === "fresh window" ? "Nothing spent since it reset" : reset}.` : "."}</p>}
  </div>;
}

export function MeterPopover({ usage, registry, refreshing, onRefresh, onClose, onOpenBridge }: {
  usage: Partial<Record<UsageProvider, UsageSnapshot>>;
  registry: MeterRegistry | null;
  refreshing: boolean;
  onRefresh: () => void;
  onClose: () => void;
  /** Present only on the menu-bar panel, which is otherwise a dead end: from
   *  the menu bar there is no other way through to the app. */
  onOpenBridge?: () => void;
}) {
  const [nowMs, setNowMs] = useState(() => Date.now());
  // Countdowns and pace go stale against a frozen clock, so tick while open.
  useEffect(() => {
    const timer = window.setInterval(() => setNowMs(Date.now()), 30_000);
    return () => window.clearInterval(timer);
  }, []);
  const dialogRef = useRef<HTMLDivElement>(null);
  // A dialog that opens without focus strands keyboard users; focus the
  // dialog itself on mount (Escape is handled globally, topmost-layer-first).
  useEffect(() => { dialogRef.current?.focus(); }, []);
  const live = providerSnapshots(usage);
  const liveProviders = new Set(live.map(entry => entry.provider));
  const awaiting = (registry?.providers ?? []).filter(entry => entry.supported && !liveProviders.has(entry.id as UsageProvider));
  const worst = live.flatMap(({ snapshot }) => snapshot.windows).reduce<number | null>((max, window) => (max == null ? window.usedPercent : Math.max(max, window.usedPercent)), null);
  const gauges = live.map(({ provider, snapshot }) => ({ provider, window: headlineWindow(snapshot.windows) })).filter((entry): entry is { provider: UsageProvider; window: RateWindow } => entry.window != null);
  // The meter is its own menu-bar window, so the card fills that window rather
  // than floating inside the app. Nothing sits behind it to be made inert,
  // which is why there is no `aria-modal` here.
  return <div ref={dialogRef} role="dialog" aria-label="Usage meter" tabIndex={-1} className="u-glass-popover flex h-dvh w-full flex-col overflow-hidden rounded-2xl outline-none">
    <div className="flex items-center gap-2 border-b border-border px-3.5 py-2.5">
      <Gauge size={13} className="shrink-0 text-muted-foreground" aria-hidden="true" />
      <h2 className="font-display text-sm font-semibold text-foreground">Meter</h2>
      {/* The worst window is the headline — it is the one about to bite. Kept
          terse because the header has to survive a 360pt panel. */}
      <span className="truncate text-caption tabular-nums text-muted-foreground">{worst == null ? "no limits" : `${Math.round(worst)}% worst`}</span>
      <span className="ml-auto flex shrink-0 items-center gap-1">
        {onOpenBridge && <button type="button" onClick={onOpenBridge} className="rounded-md px-2 py-1 text-[11px] font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">Open Bridge</button>}
        <button type="button" onClick={onRefresh} disabled={refreshing} aria-label="Refresh meter" className="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-40">
          <RefreshCw size={13} className={refreshing ? "animate-spin" : ""} aria-hidden="true" />
        </button>
        <button type="button" onClick={onClose} aria-label="Close meter" className="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">
          <X size={13} aria-hidden="true" />
        </button>
      </span>
    </div>
    <div className="min-h-0 flex-1 overflow-y-auto px-3.5 py-2">
      {registry == null && live.length === 0 && <div role="status" className="flex items-center justify-center gap-2 py-5 text-caption text-muted-foreground"><RefreshCw size={12} className="animate-spin" aria-hidden="true" />Loading meter</div>}
      {gauges.length > 0 && <section aria-label="Gauges" className="flex items-start justify-around gap-2 border-b border-border py-2.5">
        {gauges.map(({ provider, window }) => <RingGauge key={provider} provider={provider} used={window.usedPercent} label={window.label || windowLabel(window.id, window.windowMinutes)} />)}
      </section>}
      {live.map(({ provider, snapshot }) => {
        const headline = headlineWindow(snapshot.windows);
        return <section key={provider} aria-label={`${harnessLabel(provider)} usage`} className="border-b border-border py-1.5 last:border-0">
          <div className="flex items-center gap-1.5 text-ui">
            <HarnessMark harness={provider} size={12} />
            <span className="font-medium text-foreground">{harnessLabel(provider)}</span>
            {snapshot.planType && <span className="text-[11px] text-muted-foreground">{snapshot.planType}</span>}
          </div>
          {snapshot.windows.length === 0 && <p className="py-1 text-[11px] text-muted-foreground">Connected — no quota windows reported.</p>}
          {snapshot.windows.map(window => <WindowRow key={window.id} provider={provider} window={window} nowMs={nowMs} showPaceChart={window === headline} />)}
        </section>;
      })}
      {awaiting.map(entry => <section key={entry.id} aria-label={`${entry.label} usage`} className="flex items-center gap-2 border-b border-border py-3 last:border-0">
        <HarnessMark harness={entry.id} size={12} />
        <span className="font-medium text-foreground">{entry.label}</span>
        <span className="ml-auto text-[11px] text-muted-foreground">Awaiting live usage</span>
      </section>)}
    </div>
  </div>;
}
