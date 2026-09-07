import { useState } from "react";
import { ArrowUpRight, ChevronRight, ExternalLink, Layers, X } from "lucide-react";
import { cn } from "@/lib/utils";
import type { ContextBreakdownResult, ContextBreakdownState as WireSourceState } from "../protocol/generated/protocol";
import {
  breakdownMath,
  deltaSummary,
  formatCompactTokens,
  formatTokens,
  rankSegments,
  useContextBreakdown,
} from "../contextBreakdown";
import { contextPressure } from "../usage";

const SEGMENT_RAMP = ["bg-foreground/80", "bg-foreground/65", "bg-foreground/50", "bg-foreground/38", "bg-foreground/28", "bg-foreground/20"];
const RAMP_FALLBACK = "bg-foreground/12";

function rampClass(index: number): string {
  return SEGMENT_RAMP[index] ?? RAMP_FALLBACK;
}

function percentLabel(part: number, whole: number): string {
  if (whole <= 0) return "0%";
  const percent = (part / whole) * 100;
  return `${percent >= 10 ? Math.round(percent) : Math.round(percent * 10) / 10}%`;
}

export function SourceStateBadge({ state }: { state: WireSourceState }) {
  const dashed = state === "estimated" || state === "unavailable";
  return <span
    className={cn(
      "shrink-0 whitespace-nowrap rounded-full border px-1.5 py-0.5 text-[11px] font-semibold uppercase tracking-[0.08em]",
      dashed ? "border-dashed text-muted-foreground" : "text-muted-foreground",
      state === "unavailable"
        ? "border-destructive/40 text-destructive/80"
        : "border-border",
    )}>{state.charAt(0).toUpperCase() + state.slice(1)}</span>;
}

export interface ContextBreakdownPanelProps {
  state: ReturnType<typeof useContextBreakdown>;
  sessionId: string | null | undefined;
  onClose: () => void;
  onOpenPromptStudio?: (segmentClass: string) => void;
}

export function ContextBreakdownPanel({ state, sessionId, onClose, onOpenPromptStudio }: ContextBreakdownPanelProps) {
  const { result } = state;
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const toggle = (id: string) => setExpanded(current => {
    const next = new Set(current);
    if (next.has(id)) next.delete(id); else next.add(id);
    return next;
  });

  return <div className="grid gap-2.5" aria-label="Context breakdown">
    <div className="flex items-center gap-2 px-0.5">
      <Layers size={13} className="shrink-0 text-muted-foreground" aria-hidden="true" />
      <h2 className="font-display text-sm font-semibold text-foreground">Context breakdown</h2>
      <span className="ml-auto truncate font-mono text-[11px] text-muted-foreground">
        {state.reconciledAt ? `reconciled ${new Date(state.reconciledAt).toLocaleTimeString()}` : "reconciling…"}
      </span>
      <button type="button" onClick={onClose} className="grid size-7 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground" aria-label="Back to usage health"><X size={13} aria-hidden="true" /></button>
    </div>

    {!result ? <EmptyCard unavailable={state.unavailable} /> : <>
      <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 px-0.5">
        <span className="min-w-0 truncate font-mono text-[11px] text-muted-foreground" title={sessionId ?? undefined}>{result.sessionId}</span>
        {result.conversation.model && <span className="truncate font-mono text-[11px] text-muted-foreground">{result.conversation.model}</span>}
        {result.conversation.effort && <span className="shrink-0 rounded-full border border-border px-1.5 py-0.5 text-[11px] font-semibold uppercase tracking-[0.08em] text-muted-foreground">{result.conversation.effort}</span>}
        {result.totals.unavailableSources > 0 && <span className="ml-auto shrink-0 text-[11px] text-muted-foreground">{result.totals.unavailableSources} unavailable sources</span>}
      </div>

      <PressureCard result={result} />
      <CompositionBar result={result} />
      <SegmentRows result={result} expanded={expanded} onToggle={toggle} onOpenPromptStudio={onOpenPromptStudio} />
      <DeltaStrip result={result} />

      <p className="px-0.5 text-[11px] leading-relaxed text-muted-foreground">
        Hatched area is material this harness cannot observe. Unavailable is not zero — hidden instructions, tool payloads, and plugins still consume the window.
      </p>
    </>}
  </div>;
}

