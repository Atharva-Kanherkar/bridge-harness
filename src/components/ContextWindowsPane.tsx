// The Context pane: every live context window in this chat (the chat model,
// an orchestrator, each worker), how full it is, what fills it and what
// Bridge adds. Numbers come only from what each harness reports; a window
// that cannot report says why instead of showing a zero.
import { useEffect, useState } from "react";
import { ChevronLeft, ChevronRight, Gauge, Layers } from "lucide-react";
import { PaneState } from "@/components/ui/pane";
import { cn } from "@/lib/utils";
import type { ContextBridgeContribution, ContextReadingState, ContextWindow, ContextWindowReading, EarlierContextWindow } from "../protocol/generated/protocol";
import { contextPressure } from "../usage";
import { compactTokens, contextTone, TONE_TEXT, useContextWindows, windowComposition } from "../contextWindows";
import { harnessLabel, modelLabel } from "../utils";
import { ContextGauge } from "./ContextRing";

const SEGMENT_RAMP = ["bg-foreground/80", "bg-foreground/62", "bg-foreground/48", "bg-foreground/36", "bg-foreground/26", "bg-foreground/18"];
const rampClass = (index: number) => SEGMENT_RAMP[index] ?? "bg-foreground/12";

const PRESSURE_CARD: Record<string, string> = {
  critical: "border-destructive/45 bg-destructive/10",
  high: "border-warning/40 bg-warning/10",
  elevated: "border-border bg-muted/60",
};

function StateBadge({ state }: { state: ContextReadingState }) {
  return <span className={cn(
    "shrink-0 whitespace-nowrap rounded-full border border-border px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-[0.08em] text-muted-foreground",
    state === "estimated" && "border-dashed",
  )}>{state}</span>;
}

function windowTitle(window: ContextWindow): string {
  const role = window.role === "chat" ? "Chat" : window.role === "orchestrator" ? "Orchestrator" : window.label;
  return window.model ? `${role} · ${modelLabel(window.model)}` : role;
}

function percentOf(part: number, whole: number): string {
  if (whole <= 0) return "0%";
  const value = (part / whole) * 100;
  return `${value >= 10 ? Math.round(value) : Math.round(value * 10) / 10}%`;
}

/** One track for the whole window: named segments in an achromatic ramp,
 *  the occupied remainder hatched, headroom left empty. */
export function CompositionBar({ reading, tall = false }: { reading: ContextWindowReading; tall?: boolean }) {
  const composition = windowComposition(reading);
  const width = (tokens: number) => `${(tokens / reading.windowTokens) * 100}%`;
  return <div
    role="img"
    aria-label={`${reading.percent}% of the window is occupied`}
    className={cn("relative flex overflow-hidden rounded-full border border-border bg-muted", tall ? "h-3.5" : "h-2")}
  >
    {composition.used.map((segment, index) => <span key={segment.name} className={cn("h-full", rampClass(index))} style={{ width: width(segment.tokens) }} />)}
    {composition.unattributed > 0 && <span className={cn("h-full", composition.used.length === 0 ? "bg-foreground/45" : "ctx-hatch-neutral")} style={{ width: width(composition.unattributed) }} />}
    {reading.autoCompactTokens != null && reading.autoCompactTokens < reading.windowTokens && <span
      aria-hidden="true"
      className="absolute inset-y-0 w-px bg-ring"
      style={{ left: width(reading.autoCompactTokens) }}
      title={`Auto-compacts at ${compactTokens(reading.autoCompactTokens)}`}
    />}
  </div>;
}

function WindowCard({ window, onOpen }: { window: ContextWindow; onOpen: () => void }) {
  const reading = window.current;
  const harness = harnessLabel(window.harness);
  if (!reading) {
    return <div className="rounded-xl border border-dashed border-border p-3">
      <div className="flex items-center gap-2.5">
        <ContextGauge percent={null} />
        <div className="min-w-0 flex-1">
          <p className="truncate text-[13px] font-medium">{windowTitle(window)}</p>
          <p className="truncate font-mono text-[11px] text-muted-foreground">{harness}</p>
        </div>
        <span className="shrink-0 rounded-full border border-dashed border-border px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-[0.08em] text-muted-foreground">Unavailable</span>
      </div>
      <p className="mt-2 text-[11px] leading-relaxed text-muted-foreground">{window.unavailableReason}</p>
    </div>;
  }
  const tone = contextTone(reading.percent);
  return <button type="button" onClick={onOpen} className="w-full rounded-xl border border-border bg-card p-3 text-left transition-colors hover:border-foreground/25">
    <div className="flex items-center gap-2.5">
      <ContextGauge percent={reading.percent} />
      <div className="min-w-0 flex-1">
        <p className="truncate text-[13px] font-medium">{windowTitle(window)}</p>
        <p className="truncate font-mono text-[11px] text-muted-foreground">{harness} · {compactTokens(reading.windowTokens)} window</p>
      </div>
      <div className="shrink-0 text-right">
        <p className={cn("font-mono text-[13px] tabular-nums", TONE_TEXT[tone])}>{reading.percent}%</p>
        <p className="font-mono text-[11px] tabular-nums text-muted-foreground">{compactTokens(reading.usedTokens)} / {compactTokens(reading.windowTokens)}</p>
      </div>
    </div>
    <div className="mt-2.5"><CompositionBar reading={reading} /></div>
    <div className="mt-2 flex items-center justify-between gap-2">
      <StateBadge state={reading.state} />
      <span className="inline-flex items-center gap-0.5 text-[11px] text-muted-foreground">Details<ChevronRight size={11} aria-hidden="true" /></span>
    </div>
  </button>;
}

