// The Context lens: a modal over the chat showing every live context window
// in it (the chat model, an orchestrator, each worker), how full each one is,
// what fills it and what Bridge adds. Numbers come only from what each harness
// reports; a window that cannot report says why instead of showing a zero.
import { useEffect, useState } from "react";
import { Gauge, Layers, TrendingUp } from "lucide-react";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogPanel, DialogTitle } from "@/components/ui/dialog";
import { Spinner } from "@/components/ui/spinner";
import { cn } from "@/lib/utils";
import type { ContextBridgeContribution, ContextReadingState, ContextWindow, ContextWindowReading, EarlierContextWindow } from "../protocol/generated/protocol";
import { contextPressure } from "../usage";
import { compactTokens, contextTone, TONE_TEXT, useContextWindows, windowComposition, type ContextTone } from "../contextWindows";
import { harnessLabel, modelLabel } from "../utils";
import { ContextGauge } from "./ContextRing";
import { HarnessMark } from "./harnessMarks";

// Segment hues in rank order: the largest segment always wears ctx-1, so the
// bar and the legend agree without a lookup.
const SEGMENT_FILL = ["bg-ctx-1", "bg-ctx-2", "bg-ctx-3", "bg-ctx-4", "bg-ctx-5", "bg-ctx-6"];
const segmentFill = (index: number) => SEGMENT_FILL[index] ?? "bg-foreground/25";

/** The hero wears a wash and an ink per pressure level; the ring beside the
 *  composer stays quiet (see TONE_TEXT) because it is in view all day. */
const PRESSURE_CARD: Record<ContextTone, string> = {
  unknown: "border-border bg-card",
  healthy: "border-success/25 bg-success/[0.06]",
  elevated: "border-border bg-muted/60",
  high: "border-warning/40 bg-warning/10",
  critical: "border-destructive/45 bg-destructive/10",
};
const PRESSURE_INK: Record<ContextTone, string> = {
  unknown: "text-muted-foreground",
  healthy: "text-success",
  elevated: "text-foreground",
  high: "text-warning",
  critical: "text-destructive",
};

const STATE_BADGE: Record<ContextReadingState, string> = {
  measured: "bg-success/8 text-success dark:bg-success/16",
  reported: "bg-info/8 text-info dark:bg-info/16",
  estimated: "border-dashed border-border text-muted-foreground",
};

function StateBadge({ state, className }: { state: ContextReadingState; className?: string }) {
  return <span className={cn(
    "inline-flex h-5 shrink-0 items-center whitespace-nowrap rounded-full border border-transparent px-2 text-[10px] font-semibold uppercase tracking-[0.08em]",
    STATE_BADGE[state],
    className,
  )}>{state}</span>;
}

function roleTitle(window: ContextWindow): string {
  return window.role === "chat" ? "Chat" : window.role === "orchestrator" ? "Orchestrator" : window.label;
}

function percentOf(part: number, whole: number): string {
  if (whole <= 0) return "0%";
  const value = (part / whole) * 100;
  return `${value >= 10 ? Math.round(value) : Math.round(value * 10) / 10}%`;
}