function EmptyCard({ unavailable }: { unavailable: boolean }) {
  return <p className={cn("border border-dashed border-border px-3 py-6 text-center text-[11px] leading-relaxed text-muted-foreground", "rounded-[calc(var(--radius-2xl)-0.625rem)]")}>
    {unavailable
      ? "No breakdown is available for this session right now. Polling stopped after repeated misses."
      : "No breakdown recorded yet — reconciling with the focused session…"}
  </p>;
}

function PressureCard({ result }: { result: ContextBreakdownResult }) {
  const percent = result.conversation.contextWindowTokens > 0
    ? (result.conversation.tokenEstimate / result.conversation.contextWindowTokens) * 100
    : undefined;
  const pressure = contextPressure(percent);
  return <section
    aria-label="Context pressure"
    className={cn(
      "border p-3 transition-colors duration-300",
      "rounded-[calc(var(--radius-2xl)-0.625rem)]",
      pressure.level === "critical" && "border-destructive/45 bg-destructive/10",
      pressure.level === "high" && "border-warning/40 bg-warning/10",
      pressure.level === "elevated" && "border-warning/30 bg-warning/[0.06]",
    )}>
    <div className="flex flex-wrap items-center gap-2">
      <b className={cn(
        "text-[11px]",
        pressure.level === "critical" ? "text-destructive" : pressure.level === "high" ? "text-warning" : "text-foreground",
      )}>{pressure.label}</b>
      {pressure.percent != null && <span className="font-mono text-[11px] tabular-nums text-muted-foreground">{Math.round(pressure.percent)}%</span>}
      <span className="shrink-0 whitespace-nowrap rounded-full border border-dashed border-border px-1.5 py-0.5 text-[11px] font-semibold uppercase tracking-[0.08em] text-muted-foreground">Estimated</span>
      <span className="ml-auto font-mono text-[11px] text-muted-foreground">of ~{formatCompactTokens(result.conversation.contextWindowTokens)} window</span>
    </div>
    <p className="mt-1.5 text-[11px] leading-relaxed text-muted-foreground">{pressure.explanation}</p>
  </section>;
}

function CompositionBar({ result }: { result: ContextBreakdownResult }) {
  const math = breakdownMath(result);
  const width = (tokens: number) => `${(tokens / math.windowTokens) * 100}%`;
  const ranked = rankSegments(result.segments);
  let rampIndex = -1;
  const delta = result.compactionDelta;
  return <div className="px-0.5">
    <div className="relative flex h-3.5 overflow-hidden rounded-full border border-border bg-muted" role="img" aria-label={`Context window composition: ${formatTokens(math.knownTokens)} of ${formatTokens(math.windowTokens)} tokens attributed`} title={`${formatCompactTokens(math.occupiedTokens)} of ${formatCompactTokens(math.windowTokens)} tok occupied`}>
      {ranked.map(({ segment }) => {
        if (segment.state === "unavailable" || segment.tokens == null) return null;
        rampIndex += 1;
        return <span key={`${segment.origin}:${segment.segmentClass}`} className={cn("h-full", rampClass(rampIndex))} style={{ width: width(segment.tokens) }} />;
      })}
      {math.unattributedTokens > 0 && <span className="ctx-hatch h-full" style={{ width: width(math.unattributedTokens) }} />}
      {delta && <span
        className="absolute inset-y-[-3px] w-[2px] rounded-sm bg-ring"
        style={{ left: `${Math.min(100, (delta.tokensBefore / math.windowTokens) * 100)}%` }}
        title={`Last checkpoint · ${formatTokens(delta.tokensBefore)} tok before`}
      />}
    </div>
    <div className="mt-1 flex items-center justify-between font-mono text-[11px] tabular-nums text-muted-foreground">
      <span>{formatTokens(math.knownTokens)} tok attributed</span>
      <span>{formatCompactTokens(math.freeTokens)} free</span>
    </div>
  </div>;
}

