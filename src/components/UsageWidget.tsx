import { memo, useEffect, useRef, useState } from "react";
import { AlertTriangle, Gauge, X } from "lucide-react";
import { clampPercent, contextPressure, formatReset, projectUsageExhaustion, type MetricSource, type UsageHistoryEntry, type UsageProvider, type UsageRateSample, type UsageSnapshot } from "../usage";

const PROVIDERS: Array<{ id: UsageProvider; label: string }> = [
  { id: "codex", label: "Codex" },
  { id: "claude", label: "Claude" },
  { id: "opencode", label: "OpenCode" },
];

export interface UsageWidgetProps {
  usage: Partial<Record<UsageProvider, UsageSnapshot>>;
  samples?: Partial<Record<UsageProvider, UsageRateSample[]>>;
  history?: UsageHistoryEntry[];
  contextPercent?: number;
  contextSource?: MetricSource;
}

function sourceLabel(source: MetricSource): string {
  return source.charAt(0).toUpperCase() + source.slice(1);
}

function SourceBadge({ source }: { source: MetricSource }) {
  return <span className="rounded-full border border-white/[0.09] px-1.5 py-0.5 text-[8px] font-semibold uppercase tracking-[0.08em] text-neutral-500">{sourceLabel(source)}</span>;
}

function highestUse(snapshot?: UsageSnapshot): number | undefined {
  if (!snapshot?.windows.length) return undefined;
  return clampPercent(Math.max(...snapshot.windows.map(window => window.usedPercent)));
}

function UsageRing({ used }: { used?: number }) {
  const clamped = used == null ? 0 : clampPercent(used);
  const radius = 7;
  const circumference = 2 * Math.PI * radius;
  return <svg width="18" height="18" viewBox="0 0 18 18" className="shrink-0 -rotate-90" aria-hidden="true">
    <circle cx="9" cy="9" r={radius} fill="none" strokeWidth="2" stroke="currentColor" className="text-white/[0.1]" />
    {used != null && <circle cx="9" cy="9" r={radius} fill="none" strokeWidth="2" strokeLinecap="round" stroke="currentColor" strokeDasharray={circumference} strokeDashoffset={circumference * (1 - clamped / 100)} className="text-neutral-300 transition-[stroke-dashoffset] duration-700 ease-out" />}
  </svg>;
}

function UsageBar({ used }: { used: number }) {
  const clamped = clampPercent(used);
  return <span className="block h-1.5 w-full overflow-hidden rounded-full bg-white/[0.07]">
    <span className="block h-full rounded-full bg-gradient-to-r from-neutral-600 to-neutral-200 transition-[width] duration-700 ease-out" style={{ width: `${clamped}%` }} />
  </span>;
}

export const UsageWidget = memo(function UsageWidget({ usage, samples = {}, history = [], contextPercent, contextSource = "measured" }: UsageWidgetProps) {
  const [open, setOpen] = useState(false);
  const [dismissed, setDismissed] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const pressure = contextPressure(contextPercent);
  const projections = PROVIDERS.map(provider => {
    const projection = projectUsageExhaustion(samples[provider.id] ?? []);
    return projection ? { provider, projection } : null;
  }).filter((value): value is NonNullable<typeof value> => value != null);

  useEffect(() => {
    if (!open) return;
    const dismiss = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false);
    };
    const dismissOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("pointerdown", dismiss);
    document.addEventListener("keydown", dismissOnEscape);
    return () => {
      document.removeEventListener("pointerdown", dismiss);
      document.removeEventListener("keydown", dismissOnEscape);
    };
  }, [open]);

  if (dismissed) return null;

  return <div ref={rootRef} className="relative">
    <button type="button" className="flex h-9 cursor-pointer items-center gap-2 rounded-full border border-white/[0.08] bg-white/[0.045] py-0 pl-3 pr-9 text-neutral-300 shadow-[inset_0_1px_0_rgba(255,255,255,0.05)] backdrop-blur-xl backdrop-saturate-[1.8] transition-all hover:border-white/[0.14] hover:bg-white/[0.075]" aria-label={open ? "Close usage health details" : "Open usage health details"} aria-expanded={open} aria-controls="usage-health-panel" onClick={() => setOpen(value => !value)}>
      <Gauge size={12} className="text-neutral-500" aria-hidden="true" />
      {PROVIDERS.map((provider, index) => {
        const snapshot = usage[provider.id];
        const used = highestUse(snapshot);
        return <div key={provider.id} className="flex items-center gap-1.5">
          {index > 0 && <span className="mr-0.5 h-4 w-px bg-white/[0.08]" aria-hidden="true" />}
          <UsageRing used={used} />
          <span className="text-[10px] font-medium">{provider.label}</span>
          <span className="font-mono text-[9px] tabular-nums text-neutral-500">{used == null ? "unknown" : `${Math.round(used)}% · ${snapshot?.source}`}</span>
        </div>;
      })}
      <span className="h-4 w-px bg-white/[0.08]" aria-hidden="true" />
      <span className="text-[9px] text-neutral-500">Ctx {pressure.percent == null ? "unknown" : `${Math.round(pressure.percent)}% · ${contextSource}`}</span>
      {projections.length > 0 && <AlertTriangle size={12} className="text-amber-300" aria-label="Projected usage exhaustion" />}
    </button>
    <button type="button" onClick={() => { setOpen(false); setDismissed(true); }} className="absolute right-1.5 top-1/2 z-10 grid size-6 -translate-y-1/2 place-items-center rounded-full text-neutral-600 transition-colors hover:bg-white/[0.09] hover:text-neutral-200" aria-label="Hide usage widget"><X size={12} aria-hidden="true" /></button>

    <div id="usage-health-panel" role="dialog" aria-label="Usage health details" className={`absolute right-0 top-full z-50 pt-2 transition-all duration-150 ${open ? "visible pointer-events-auto opacity-100" : "invisible pointer-events-none opacity-0"}`}>
      <div className="u-glass-popover max-h-[min(640px,80vh)] w-[390px] overflow-y-auto rounded-2xl p-4">
        <div className="mb-3 flex items-center gap-2">
          <Gauge size={13} className="text-neutral-500" aria-hidden="true" />
          <h2 className="font-display text-sm font-semibold text-neutral-100">Usage health</h2>
          <span className="ml-auto text-[9px] text-neutral-600">No invented limits</span>
          <button type="button" onClick={() => setOpen(false)} className="grid size-6 place-items-center rounded-md text-neutral-500 transition-colors hover:bg-white/[0.07] hover:text-neutral-200" aria-label="Close usage health details"><X size={13} aria-hidden="true" /></button>
        </div>

        <div className="grid gap-3">
          {PROVIDERS.map(provider => <ProviderDetail key={provider.id} provider={provider} snapshot={usage[provider.id]} samples={samples[provider.id] ?? []} />)}
        </div>

        <section className="mt-3 rounded-xl border border-white/[0.07] bg-white/[0.025] p-3" aria-label="Context pressure">
          <div className="flex items-center gap-2">
            <b className="text-[11px] text-neutral-200">{pressure.label}</b>
            {pressure.percent != null && <span className="font-mono text-[10px] text-neutral-400">{Math.round(pressure.percent)}%</span>}
            {pressure.percent != null && <SourceBadge source={contextSource} />}
          </div>
          <p className="mt-1.5 text-[10px] leading-relaxed text-neutral-500">{pressure.explanation}</p>
        </section>

        <section className="mt-4" aria-label="Usage history">
          <div className="mb-2 flex items-center justify-between">
            <h3 className="text-[9px] font-semibold uppercase tracking-[0.13em] text-neutral-500">Recent work units</h3>
            <span className="text-[9px] text-neutral-600">Newest first</span>
          </div>
          {history.length ? <div className="grid gap-1.5">{history.slice(0, 6).map(entry => <HistoryRow key={entry.id} entry={entry} />)}</div> : <p className="rounded-xl border border-dashed border-white/[0.08] px-3 py-4 text-center text-[10px] text-neutral-600">No measured work-unit history yet.</p>}
        </section>
      </div>
    </div>
  </div>;
});

