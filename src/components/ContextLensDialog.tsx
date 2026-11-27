// The Context lens: a modal over the chat showing every live context window
// in it (the chat model, an orchestrator, each worker), how full each one is,
// what fills it and what Bridge adds. Numbers come only from what each harness
// reports; a window that cannot report says why instead of showing a zero.
import { useState } from "react";
import { Skeleton, SkeletonRegion, SkeletonText } from "@/components/ui/skeleton";
import { Minimize2, TrendingUp } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogPanel, DialogTitle } from "@/components/ui/dialog";
import { RowButton } from "@/components/ui/row";
import { Tip } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import type { ContextBridgeContribution, ContextReadingState, ContextWindow, ContextWindowReading, EarlierContextWindow } from "../protocol/generated/protocol";
import { contextPressure } from "../usage";
import { friendlyError } from "../errors";
import { compactTokens, contextTone, TONE_TEXT, useContextWindows, windowComposition, type ContextTone } from "../contextWindows";
import { harnessLabel, modelLabel } from "../utils";
import { ContextGauge } from "./ContextRing";
import { HarnessMark } from "./harnessMarks";
import { useResetOnChange } from "../useResetOnChange";

// Segments wear one neutral ink at six strengths, strongest for the largest,
// so the bar and the legend agree without a lookup and colour stays free to
// mean pressure. Bridge's chrome is achromatic; six hues read as six statuses.
const SEGMENT_FILL = ["bg-foreground/90", "bg-foreground/68", "bg-foreground/50", "bg-foreground/36", "bg-foreground/26", "bg-foreground/18"];
const segmentFill = (index: number) => SEGMENT_FILL[index] ?? "bg-foreground/12";

/** The hero stays a neutral card; only high and critical earn a wash, so a
 *  healthy window reads calm. The badge carries the level everywhere. */
const PRESSURE_CARD: Record<ContextTone, string> = {
  unknown: "border-border bg-card",
  healthy: "border-border bg-card",
  elevated: "border-border bg-card",
  high: "border-warning/40 bg-warning/[0.06]",
  critical: "border-destructive/45 bg-destructive/[0.06]",
};
const PRESSURE_BADGE: Record<ContextTone, { label: string; variant: "outline" | "success" | "secondary" | "warning" | "error" }> = {
  unknown: { label: "Unknown", variant: "outline" },
  healthy: { label: "Healthy", variant: "success" },
  elevated: { label: "Elevated", variant: "secondary" },
  high: { label: "High pressure", variant: "warning" },
  critical: { label: "Critical", variant: "error" },
};

function PressureBadge({ level }: { level: ContextTone }) {
  const badge = PRESSURE_BADGE[level];
  return <Badge variant={badge.variant}>{badge.label}</Badge>;
}

/** How the number was obtained. An estimate is dashed, like everything else
 *  in the lens that Bridge worked out instead of reading. */