function observedLabel(iso: string): string | null {
  const time = new Date(iso);
  return Number.isNaN(time.getTime()) ? null : time.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

/** One track for the whole window: named segments in rank hues, the occupied
 *  remainder hatched, headroom left empty, the auto-compact line marked. */
export function CompositionBar({ reading, tall = false }: { reading: ContextWindowReading; tall?: boolean }) {
  const composition = windowComposition(reading);
  const width = (tokens: number) => `${(tokens / reading.windowTokens) * 100}%`;
  const compactsAt = reading.autoCompactTokens != null && reading.autoCompactTokens < reading.windowTokens ? reading.autoCompactTokens : null;
  return <div className={cn("relative", tall && compactsAt != null && "pt-4")}>
    {compactsAt != null && <span
      aria-hidden="true"
      className={cn("pointer-events-none absolute top-0 -translate-x-1/2 whitespace-nowrap font-mono text-[10px] text-muted-foreground", !tall && "hidden")}
      style={{ left: width(compactsAt) }}
    >compacts ~{compactTokens(compactsAt)}</span>}
    <div
      role="img"
      aria-label={`${reading.percent}% of the window is occupied`}
      className={cn("relative flex overflow-hidden rounded-full bg-muted ring-1 ring-inset ring-border", tall ? "h-4" : "h-2")}
    >
      {composition.used.map((segment, index) => <span
        key={segment.name}
        className={cn("h-full origin-left motion-safe:animate-[meter-fill_600ms_ease-out_both]", segmentFill(index))}
        style={{ width: width(segment.tokens), animationDelay: `${index * 40}ms` }}
        title={`${segment.name} · ${compactTokens(segment.tokens)}`}
      />)}
      {composition.unattributed > 0 && <span
        className={cn("h-full", composition.used.length === 0 ? "bg-ctx-1" : "ctx-hatch-neutral")}
        style={{ width: width(composition.unattributed) }}
        title={`Not attributed · ${compactTokens(composition.unattributed)}`}
      />}
      {compactsAt != null && <span
        aria-hidden="true"
        className="absolute inset-y-0 w-0.5 bg-foreground/70"
        style={{ left: width(compactsAt) }}
        title={`Auto-compacts at ${compactTokens(compactsAt)}`}
      />}
    </div>
  </div>;
}

function Legend({ reading }: { reading: ContextWindowReading }) {
  const composition = windowComposition(reading);
  return <div className="mt-2.5 flex flex-wrap gap-x-3.5 gap-y-1.5 text-[11px]">
    {composition.used.map((segment, index) => <span key={segment.name} className="inline-flex items-center gap-1.5">
      <span className={cn("size-2 rounded-full", segmentFill(index))} aria-hidden="true" />
      <span className="text-foreground">{segment.name}</span>
      <span className="font-mono tabular-nums text-muted-foreground">{percentOf(segment.tokens, reading.windowTokens)}</span>
    </span>)}
    {composition.unattributed > 0 && <span className="inline-flex items-center gap-1.5">
      <span className={cn("size-2 rounded-full", composition.used.length === 0 ? "bg-ctx-1" : "ctx-hatch-neutral")} aria-hidden="true" />
      <span className="text-foreground">{composition.used.length === 0 ? "Used" : "Not attributed"}</span>
      <span className="font-mono tabular-nums text-muted-foreground">{percentOf(composition.unattributed, reading.windowTokens)}</span>
    </span>}
    <span className="inline-flex items-center gap-1.5">
      <span className="size-2 rounded-full ring-1 ring-inset ring-border" aria-hidden="true" />
      <span className="text-foreground">Free</span>
      <span className="font-mono tabular-nums text-muted-foreground">{percentOf(composition.free, reading.windowTokens)}</span>
    </span>
  </div>;
}

function WindowTab({ window, selected, onSelect }: { window: ContextWindow; selected: boolean; onSelect: () => void }) {
  const reading = window.current;
  const tone = contextTone(reading?.percent);
  return <button
    type="button"
    role="tab"
    aria-selected={selected}
    onClick={onSelect}
    className={cn(
      "flex min-w-0 items-center gap-2.5 rounded-xl border px-3 py-2 text-left transition-colors",
      selected ? "border-foreground/30 bg-card shadow-[var(--control-shadow)]" : "border-border hover:bg-accent",
      !reading && "border-dashed",
    )}
  >
    <ContextGauge percent={reading?.percent} size={26} />
    <span className="min-w-0 flex-1">
      <span className="flex items-center gap-1.5 text-[12px] font-medium text-foreground">
        <HarnessMark harness={window.harness} size={12} />
        <span className="truncate">{roleTitle(window)}</span>
      </span>
      <span className="block truncate font-mono text-[11px] text-muted-foreground">{window.model ? modelLabel(window.model) : harnessLabel(window.harness)}</span>
    </span>
    <span className={cn("shrink-0 font-mono text-[13px] tabular-nums", reading ? TONE_TEXT[tone] : "text-muted-foreground")}>{reading ? `${reading.percent}%` : "–"}</span>
  </button>;
}

function SectionTitle({ children, aside }: { children: React.ReactNode; aside?: React.ReactNode }) {
  return <div className="mb-2 mt-5 flex items-baseline justify-between gap-2">
    <h3 className="text-[11px] font-semibold uppercase tracking-[0.13em] text-muted-foreground">{children}</h3>
    {aside}
  </div>;
}

function Row({ fill, label, detail, tokens, share, relative }: { fill: string; label: string; detail?: string | null; tokens: number; share: string; relative: number }) {
  return <li className="rounded-xl border border-border bg-card px-3 py-2">
    <div className="flex items-center gap-2.5">
      <span className={cn("size-2.5 shrink-0 rounded-full", fill)} aria-hidden="true" />
      <span className="min-w-0 flex-1 truncate text-[12px] font-medium text-foreground">{label}{detail && <span className="ml-1.5 font-normal text-muted-foreground">{detail}</span>}</span>
      <span className="shrink-0 font-mono text-[12px] tabular-nums text-foreground">{compactTokens(tokens)}</span>
      <span className="w-10 shrink-0 text-right font-mono text-[11px] tabular-nums text-muted-foreground">{share}</span>
    </div>
    <div className="mt-1.5 h-1.5 overflow-hidden rounded-full bg-muted" aria-hidden="true">
      <div className={cn("h-full origin-left rounded-full motion-safe:animate-[meter-fill_600ms_ease-out_both]", fill)} style={{ width: `${Math.max(relative, 1.5)}%` }} />
    </div>
  </li>;
}

function BridgeShare({ bridge }: { bridge: ContextBridgeContribution }) {
  const tiles = [
    bridge.stableTokens != null && { label: "Stable prompt · cacheable", tokens: bridge.stableTokens },
    bridge.variableTokens != null && { label: "Variable prompt and briefing", tokens: bridge.variableTokens },
  ].filter((tile): tile is { label: string; tokens: number } => !!tile);
  if (tiles.length === 0) return null;
  return <>
    <SectionTitle aside={<StateBadge state="estimated" />}>What Bridge adds</SectionTitle>
    <div className="grid gap-2 sm:grid-cols-2">
      {tiles.map(tile => <div key={tile.label} className="rounded-xl border border-border bg-card px-3 py-2.5">
        <p className="text-[11px] text-muted-foreground">{tile.label}</p>
        <p className="mt-0.5 font-mono text-[17px] font-semibold tabular-nums text-foreground">~{compactTokens(tile.tokens)}</p>
      </div>)}
    </div>
    <p className="mt-1.5 text-[11px] text-muted-foreground">From the last compiled prompt{bridge.method ? `, counted as ${bridge.method}` : ""}.</p>
  </>;
}

function EarlierList({ earlier }: { earlier: EarlierContextWindow[] }) {
  return <>
    <SectionTitle>Earlier in this chat</SectionTitle>
    <ul className="grid gap-1.5">
      {earlier.map(window => <li key={`${window.harness}:${window.model}:${window.observedAt}`} className="flex items-center gap-2.5 rounded-xl border border-dashed border-border px-3 py-2 text-[12px]">
        <ContextGauge percent={window.percent} size={18} />
        <HarnessMark harness={window.harness} size={12} />
        <span className="min-w-0 flex-1 truncate text-foreground">{window.model ? modelLabel(window.model) : harnessLabel(window.harness)}</span>
        <StateBadge state={window.state} />
        <span className="shrink-0 font-mono tabular-nums text-muted-foreground">{compactTokens(window.usedTokens)} / {compactTokens(window.windowTokens)} · {window.percent}%</span>
      </li>)}
    </ul>
  </>;
}

function WindowDetail({ window, bridge, earlier }: { window: ContextWindow; bridge: ContextBridgeContribution | null | undefined; earlier: EarlierContextWindow[] }) {
  const reading = window.current;
  const harness = harnessLabel(window.harness);
  const identity = <span className="inline-flex min-w-0 items-center gap-1.5 text-foreground">
    <HarnessMark harness={window.harness} size={12} />
    <span className="truncate">{harness}{window.model ? ` · ${modelLabel(window.model)}` : ""}</span>
  </span>;

  if (!reading) {
    return <section aria-label="Context pressure" className="rounded-2xl border border-dashed border-border p-4">
      <div className="flex items-center gap-4">
        <span className="grid size-[76px] shrink-0 place-items-center rounded-full bg-muted text-muted-foreground"><Gauge size={26} aria-hidden="true" /></span>
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="font-display text-[17px] font-semibold leading-tight text-foreground">No reading yet</h3>
            <span className="inline-flex h-5 items-center rounded-full border border-dashed border-border px-2 text-[10px] font-semibold uppercase tracking-[0.08em] text-muted-foreground">Unavailable</span>
          </div>
          <p className="mt-1 text-[12px] leading-relaxed text-muted-foreground">{window.unavailableReason}</p>
          <div className="mt-2.5 font-mono text-[11px]">{identity}</div>
        </div>
      </div>
    </section>;
  }

  const pressure = contextPressure(reading.percent);
  const level = pressure.level;
  const composition = windowComposition(reading);
  const largestSegment = Math.max(0, ...composition.used.map(segment => segment.tokens), composition.unattributed);
  const largestConsumer = Math.max(0, ...reading.consumers.map(consumer => consumer.tokens));
  const observed = observedLabel(reading.observedAt);
  const owner = reading.compactionOwner === "harness"
    ? `${harness} compacts this window itself. A Bridge checkpoint is saved separately and does not shrink it.`
    : "This harness does not compact its own window; Bridge checkpoints it when pressure is high.";

  return <div>
    <section aria-label="Context pressure" className={cn("rounded-2xl border p-4 transition-colors duration-300", PRESSURE_CARD[level])}>
      <div className="flex items-center gap-4">
        <div className="relative grid shrink-0 place-items-center">
          <ContextGauge percent={reading.percent} size={76} fillClassName={PRESSURE_INK[level]} />
          <span className={cn("absolute font-mono text-[15px] font-semibold tabular-nums", PRESSURE_INK[level])}>{reading.percent}%</span>
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className={cn("font-display text-[17px] font-semibold leading-tight", PRESSURE_INK[level])}>{pressure.label}</h3>
            <StateBadge state={reading.state} />
          </div>
          <p className="mt-1 text-[12px] leading-relaxed text-muted-foreground">{pressure.explanation}</p>
          <div className="mt-2.5 flex flex-wrap items-center gap-x-3 gap-y-1 font-mono text-[11px] text-muted-foreground">
            {identity}
            <span className="tabular-nums"><b className="font-semibold text-foreground">{compactTokens(reading.usedTokens)}</b> of {compactTokens(reading.windowTokens)}</span>
            <span className="tabular-nums">{compactTokens(composition.free)} free</span>
            {observed && <span>read at {observed}</span>}
          </div>
        </div>
      </div>
      <div className="mt-4"><CompositionBar reading={reading} tall /></div>
      <Legend reading={reading} />
    </section>

    {reading.forecast && <div className="mt-3 flex flex-wrap items-center gap-2 rounded-xl border border-border bg-card px-3 py-2 text-[12px] text-foreground">
      <TrendingUp size={14} className={cn("shrink-0", PRESSURE_INK[level])} aria-hidden="true" />
      <span>About <b className="font-semibold">{reading.forecast.turnsRemaining} turn{reading.forecast.turnsRemaining === 1 ? "" : "s"}</b> until {reading.autoCompactTokens != null ? "it compacts" : "the window is full"}, at ~{compactTokens(reading.forecast.growthPerTurn)} per turn.</span>
      <StateBadge state="estimated" className="ml-auto" />
    </div>}

    <SectionTitle aside={<span className="font-mono text-[11px] text-muted-foreground">share of window</span>}>What fills it</SectionTitle>
    {composition.used.length === 0 && composition.unattributed === 0
      ? <p className="rounded-xl border border-dashed border-border px-3 py-2.5 text-[12px] leading-relaxed text-muted-foreground">{harness} reports how full the window is but not what is in it.</p>
      : <ul className="grid gap-1.5">
        {composition.used.map((segment, index) => <Row key={segment.name} fill={segmentFill(index)} label={segment.name} tokens={segment.tokens} share={percentOf(segment.tokens, reading.windowTokens)} relative={(segment.tokens / largestSegment) * 100} />)}
        {composition.unattributed > 0 && (composition.used.length === 0
          ? <li className="rounded-xl border border-dashed border-border px-3 py-2.5 text-[12px] leading-relaxed text-muted-foreground">{harness} reports how full the window is but not what is in it.</li>
          : <Row fill="ctx-hatch-neutral" label="Not attributed" tokens={composition.unattributed} share={percentOf(composition.unattributed, reading.windowTokens)} relative={(composition.unattributed / largestSegment) * 100} />)}
      </ul>}

    {reading.consumers.length > 0 && <>
      <SectionTitle>Biggest consumers</SectionTitle>
      <ul className="grid gap-1.5">
        {reading.consumers.map(consumer => <Row key={consumer.label} fill="bg-foreground/55" label={consumer.label} detail={consumer.detail} tokens={consumer.tokens} share={percentOf(consumer.tokens, reading.windowTokens)} relative={(consumer.tokens / largestConsumer) * 100} />)}
      </ul>
    </>}

    {window.role !== "worker" && bridge && <BridgeShare bridge={bridge} />}
    {window.role !== "worker" && earlier.length > 0 && <EarlierList earlier={earlier} />}
    <p className="mt-5 text-[11px] leading-relaxed text-muted-foreground">{owner}</p>
  </div>;
}

export function ContextLensDialog({ open, sessionId, focusSessionId, refreshKey, onClose }: {
  open: boolean;
  /** The chat whose windows are shown: itself first, then every worker under it. */
  sessionId: string;
  /** Which window opens selected; defaults to the chat's own. */
  focusSessionId?: string | null;
  /** Changes when the chat's context moved; triggers an immediate refetch. */
  refreshKey?: unknown;
  onClose: () => void;
}) {
  const { result, unavailable } = useContextWindows(sessionId, open, refreshKey);
  const [selected, setSelected] = useState<string | null>(null);
  useEffect(() => { if (open) setSelected(focusSessionId ?? sessionId); }, [open, focusSessionId, sessionId]);
  const windows = result?.windows ?? [];
  const detail = windows.find(window => window.sessionId === selected) ?? windows[0];
  const live = windows.filter(window => window.current).length;

  return <Dialog open={open} onOpenChange={next => { if (!next) onClose(); }}>
    <DialogContent aria-label="Context lens" className="max-w-2xl">
      <DialogHeader>
        <div className="flex items-start gap-3">
          <span className="mt-0.5 grid size-9 shrink-0 place-items-center rounded-xl bg-ctx-1/12 text-ctx-1"><Layers size={17} aria-hidden="true" /></span>
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-2">
              <DialogTitle className="font-display text-base font-medium text-foreground">Context lens</DialogTitle>
              {result && <span className="inline-flex h-5 items-center rounded-full bg-muted px-2 font-mono text-[10px] font-semibold uppercase tracking-[0.08em] text-muted-foreground">{live} live · {windows.length} window{windows.length === 1 ? "" : "s"}</span>}
            </div>
            <DialogDescription className="mt-1 text-[11px] leading-5">Every live window in this chat, read from its own harness. A window that cannot report says why; it is never drawn as 0%.</DialogDescription>
          </div>
        </div>
      </DialogHeader>
      <DialogPanel>
        {!result
          ? (unavailable
            ? <div role="alert" className="rounded-2xl border border-dashed border-border px-4 py-10 text-center">
              <p className="text-[13px] font-medium text-foreground">Context is unavailable</p>
              <p className="mt-1 text-[12px] text-muted-foreground">This Bridge daemon does not report context windows yet. Restart Bridge after updating.</p>
            </div>
            : <div className="flex flex-col items-center gap-3 px-4 py-12 text-center">
              <Spinner className="size-5 text-muted-foreground" />
              <p className="text-[12px] text-muted-foreground">Reading context windows…</p>
            </div>)
          : <>
            {windows.length > 1 && <div role="tablist" aria-label="Context windows" className="mb-4 grid auto-cols-[minmax(11rem,1fr)] grid-flow-col gap-2 overflow-x-auto pb-1">
              {windows.map(window => <WindowTab key={window.sessionId} window={window} selected={window.sessionId === detail?.sessionId} onSelect={() => setSelected(window.sessionId)} />)}
            </div>}
            {detail
              ? <WindowDetail window={detail} bridge={result.bridge} earlier={result.earlier} />
              : <p className="rounded-2xl border border-dashed border-border px-4 py-10 text-center text-[12px] text-muted-foreground">No window is live in this chat yet.</p>}
          </>}
      </DialogPanel>
    </DialogContent>
  </Dialog>;
}