function ProviderDetail({ provider, snapshot, samples }: { provider: { id: UsageProvider; label: string }; snapshot?: UsageSnapshot; samples: UsageRateSample[] }) {
  const used = highestUse(snapshot);
  const projection = projectUsageExhaustion(samples);
  return <section className="rounded-xl border border-white/[0.07] bg-white/[0.025] p-3" aria-label={`${provider.label} usage`}>
    <div className="flex items-center gap-2">
      <UsageRing used={used} />
      <b className="text-[11px] text-neutral-200">{provider.label}</b>
      {snapshot?.planType && <span className="text-[9px] text-neutral-500">{snapshot.planType}</span>}
      {snapshot?.model && <span className="truncate font-mono text-[9px] text-neutral-500">{snapshot.model}</span>}
      <span className="ml-auto">{snapshot ? <SourceBadge source={snapshot.source} /> : <span className="text-[9px] text-neutral-600">Limit unknown</span>}</span>
    </div>
    {snapshot?.windows.length ? <div className="mt-2.5 grid gap-2.5">{snapshot.windows.map(window => {
      const clamped = clampPercent(window.usedPercent);
      const reset = window.resetsLabel ?? formatReset(window.resetsInSeconds);
      return <div key={window.id}>
        <div className="mb-1 flex items-center gap-2 text-[10px]"><span className="text-neutral-500">{window.label}</span><span className="ml-auto font-mono text-neutral-300">{Math.round(clamped)}% used</span><SourceBadge source={window.source} /></div>
        <UsageBar used={clamped} />
        <div className="mt-1 text-[9px] text-neutral-600">{reset ?? "Reset unknown"}</div>
      </div>;
    })}</div> : <p className="mt-2 text-[10px] leading-relaxed text-neutral-500">Limit unknown — this provider has not exposed a stable quota value. Token and context history remain available below.</p>}
    {projection && <div className="mt-2.5 flex gap-2 rounded-lg border border-amber-300/15 bg-amber-300/[0.04] p-2 text-[9.5px] leading-relaxed text-amber-100/70"><AlertTriangle size={12} className="mt-0.5 shrink-0" aria-hidden="true" /><span>{projection.explanation} <b className="font-semibold uppercase tracking-wide">Estimated</b></span></div>}
  </section>;
}

function HistoryRow({ entry }: { entry: UsageHistoryEntry }) {
  return <div className="rounded-xl border border-white/[0.06] px-3 py-2">
    <div className="flex min-w-0 items-center gap-2">
      <span className="min-w-0 flex-1 truncate font-mono text-[9.5px] text-neutral-300" title={entry.workUnit}>{entry.workUnit}</span>
      <SourceBadge source={entry.source} />
    </div>
    <div className="mt-1 flex flex-wrap gap-x-2 gap-y-1 text-[9px] text-neutral-600">
      <span>{entry.harness}</span>
      <span>{entry.model ?? "model unknown"}</span>
      <span>{entry.outcome}</span>
      {entry.totalTokens != null && <span>{entry.totalTokens.toLocaleString()} tokens</span>}
      {entry.contextPercent != null && <span>{entry.contextPercent}% context</span>}
    </div>
  </div>;
}