function StateBadge({ state, className }: { state: ContextReadingState; className?: string }) {
  return <Badge variant={state === "estimated" ? "dashed" : "muted"} className={className}><span className="capitalize">{state}</span></Badge>;
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

/** One track for the whole window: named segments in rank ink, the occupied
 *  remainder hatched, headroom left empty, the auto-compact line marked. */
export function CompositionBar({ reading, tall = false }: { reading: ContextWindowReading; tall?: boolean }) {
  const composition = windowComposition(reading);
  const width = (tokens: number) => `${(tokens / reading.windowTokens) * 100}%`;
  const compactsAt = reading.autoCompactTokens != null && reading.autoCompactTokens < reading.windowTokens ? reading.autoCompactTokens : null;
  // The label hangs off whichever side of the marker has room, so it never
  // runs past the end of the track.
  const labelLeft = compactsAt != null && compactsAt / reading.windowTokens > 0.5;
  return <div className={cn("relative", tall && compactsAt != null && "pt-5")}>
    {compactsAt != null && <span
      aria-hidden="true"
      className={cn("pointer-events-none absolute top-0 whitespace-nowrap font-mono text-micro text-muted-foreground", labelLeft ? "-translate-x-full pr-1.5" : "pl-1.5", !tall && "hidden")}
      style={{ left: width(compactsAt) }}
    >compacts ~{compactTokens(compactsAt)}</span>}
    <div
      role="img"
      aria-label={`${reading.percent}% of the window is occupied`}
      className={cn("relative flex gap-px overflow-hidden rounded-full bg-muted ring-1 ring-inset ring-border", tall ? "h-3" : "h-2")}
    >
      {composition.used.map((segment, index) => <Tip key={segment.name} label={`${segment.name} · ${compactTokens(segment.tokens)}`}>
        <span
          className={cn("h-full origin-left motion-safe:animate-meter-fill", segmentFill(index))}
          style={{ width: width(segment.tokens), animationDelay: `${index * 40}ms` }}
        />
      </Tip>)}
      {composition.unattributed > 0 && <Tip label={`Not attributed · ${compactTokens(composition.unattributed)}`}>
        <span
          className={cn("h-full", composition.used.length === 0 ? segmentFill(0) : "ctx-hatch-neutral")}
          style={{ width: width(composition.unattributed) }}
        />
      </Tip>}
    </div>
    {compactsAt != null && <span
      aria-hidden="true"
      className={cn("pointer-events-none absolute bottom-0 w-px bg-foreground", tall ? "h-5" : "h-2")}
      style={{ left: width(compactsAt) }}
    />}
  </div>;
}

function LegendItem({ swatch, label, share }: { swatch: string; label: string; share: string }) {
  return <span className="inline-flex items-center gap-1.5">
    <span className={cn("size-2 rounded-sm", swatch)} aria-hidden="true" />
    <span className="text-foreground">{label}</span>
    <span className="font-mono tabular-nums text-muted-foreground">{share}</span>
  </span>;
}

function Legend({ reading }: { reading: ContextWindowReading }) {
  const composition = windowComposition(reading);
  return <div className="mt-3 flex flex-wrap gap-x-4 gap-y-1.5 text-caption">
    {composition.used.map((segment, index) => <LegendItem key={segment.name} swatch={segmentFill(index)} label={segment.name} share={percentOf(segment.tokens, reading.windowTokens)} />)}
    {composition.unattributed > 0 && <LegendItem
      swatch={composition.used.length === 0 ? segmentFill(0) : "ctx-hatch-neutral"}
      label={composition.used.length === 0 ? "Used" : "Not attributed"}
      share={percentOf(composition.unattributed, reading.windowTokens)}
    />}
    <LegendItem swatch="bg-muted ring-1 ring-inset ring-foreground/25" label="Free" share={percentOf(composition.free, reading.windowTokens)} />
  </div>;
}

function WindowTab({ window, selected, onSelect }: { window: ContextWindow; selected: boolean; onSelect: () => void }) {
  const reading = window.current;
  const tone = contextTone(reading?.percent);
  return <RowButton
    role="tab"
    aria-selected={selected}
    onClick={onSelect}
    size="lg"
    tone="tab"
    shape="card"
  >
    <ContextGauge percent={reading?.percent} size={22} />
    <span className="min-w-0 flex-1">
      <span className="flex items-center gap-1.5 text-meta font-medium text-foreground">
        <HarnessMark harness={window.harness} size={12} />
        <span className="truncate">{roleTitle(window)}</span>
      </span>
      <span className="block truncate text-caption text-muted-foreground">{window.model ? modelLabel(window.model) : harnessLabel(window.harness)}</span>
    </span>
    <span className={cn("shrink-0 font-mono text-meta tabular-nums", reading ? TONE_TEXT[tone] : "text-muted-foreground")}>{reading ? `${reading.percent}%` : "–"}</span>
  </RowButton>;
}

function SectionTitle({ children, aside }: { children: React.ReactNode; aside?: React.ReactNode }) {
  return <div className="mb-2 mt-6 flex items-center justify-between gap-2 px-0.5">
    <h3 className="text-meta font-medium text-foreground">{children}</h3>
    {aside}
  </div>;
}

function Row({ fill, label, detail, tokens, share, relative }: { fill: string; label: string; detail?: string | null; tokens: number; share: string; relative: number }) {
  return <li className="grid grid-cols-[minmax(0,1fr)_3.25rem_2.75rem] items-center gap-3 px-3 py-2 @md:grid-cols-[minmax(0,1fr)_minmax(4rem,30%)_3.25rem_2.75rem]">
    <span className="flex min-w-0 items-center gap-2 text-meta text-foreground">
      <span className={cn("size-2 shrink-0 rounded-sm", fill)} aria-hidden="true" />
      <span className="min-w-0">
        <span className="block truncate">{label}</span>
        {detail && <span className="block truncate text-caption text-muted-foreground">{detail}</span>}
      </span>
    </span>
    <span className="hidden h-1 overflow-hidden rounded-full bg-muted @md:block" aria-hidden="true">
      <span className={cn("block h-full origin-left rounded-full motion-safe:animate-meter-fill", fill)} style={{ width: `${Math.max(relative, 2)}%` }} />
    </span>
    <span className="text-right font-mono text-meta tabular-nums text-foreground">{compactTokens(tokens)}</span>
    <span className="text-right font-mono text-caption tabular-nums text-muted-foreground">{share}</span>
  </li>;
}

function RowList({ children }: { children: React.ReactNode }) {
  return <ul className="divide-y divide-border overflow-hidden rounded-xl border border-border bg-card">{children}</ul>;
}

function BridgeShare({ bridge }: { bridge: ContextBridgeContribution }) {
  const tiles = [
    bridge.stableTokens != null && { label: "Stable prompt · cacheable", tokens: bridge.stableTokens },
    bridge.variableTokens != null && { label: "Variable prompt and briefing", tokens: bridge.variableTokens },
  ].filter((tile): tile is { label: string; tokens: number } => !!tile);
  if (tiles.length === 0) return null;
  return <>
    <SectionTitle aside={<StateBadge state="estimated" />}>What Bridge adds</SectionTitle>
    <div className="grid gap-2 @md:grid-cols-2">
      {tiles.map(tile => <div key={tile.label} className="rounded-xl border border-border bg-card px-3 py-2.5">
        <p className="truncate text-caption text-muted-foreground">{tile.label}</p>
        <p className="mt-0.5 font-mono text-message font-medium tabular-nums text-foreground">~{compactTokens(tile.tokens)}</p>
      </div>)}
    </div>
    <p className="mt-1.5 px-0.5 text-caption text-muted-foreground">From the last compiled prompt{bridge.method ? `, counted as ${bridge.method}` : ""}.</p>
  </>;
}

function EarlierList({ earlier }: { earlier: EarlierContextWindow[] }) {
  return <>
    <SectionTitle>Earlier in this chat</SectionTitle>
    <ul className="divide-y divide-border overflow-hidden rounded-xl border border-border bg-card">
      {earlier.map(window => <li key={`${window.harness}:${window.model}:${window.observedAt}`} className="flex items-center gap-2.5 px-3 py-2 text-meta">
        <ContextGauge percent={window.percent} size={18} />
        <HarnessMark harness={window.harness} size={12} />
        <span className="min-w-0 flex-1 truncate text-foreground">{window.model ? modelLabel(window.model) : harnessLabel(window.harness)}</span>
        <StateBadge state={window.state} className="hidden @md:inline-flex" />
        <span className="shrink-0 font-mono text-caption tabular-nums text-muted-foreground">{compactTokens(window.usedTokens)} / {compactTokens(window.windowTokens)} · {window.percent}%</span>
      </li>)}
    </ul>
  </>;
}

// Harnesses with their own compaction command. Anything else falls back to a
// Bridge checkpoint, which summarises history for a later cold start without
// freeing provider tokens (`docs/compaction-and-resume.md`).
const NATIVE_COMPACT = new Set(["claude", "codex", "opencode"]);

type CompactState = { phase: "idle" } | { phase: "sending" } | { phase: "sent" } | { phase: "failed"; message: string };

/** The pressure card's footer: what compacting this window does, and the
 *  button that does it. Primary only once the window is under pressure. */
function CompactAction({ window, owner, urgent, onCompact }: {
  window: ContextWindow;
  owner: ContextWindowReading["compactionOwner"] | null;
  urgent: boolean;
  onCompact: (sessionId: string) => Promise<void>;
}) {
  const [state, setState] = useState<CompactState>({ phase: "idle" });
  // A new reading means the compaction landed (or the window moved on), so
  // the button is offered again.
  useResetOnChange([window.sessionId, window.current?.observedAt], () => setState({ phase: "idle" }));
  const harness = harnessLabel(window.harness);
  const native = owner ? owner === "harness" : NATIVE_COMPACT.has(window.harness);
  const how = native
    ? `Runs ${harness}'s own /compact on this window.`
    : `${harness} has no compact command, so Bridge saves a checkpoint instead. It does not shrink this window.`;
  const compact = async () => {
    setState({ phase: "sending" });
    try {
      await onCompact(window.sessionId);
      setState({ phase: "sent" });
    } catch (error) {
      setState({ phase: "failed", message: friendlyError(error, native ? `Couldn't compact this ${harness} window.` : "Couldn't save a checkpoint.") });
    }
  };
  const note = state.phase === "sent"
    ? (native ? `${harness} is compacting. The reading updates after it finishes.` : "Checkpoint requested.")
    : state.phase === "failed" ? state.message
    : how;
  return <div data-slot="compact-action" className="flex flex-wrap items-center justify-end gap-x-3 gap-y-2 border-t border-border px-4 py-3 @md:px-5">
    <p role={state.phase === "failed" ? "alert" : undefined} className={cn("min-w-[12rem] flex-1 text-meta leading-relaxed", state.phase === "failed" ? "text-destructive" : "text-muted-foreground")}>{note}</p>
    <Button
      size="sm"
      variant={urgent ? "default" : "outline"}
      onClick={() => void compact()}
      loading={state.phase === "sending"}
      disabled={state.phase === "sent"}
      aria-label={`Compact ${roleTitle(window)}`}
    >
      <Minimize2 className="size-3.5" aria-hidden="true" />
      {state.phase === "sent" ? "Compacting" : native ? "Compact" : "Checkpoint"}
    </Button>
  </div>;
}

function WindowDetail({ window, bridge, earlier, onCompact }: { window: ContextWindow; bridge: ContextBridgeContribution | null | undefined; earlier: EarlierContextWindow[]; onCompact?: (sessionId: string) => Promise<void> }) {
  const reading = window.current;
  const harness = harnessLabel(window.harness);
  const identity = <p className="flex min-w-0 items-center gap-1.5 text-meta text-muted-foreground">
    <HarnessMark harness={window.harness} size={12} />
    <span className="truncate">{harness}{window.model ? ` · ${modelLabel(window.model)}` : ""}</span>
  </p>;

  if (!reading) {
    return <section aria-label="Context pressure" className="overflow-hidden rounded-2xl border border-dashed border-border">
      <div className="p-4 @md:p-5">
        <div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-2">
          {identity}
          <Badge variant="dashed">Unavailable</Badge>
        </div>
        <p className="mt-4 font-display text-xl font-semibold leading-none tracking-tight text-foreground">No reading yet</p>
        <p className="mt-2 text-meta leading-relaxed text-muted-foreground">{window.unavailableReason}</p>
      </div>
      {onCompact && <CompactAction window={window} owner={null} urgent={false} onCompact={onCompact} />}
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
    <section aria-label="Context pressure" className={cn("overflow-hidden rounded-2xl border transition-colors duration-300", PRESSURE_CARD[level])}>
      <div className="p-4 @md:p-5">
        <div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-2">
          {identity}
          <div className="flex shrink-0 items-center gap-1.5">
            <StateBadge state={reading.state} />
            <PressureBadge level={level} />
          </div>
        </div>
        <div className="mt-4 flex flex-wrap items-baseline gap-x-2.5 gap-y-1">
          <span className="font-display text-4xl font-semibold leading-none tracking-tight tabular-nums text-foreground">{reading.percent}%</span>
          <span className="text-ui text-muted-foreground"><span className="font-mono tabular-nums text-foreground">{compactTokens(reading.usedTokens)}</span> of {compactTokens(reading.windowTokens)} tokens</span>
          <span className="ml-auto font-mono text-caption tabular-nums text-muted-foreground">{compactTokens(composition.free)} free</span>
        </div>
        <div className="mt-4"><CompositionBar reading={reading} tall /></div>
        <Legend reading={reading} />
        <p className="mt-4 text-caption leading-relaxed text-muted-foreground">
          {pressure.explanation}{observed ? ` Read at ${observed}.` : ""}
        </p>
      </div>
      {onCompact && <CompactAction window={window} owner={reading.compactionOwner} urgent={level === "high" || level === "critical"} onCompact={onCompact} />}
    </section>

    {reading.forecast && <p className="mt-3 flex flex-wrap items-center gap-2 px-0.5 text-meta text-muted-foreground">
      <TrendingUp size={13} className="shrink-0" aria-hidden="true" />
      <span className="min-w-0">About <b className="font-medium text-foreground">{reading.forecast.turnsRemaining} turn{reading.forecast.turnsRemaining === 1 ? "" : "s"}</b> until {reading.autoCompactTokens != null ? "it compacts" : "the window is full"}, at ~{compactTokens(reading.forecast.growthPerTurn)} per turn.</span>
      <StateBadge state="estimated" />
    </p>}

    <SectionTitle aside={<span className="text-caption text-muted-foreground">share of window</span>}>What fills it</SectionTitle>
    {composition.used.length === 0
      ? <p className="rounded-xl border border-dashed border-border px-3 py-2.5 text-meta leading-relaxed text-muted-foreground">{harness} reports how full the window is but not what is in it.</p>
      : <RowList>
        {composition.used.map((segment, index) => <Row key={segment.name} fill={segmentFill(index)} label={segment.name} tokens={segment.tokens} share={percentOf(segment.tokens, reading.windowTokens)} relative={(segment.tokens / largestSegment) * 100} />)}
        {composition.unattributed > 0 && <Row fill="ctx-hatch-neutral" label="Not attributed" tokens={composition.unattributed} share={percentOf(composition.unattributed, reading.windowTokens)} relative={(composition.unattributed / largestSegment) * 100} />}
      </RowList>}

    {reading.consumers.length > 0 && <>
      <SectionTitle>Biggest consumers</SectionTitle>
      <RowList>
        {reading.consumers.map(consumer => <Row key={consumer.label} fill="bg-foreground/40" label={consumer.label} detail={consumer.detail} tokens={consumer.tokens} share={percentOf(consumer.tokens, reading.windowTokens)} relative={(consumer.tokens / largestConsumer) * 100} />)}
      </RowList>
    </>}

    {window.role !== "worker" && bridge && <BridgeShare bridge={bridge} />}
    {window.role !== "worker" && earlier.length > 0 && <EarlierList earlier={earlier} />}
    <p className="mt-6 px-0.5 text-caption leading-relaxed text-muted-foreground">{owner}</p>
  </div>;
}

export function ContextLensDialog({ open, sessionId, focusSessionId, refreshKey, onClose, onCompact }: {
  open: boolean;
  /** The chat whose windows are shown: itself first, then every worker under it. */
  sessionId: string;
  /** Which window opens selected; defaults to the chat's own. */
  focusSessionId?: string | null;
  /** Changes when the chat's context moved; triggers an immediate refetch. */
  refreshKey?: unknown;
  onClose: () => void;
  /** Compact one window: the chat's own, an orchestrator's, or a worker's. */
  onCompact?: (sessionId: string) => Promise<void>;
}) {
  const { result, unavailable } = useContextWindows(sessionId, open, refreshKey);
  const [selected, setSelected] = useState<string | null>(open ? focusSessionId ?? sessionId : null);
  useResetOnChange([open, focusSessionId, sessionId], () => { if (open) setSelected(focusSessionId ?? sessionId); });
  const windows = result?.windows ?? [];
  const detail = windows.find(window => window.sessionId === selected) ?? windows[0];
  const live = windows.filter(window => window.current).length;
  const several = windows.length > 1;

  return <Dialog open={open} onOpenChange={next => { if (!next) onClose(); }}>
    {/* One window fits a column; several earn the room for a rail beside it. */}
    <DialogContent aria-label="Context lens" className={cn("max-w-2xl", several && "lg:max-w-4xl")}>
      <DialogHeader className="me-6">
        <div className="flex flex-col gap-1">
          <div className="flex flex-wrap items-baseline gap-x-2.5 gap-y-1">
            <DialogTitle>Context lens</DialogTitle>
            {result && <span className="text-meta tabular-nums text-muted-foreground">{live} live · {windows.length} window{windows.length === 1 ? "" : "s"}</span>}
          </div>
          <DialogDescription>How full each window in this chat is, read from its own harness.</DialogDescription>
        </div>
      </DialogHeader>
      <DialogPanel className="@container/lens">
        {!result
          ? (unavailable
            ? <div role="alert" className="rounded-2xl border border-dashed border-border px-4 py-10 text-center">
              <p className="text-ui font-medium text-foreground">Context is unavailable</p>
              <p className="mt-1 text-meta text-muted-foreground">This Bridge daemon does not report context windows yet. Restart Bridge after updating.</p>
            </div>
            : <SkeletonRegion label="Reading context windows">
              <div className="flex flex-col gap-4">
                <Skeleton className="h-44 rounded-2xl" />
                <Skeleton index={1} className="h-3 w-full rounded-full" />
                <SkeletonText index={2} lines={4} />
              </div>
            </SkeletonRegion>)
          : <div className="flex flex-col gap-4 @3xl/lens:flex-row @3xl/lens:items-start @3xl/lens:gap-5">
            {/* A wrapping grid when the lens is narrow, a rail when it is wide.
                Never a sideways scroller: the panel sizes itself to its widest
                child, so one unwrapped row would push every card past the edge. */}
            {several && <div
              role="tablist"
              aria-label="Context windows"
              className="grid max-h-44 shrink-0 grid-cols-[repeat(auto-fill,minmax(8.5rem,1fr))] gap-1 overflow-y-auto rounded-xl border border-border bg-muted p-1 @3xl/lens:sticky @3xl/lens:top-1 @3xl/lens:max-h-none @3xl/lens:w-60 @3xl/lens:grid-cols-1"
            >
              {windows.map(window => <WindowTab key={window.sessionId} window={window} selected={window.sessionId === detail?.sessionId} onSelect={() => setSelected(window.sessionId)} />)}
            </div>}
            <div className="@container min-w-0 flex-1">
              {detail
                ? <WindowDetail window={detail} bridge={result.bridge} earlier={result.earlier} onCompact={onCompact} />
                : <p className="rounded-2xl border border-dashed border-border px-4 py-10 text-center text-meta text-muted-foreground">No window is live in this chat yet.</p>}
            </div>
          </div>}
      </DialogPanel>
    </DialogContent>
  </Dialog>;
}