function SegmentRows({
  result,
  expanded,
  onToggle,
  onOpenPromptStudio,
}: {
  result: ContextBreakdownResult;
  expanded: Set<string>;
  onToggle: (id: string) => void;
  onOpenPromptStudio?: (segmentClass: string) => void;
}) {
  const math = breakdownMath(result);
  const ranked = rankSegments(result.segments);
  let rampIndex = -1;
  return <div className="grid gap-1">
    <div className="px-0.5 pb-0.5 text-[11px] font-semibold uppercase tracking-[0.13em] text-muted-foreground">Segments · ranked by size</div>
    {ranked.map(entry => {
      const { segment, meta } = entry;
      const id = `${segment.origin}:${segment.segmentClass}`;
      const unavailable = segment.state === "unavailable";
      const sized = !unavailable && segment.tokens != null;
      if (sized) rampIndex += 1;
      const isOpen = expanded.has(id);
      const expandable = (segment.names?.length ?? 0) > 0 || meta.editable;
      return <div key={id} className={cn("overflow-hidden border border-border bg-card transition-colors hover:border-foreground/20", "rounded-[calc(var(--radius-2xl)-0.625rem)]", unavailable && "border-dashed bg-transparent")}>
        <div className="flex min-w-0 flex-wrap items-center gap-2 px-2.5 py-2">
          {expandable
            ? <button type="button" onClick={() => onToggle(id)} aria-expanded={isOpen} aria-label={`${isOpen ? "Collapse" : "Expand"} ${meta.label}`} className="grid size-7 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"><ChevronRight size={10} className={cn("transition-transform duration-150", isOpen && "rotate-90")} aria-hidden="true" /></button>
            : <span className="w-[10px] shrink-0" aria-hidden="true" />}
          <span className={cn("size-2 shrink-0 rounded-[3px]", sized ? rampClass(rampIndex) : "ctx-hatch")} />
          <span className="min-w-0 truncate text-[11px] font-medium">{meta.label}</span>
          {meta.editable && onOpenPromptStudio && <button
            type="button"
            onClick={() => onOpenPromptStudio(segment.segmentClass)}
            className="ml-auto inline-flex shrink-0 items-center gap-1 min-h-7 rounded-md px-2 text-[11px] font-medium text-ring transition-colors hover:bg-accent"
          >Edit in Prompt Studio<ExternalLink size={9} aria-hidden="true" /></button>}
          <span className={cn("whitespace-nowrap font-mono text-[11px] tabular-nums", meta.editable && onOpenPromptStudio ? "" : "ml-auto", sized ? "text-foreground" : "text-muted-foreground")}>
            {sized ? formatTokens(segment.tokens!) : "—"}
          </span>
          <span className="w-9 shrink-0 text-right font-mono text-[11px] tabular-nums text-muted-foreground">
            {sized ? percentLabel(segment.tokens!, math.windowTokens) : ""}
          </span>
          <SourceStateBadge state={segment.state} />
        </div>
        {unavailable && segment.reason && <p className="px-2.5 pb-2 pl-[26px] text-[11px] leading-relaxed text-muted-foreground">{segment.reason}</p>}
        {isOpen && segment.names && segment.names.length > 0 && <ul className="grid gap-1 px-2.5 pb-2 pl-[26px]">
          {segment.names.map(name => (
            <li key={name} className="truncate rounded-md px-1.5 py-1 text-[11px] text-muted-foreground hover:bg-accent" title={name}>{name}</li>
          ))}
        </ul>}
        {isOpen && meta.editable && segment.names?.length === 0 && <p className="px-2.5 pb-2 pl-[26px] text-[11px] leading-relaxed text-muted-foreground">Compiled sections live in Prompt Studio; per-section sizes are not reported separately.</p>}
      </div>;
    })}
  </div>;
}

function DeltaStrip({ result }: { result: ContextBreakdownResult }) {
  const summary = deltaSummary(result);
  if (!summary) return null;
  return <div className="flex items-center gap-2 rounded-[calc(var(--radius-2xl)-0.625rem)] border border-dashed border-border px-2.5 py-2 text-[11px] text-muted-foreground">
    <ArrowUpRight size={12} className="shrink-0 text-warning" aria-hidden="true" />
    <span>Since last checkpoint: <b className="font-semibold text-foreground">{summary.text}</b></span>
    {summary.detail && <span className="min-w-0 truncate text-muted-foreground">({summary.detail})</span>}
  </div>;
}