function SectionTitle({ children }: { children: React.ReactNode }) {
  return <p className="mb-1.5 mt-4 text-[11px] font-semibold uppercase tracking-[0.13em] text-muted-foreground">{children}</p>;
}

function BridgeShare({ bridge }: { bridge: ContextBridgeContribution }) {
  return <>
    <SectionTitle>What Bridge adds</SectionTitle>
    <div className="rounded-xl border border-border bg-background p-3 text-xs">
      {bridge.stableTokens != null && <div className="flex justify-between gap-2"><span>Stable prompt (cacheable)</span><span className="font-mono tabular-nums text-muted-foreground">~{compactTokens(bridge.stableTokens)}</span></div>}
      {bridge.variableTokens != null && <div className="mt-1.5 flex justify-between gap-2"><span>Variable prompt and briefing</span><span className="font-mono tabular-nums text-muted-foreground">~{compactTokens(bridge.variableTokens)}</span></div>}
      <p className="mt-2 text-[11px] text-muted-foreground">Estimated{bridge.method ? ` (${bridge.method})` : ""} from the last compiled prompt.</p>
    </div>
  </>;
}

function EarlierList({ earlier }: { earlier: EarlierContextWindow[] }) {
  return <div className="mt-4 rounded-xl border border-dashed border-border p-3">
    <p className="text-[11px] font-semibold uppercase tracking-[0.12em] text-muted-foreground">Earlier in this chat</p>
    <ul className="mt-2 space-y-1.5 text-xs">
      {earlier.map(window => <li key={`${window.harness}:${window.model}:${window.observedAt}`} className="flex items-center justify-between gap-2">
        <span className="min-w-0 truncate">{window.model ? modelLabel(window.model) : harnessLabel(window.harness)}</span>
        <span className="shrink-0 font-mono tabular-nums text-muted-foreground">{compactTokens(window.usedTokens)} / {compactTokens(window.windowTokens)} · {window.percent}%</span>
      </li>)}
    </ul>
  </div>;
}

function WindowDetail({ window, bridge, onBack }: { window: ContextWindow; bridge: ContextBridgeContribution | null | undefined; onBack: () => void }) {
  const reading = window.current;
  const harness = harnessLabel(window.harness);
  const header = <div className="mb-3 flex items-center gap-1.5">
    <button type="button" onClick={onBack} className="inline-flex h-7 items-center gap-0.5 rounded-md px-1.5 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"><ChevronLeft size={13} aria-hidden="true" />Windows</button>
    <h2 className="min-w-0 truncate text-sm font-semibold">{windowTitle(window)}</h2>
  </div>;
  if (!reading) {
    return <div>{header}<PaneState icon={Gauge} title="No reading">{window.unavailableReason}</PaneState></div>;
  }
  const pressure = contextPressure(reading.percent);
  const composition = windowComposition(reading);
  const owner = reading.compactionOwner === "harness" ? `${harness} compacts this window itself. A Bridge checkpoint is saved separately and does not shrink it.` : "This harness does not compact its own window; Bridge checkpoints it when pressure is high.";
  return <div>
    {header}
    <section aria-label="Context pressure" className={cn("rounded-xl border p-3", PRESSURE_CARD[pressure.level] ?? "border-border")}>
      <div className="flex items-center gap-2">
        <ContextGauge percent={reading.percent} size={22} />
        <b className={cn("text-[13px]", TONE_TEXT[pressure.level])}>{pressure.label}</b>
        <span className="font-mono text-[13px] tabular-nums">{reading.percent}%</span>
        <span className="ml-auto font-mono text-[11px] tabular-nums text-muted-foreground">{compactTokens(reading.usedTokens)} / {compactTokens(reading.windowTokens)}</span>
      </div>
      <p className="mt-1.5 text-[11px] leading-relaxed text-muted-foreground">{pressure.explanation}</p>
      <div className="mt-3"><CompositionBar reading={reading} tall /></div>
      <div className="mt-1.5 flex flex-wrap justify-between gap-x-3 gap-y-1 font-mono text-[11px] text-muted-foreground">
        <span>{reading.autoCompactTokens != null ? `Auto-compacts at ~${compactTokens(reading.autoCompactTokens)}` : `${compactTokens(composition.free)} free`}</span>
        <StateBadge state={reading.state} />
      </div>
    </section>

    {reading.forecast && <p className="mt-2 flex items-center gap-2 text-[11px] text-muted-foreground">
      <span>About <b className="font-semibold text-foreground">{reading.forecast.turnsRemaining} turn{reading.forecast.turnsRemaining === 1 ? "" : "s"}</b> until {reading.autoCompactTokens != null ? "it compacts" : "the window is full"}, at ~{compactTokens(reading.forecast.growthPerTurn)} per turn.</span>
      <StateBadge state="estimated" />
    </p>}

    <SectionTitle>What fills it</SectionTitle>
    {composition.used.length === 0
      ? <p className="rounded-lg border border-dashed border-border px-2.5 py-2 text-[11px] leading-relaxed text-muted-foreground">{harness} reports how full the window is but not what is in it.</p>
      : <div className="grid gap-1">
        {composition.used.map((segment, index) => <div key={segment.name} className="flex items-center gap-2 rounded-lg border border-border px-2.5 py-2">
          <span className={cn("size-2 shrink-0 rounded-[3px]", rampClass(index))} />
          <span className="min-w-0 flex-1 truncate text-xs font-medium">{segment.name}</span>
          <span className="font-mono text-xs tabular-nums">{compactTokens(segment.tokens)}</span>
          <span className="w-10 text-right font-mono text-[11px] tabular-nums text-muted-foreground">{percentOf(segment.tokens, reading.windowTokens)}</span>
        </div>)}
        {composition.unattributed > 0 && <div className="flex items-center gap-2 rounded-lg border border-dashed border-border px-2.5 py-2">
          <span className="ctx-hatch-neutral size-2 shrink-0 rounded-[3px]" />
          <span className="min-w-0 flex-1 truncate text-xs font-medium">Not attributed</span>
          <span className="font-mono text-xs tabular-nums">{compactTokens(composition.unattributed)}</span>
          <span className="w-10 text-right font-mono text-[11px] tabular-nums text-muted-foreground">{percentOf(composition.unattributed, reading.windowTokens)}</span>
        </div>}
      </div>}

    {reading.consumers.length > 0 && <>
      <SectionTitle>Biggest consumers</SectionTitle>
      <ul className="space-y-0.5 text-xs">
        {reading.consumers.map(consumer => <li key={consumer.label} className="flex items-center justify-between gap-2 rounded-md px-1.5 py-1 hover:bg-accent">
          <span className="min-w-0 truncate">{consumer.label}</span>
          <span className="shrink-0 font-mono tabular-nums text-muted-foreground">{compactTokens(consumer.tokens)}{consumer.detail && <span className="ml-1.5 text-[11px]">{consumer.detail}</span>}</span>
        </li>)}
      </ul>
    </>}

    {window.role !== "worker" && bridge && <BridgeShare bridge={bridge} />}
    <p className="mt-4 text-[11px] leading-relaxed text-muted-foreground">{owner}</p>
  </div>;
}

export function ContextWindowsPane({ sessionId, visible, refreshKey, detailRequest }: {
  sessionId: string;
  visible: boolean;
  /** Changes when the chat's context moved; triggers an immediate refetch. */
  refreshKey?: unknown;
  /** Open a window's detail from outside (the composer ring). */
  detailRequest?: { sessionId: string; nonce: number };
}) {
  const { result, unavailable } = useContextWindows(sessionId, visible, refreshKey);
  const [selected, setSelected] = useState<string | null>(null);
  useEffect(() => { setSelected(null); }, [sessionId]);
  useEffect(() => { if (detailRequest) setSelected(detailRequest.sessionId); }, [detailRequest]);

  if (!result) {
    return unavailable
      ? <PaneState icon={Layers} title="Context is unavailable" role="alert">This Bridge daemon does not report context windows yet. Restart Bridge after updating.</PaneState>
      : <PaneState icon={Layers} title="Reading context windows" role="status" />;
  }
  const detail = selected ? result.windows.find(window => window.sessionId === selected) : undefined;
  return <div className="flex h-full flex-col">
    <div className="min-h-0 flex-1 overflow-y-auto p-4">
      {detail
        ? <WindowDetail window={detail} bridge={result.bridge} onBack={() => setSelected(null)} />
        : <>
          <div className="mb-3 flex items-center justify-between gap-2">
            <h2 className="text-sm font-semibold">Context windows</h2>
            <span className="font-mono text-[11px] text-muted-foreground">{result.windows.length} live</span>
          </div>
          <div className="grid gap-2.5">
            {result.windows.map(window => <WindowCard key={window.sessionId} window={window} onOpen={() => setSelected(window.sessionId)} />)}
          </div>
          {result.earlier.length > 0 && <EarlierList earlier={result.earlier} />}
          {result.bridge && <BridgeShare bridge={result.bridge} />}
          <p className="mt-4 text-[11px] leading-relaxed text-muted-foreground">Each window is read from its own harness. A window that cannot report says why; it is never drawn as 0%.</p>
        </>}
    </div>
  </div>;
}
