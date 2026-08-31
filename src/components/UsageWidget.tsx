import { memo, useEffect, useRef, useState } from "react";
import { AlertTriangle, Gauge, Layers, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { bridgeApi } from "../api";
import { clampPercent, contextPressure, formatReset, projectUsageExhaustion, type CacheDiagnostic, type MetricSource, type UsageHistoryEntry, type UsageProvider, type UsageRateSample, type UsageSnapshot } from "../usage";
import type { AdapterDescriptor } from "../types";
import { useContextBreakdown } from "../contextBreakdown";
import { ContextBreakdownPanel } from "./ContextBreakdown";

/** Details panel padding; inner cards use panel radius minus this so the arcs share a center. */
const PANEL_PAD = "p-2.5";
const PANEL_NESTED = "rounded-[calc(var(--radius-2xl)-0.625rem)]";

const PROVIDERS: Array<{ id: UsageProvider; label: string }> = [
  { id: "codex", label: "Codex" },
  { id: "claude", label: "Claude" },
  { id: "cursor", label: "Cursor" },
  { id: "opencode", label: "OpenCode" },
];

export interface UsageWidgetProps {
  usage: Partial<Record<UsageProvider, UsageSnapshot>>;
  adapters?: AdapterDescriptor[];
  samples?: Partial<Record<UsageProvider, UsageRateSample[]>>;
  history?: UsageHistoryEntry[];
  cacheDiagnostics?: CacheDiagnostic[];
  contextPercent?: number;
  contextSource?: MetricSource;
  focusedSessionId?: string | null;
  onOpenPromptStudio?: (segmentClass: string) => void;
}

function sourceLabel(source: MetricSource): string {
  return source.charAt(0).toUpperCase() + source.slice(1);
}

function SourceBadge({ source }: { source: MetricSource }) {
  return <span className="shrink-0 whitespace-nowrap rounded-full border border-border px-1.5 py-0.5 text-[8px] font-semibold uppercase tracking-[0.08em] text-muted-foreground">{sourceLabel(source)}</span>;
}

function highestUse(snapshot?: UsageSnapshot): number | undefined {
  if (!snapshot?.windows.length) return undefined;
  return clampPercent(Math.max(...snapshot.windows.map(window => window.usedPercent)));
}

/** A named auth state wins over install status, because an adapter can only
 *  report signed_out about a CLI it found: Cursor probes by opening a session,
 *  so a signed-out Cursor is unavailable and signed_out at once, and reading
 *  availability first would send the user to Settings instead of to Sign in.
 *  Everything else falls back to install status; a signed-in adapter with no
 *  snapshot yet stays "normal" so it keeps the existing unknown-quota look. */
type ProviderStatus = "not_installed" | "signed_out" | "normal";

function providerStatus(adapter?: AdapterDescriptor): ProviderStatus {
  if (!adapter) return "normal";
  if (adapter.authState === "signed_out") return "signed_out";
  if (!adapter.available) return "not_installed";
  return "normal";
}

function UsageRing({ used, inert = false }: { used?: number; inert?: boolean }) {
  const clamped = used == null ? 0 : clampPercent(used);
  const radius = 7;
  const circumference = 2 * Math.PI * radius;
  return <svg width="18" height="18" viewBox="0 0 18 18" className="shrink-0 -rotate-90" aria-hidden="true">
    <circle cx="9" cy="9" r={radius} fill="none" strokeWidth="2" stroke="currentColor" className={inert ? "text-muted-foreground/25" : "text-foreground/15"} />
    {used != null && !inert && <circle cx="9" cy="9" r={radius} fill="none" strokeWidth="2" strokeLinecap="round" stroke="currentColor" strokeDasharray={circumference} strokeDashoffset={circumference * (1 - clamped / 100)} className="text-foreground transition-[stroke-dashoffset] duration-700 ease-out" />}
  </svg>;
}

/** Worst-case usage across every provider that actually reports one — the
 *  single number the compact indicator colors itself by. Providers that are
 *  signed out, not installed, or simply haven't reported yet stay out of the
 *  computation rather than being treated as 0% used. */
function overallUsedPercent(usage: Partial<Record<UsageProvider, UsageSnapshot>>, adapters?: AdapterDescriptor[]): number | null {
  const values = PROVIDERS
    .filter(provider => providerStatus(adapters?.find(item => item.id === provider.id)) === "normal")
    .map(provider => highestUse(usage[provider.id]))
    .filter((value): value is number => value != null);
  return values.length ? Math.max(...values) : null;
}

type UsageTier = "unknown" | "ok" | "warning" | "critical";

function usageTier(percent: number | null): UsageTier {
  if (percent == null) return "unknown";
  if (percent >= 90) return "critical";
  if (percent >= 70) return "warning";
  return "ok";
}

const TIER_RING_CLASS: Record<UsageTier, string> = {
  unknown: "text-muted-foreground/35",
  ok: "text-success",
  warning: "text-warning",
  critical: "text-destructive",
};

/** The compact trigger: a single colorful ring sized to sit beside the
 *  composer's send button, rather than a labelled strip competing with it. */
function UsageIndicatorRing({ percent, tier }: { percent: number | null; tier: UsageTier }) {
  const clamped = percent == null ? 0 : clampPercent(percent);
  const radius = 8;
  const circumference = 2 * Math.PI * radius;
  return <svg width="20" height="20" viewBox="0 0 20 20" className="shrink-0 -rotate-90" aria-hidden="true">
    <circle cx="10" cy="10" r={radius} fill="none" strokeWidth="2.25" stroke="currentColor" className="text-foreground/10" />
    {percent != null && <circle cx="10" cy="10" r={radius} fill="none" strokeWidth="2.25" strokeLinecap="round" stroke="currentColor" strokeDasharray={circumference} strokeDashoffset={circumference * (1 - clamped / 100)} className={cn(TIER_RING_CLASS[tier], "transition-[stroke-dashoffset,color] duration-700 ease-out")} />}
  </svg>;
}

function UsageBar({ used }: { used: number }) {
  const clamped = clampPercent(used);
  return <span className="block h-1.5 w-full overflow-hidden rounded-full bg-muted">
    <span className="block h-full rounded-full bg-foreground transition-[width] duration-700 ease-out" style={{ width: `${clamped}%` }} />
  </span>;
}

export const UsageWidget = memo(function UsageWidget({ usage, adapters, samples = {}, history = [], cacheDiagnostics = [], contextPercent, contextSource = "measured", focusedSessionId = null, onOpenPromptStudio }: UsageWidgetProps) {
  const [open, setOpen] = useState(false);
  const [showBreakdown, setShowBreakdown] = useState(false);
  const [activeLogin, setActiveLogin] = useState<UsageProvider | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const pressure = contextPressure(contextPercent);
  const breakdownState = useContextBreakdown(focusedSessionId, open && showBreakdown);
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

  const overall = overallUsedPercent(usage, adapters);
  const tier = usageTier(overall);
  const indicatorTitle = overall == null ? "Usage health — no reported usage yet" : `Usage health — ${Math.round(overall)}% used`;

  return <div ref={rootRef} className="relative">
    <button
      type="button"
      className={cn(
        "relative grid size-8 shrink-0 cursor-pointer place-items-center rounded-full border border-border bg-card transition-colors hover:bg-accent",
        open && "bg-accent",
      )}
      aria-label={open ? "Close usage health details" : "Open usage health details"}
      aria-expanded={open}
      aria-controls="usage-health-panel"
      title={indicatorTitle}
      onClick={() => setOpen(value => !value)}
    >
      <UsageIndicatorRing percent={overall} tier={tier} />
      {projections.length > 0 && <span className="absolute -right-0.5 -top-0.5 grid size-3.5 place-items-center rounded-full bg-warning text-warning-foreground" aria-label="Projected usage exhaustion"><AlertTriangle size={9} strokeWidth={2.5} aria-hidden="true" /></span>}
    </button>

    <div id="usage-health-panel" role="dialog" aria-label="Usage health details" className={`absolute bottom-full right-0 z-50 pb-2 transition-all duration-150 ${open ? "visible pointer-events-auto opacity-100" : "invisible pointer-events-none opacity-0"}`}>
      <div className={cn("u-overlay-strong max-h-[80dvh] w-[390px] max-w-[calc(100vw-1.5rem)] overflow-y-auto rounded-2xl", PANEL_PAD)}>
        {showBreakdown && focusedSessionId ? <ContextBreakdownPanel
          state={breakdownState}
          sessionId={focusedSessionId}
          onClose={() => setShowBreakdown(false)}
          onOpenPromptStudio={onOpenPromptStudio}
        /> : <>
        <div className="mb-2.5 flex items-center gap-2 px-0.5">
          <Gauge size={13} className="shrink-0 text-muted-foreground" aria-hidden="true" />
          <h2 className="font-display text-sm font-semibold text-foreground">Usage health</h2>
          <span className="ml-auto truncate text-[9px] text-muted-foreground/70">No invented limits</span>
          <button type="button" onClick={() => setOpen(false)} className="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground" aria-label="Close usage health details"><X size={13} aria-hidden="true" /></button>
        </div>

        <div className="grid gap-2">
          {PROVIDERS.map(provider => <ProviderDetail key={provider.id} provider={provider} snapshot={usage[provider.id]} samples={samples[provider.id] ?? []} adapter={adapters?.find(item => item.id === provider.id)} activeLogin={activeLogin} onStartLogin={setActiveLogin} onCloseLogin={() => setActiveLogin(null)} />)}
        </div>

        <section className={cn("mt-2 border border-border p-3", PANEL_NESTED)} aria-label="Context pressure">
          <div className="flex flex-wrap items-center gap-2">
            <b className="text-[11px] text-foreground">{pressure.label}</b>
            {pressure.percent != null && <span className="font-mono text-[10px] text-muted-foreground">{Math.round(pressure.percent)}%</span>}
            {pressure.percent != null && <SourceBadge source={contextSource} />}
          </div>
          <p className="mt-1.5 text-[10px] leading-relaxed text-muted-foreground">{pressure.explanation}</p>
          {focusedSessionId && <button type="button" onClick={() => setShowBreakdown(true)} aria-haspopup="dialog" className="mt-2 inline-flex items-center gap-1.5 rounded-md px-1 py-0.5 text-[9.5px] font-medium text-ring transition-colors hover:bg-accent">
            <Layers size={10} aria-hidden="true" />Open context breakdown
          </button>}
        </section>

        <section className="mt-3" aria-label="Prompt cache diagnostics">
          <div className="mb-2 flex flex-wrap items-center justify-between gap-x-2 gap-y-1 px-0.5">
            <h3 className="text-[9px] font-semibold uppercase tracking-[0.13em] text-muted-foreground">Prompt cache</h3>
            <span className="text-[9px] text-muted-foreground/70">Provider-reported tokens</span>
          </div>
          {cacheDiagnostics.length ? <>
            <div className="grid gap-1.5">{cacheDiagnostics.slice(0, 6).map(diagnostic => <CacheRow key={diagnostic.key} diagnostic={diagnostic} />)}</div>
            {cacheDiagnostics.length > 6 && <p className="mt-2 px-0.5 text-[9px] text-muted-foreground/70">Showing 6 of {cacheDiagnostics.length} recent prompt groups.</p>}
          </> : <p className={cn("border border-dashed border-border px-3 py-4 text-center text-[10px] text-muted-foreground/70", PANEL_NESTED)}>No prompt-cache telemetry reported yet.</p>}
        </section>

        <section className="mt-3" aria-label="Usage history">
          <div className="mb-2 flex flex-wrap items-center justify-between gap-x-2 gap-y-1 px-0.5">
            <h3 className="text-[9px] font-semibold uppercase tracking-[0.13em] text-muted-foreground">Recent work units</h3>
            <span className="text-[9px] text-muted-foreground/70">Newest first</span>
          </div>
          {history.length ? <div className="grid gap-1.5">{history.slice(0, 6).map(entry => <HistoryRow key={entry.id} entry={entry} />)}</div> : <p className={cn("border border-dashed border-border px-3 py-4 text-center text-[10px] text-muted-foreground/70", PANEL_NESTED)}>No measured work-unit history yet.</p>}
        </section>
        </>}
      </div>
    </div>
  </div>;
});

function CacheRow({ diagnostic }: { diagnostic: CacheDiagnostic }) {
  const hit = diagnostic.cacheHitRatio == null ? "unknown" : `${Math.round(diagnostic.cacheHitRatio * 100)}%`;
  const amortization = diagnostic.writeAmortization == null ? "n/a" : `${diagnostic.writeAmortization.toFixed(1)}×`;
  const cost = diagnostic.reportedCostMicrousd == null
    ? "Cost unknown — provider did not report it"
    : `$${(diagnostic.reportedCostMicrousd / 1_000_000).toFixed(4)} ${diagnostic.costCoverage}`;
  return <div className={cn("border border-border px-3 py-2", PANEL_NESTED)}>
    <div className="flex min-w-0 items-center gap-2">
      <b className="shrink-0 text-[10px] text-foreground">{diagnostic.harness}</b>
      <span className="min-w-0 flex-1 truncate font-mono text-[9px] text-muted-foreground">{diagnostic.model}</span>
      <span className="shrink-0 whitespace-nowrap font-mono text-[9px] text-foreground">Hit {hit}</span>
    </div>
    <div className="mt-1 flex flex-wrap gap-x-2 gap-y-1 text-[9px] text-muted-foreground/70">
      <span>{diagnostic.cacheReadTokens.toLocaleString()} read</span>
      <span>{diagnostic.cacheWriteTokens.toLocaleString()} write</span>
      <span>{diagnostic.uncachedInputTokens.toLocaleString()} uncached</span>
      <span>write amortization {amortization}</span>
    </div>
    <div className="mt-1 flex min-w-0 flex-wrap gap-x-2 gap-y-1 text-[8.5px] text-muted-foreground/70">
      <span>Role: {humanizeMetric(diagnostic.role)}</span>
      <span>Task: {humanizeMetric(diagnostic.taskFamily)}</span>
      <span>Restore: {humanizeMetric(diagnostic.restorationMode)}</span>
      {diagnostic.crossHarnessReuse.map(marker => <span key={marker}>Reuse: {humanizeMetric(marker)}</span>)}
      {diagnostic.stablePrefixId && <span className="max-w-full truncate font-mono" title={`${diagnostic.stablePrefixId} · ${diagnostic.stablePrefixHash ?? "hash unknown"}`}>{diagnostic.stablePrefixId}</span>}
      {diagnostic.promptSchemaVersion != null && <span>schema v{diagnostic.promptSchemaVersion}</span>}
    </div>
    <div className="mt-1 text-[8.5px] text-muted-foreground/70">{cost}</div>
  </div>;
}

function humanizeMetric(value: string): string {
  const words = value.replaceAll("_", " ").replaceAll(":", " · ");
  return words.charAt(0).toUpperCase() + words.slice(1);
}

function ProviderDetail({ provider, snapshot, samples, adapter, activeLogin, onStartLogin, onCloseLogin }: { provider: { id: UsageProvider; label: string }; snapshot?: UsageSnapshot; samples: UsageRateSample[]; adapter?: AdapterDescriptor; activeLogin?: UsageProvider | null; onStartLogin: (provider: UsageProvider) => void; onCloseLogin: () => void }) {
  const status = providerStatus(adapter);
  const used = status === "normal" ? highestUse(snapshot) : undefined;
  const projection = status === "normal" ? projectUsageExhaustion(samples) : null;
  const loginActive = activeLogin === provider.id;
  return <section className={cn("border border-border p-3", PANEL_NESTED)} aria-label={`${provider.label} usage`}>
    <div className="flex min-w-0 flex-wrap items-center gap-2">
      <UsageRing used={used} inert={status !== "normal"} />
      <b className="text-[11px] text-foreground">{provider.label}</b>
      {status === "normal" && snapshot?.planType && <span className="text-[9px] text-muted-foreground">{snapshot.planType}</span>}
      {status === "normal" && snapshot?.model && <span className="min-w-0 truncate font-mono text-[9px] text-muted-foreground">{snapshot.model}</span>}
      <span className="ml-auto flex shrink-0 items-center gap-1.5">
        {status === "not_installed" && <span className="text-[9px] text-muted-foreground/70">Not installed</span>}
        {status === "signed_out" && !loginActive && <>
          <span className="text-[9px] text-muted-foreground/70">Not signed in</span>
          <button
            type="button"
            onClick={() => onStartLogin(provider.id)}
            className="rounded-md border border-border px-1.5 py-0.5 text-[9px] font-medium text-foreground transition-colors hover:bg-accent"
          >
            Sign in
          </button>
        </>}
        {status === "normal" && (snapshot ? <SourceBadge source={snapshot.source} /> : <span className="text-[9px] text-muted-foreground/70">Limit unknown</span>)}
      </span>
    </div>
    {loginActive ? <ProviderLoginPane provider={provider.id} label={provider.label} onClose={onCloseLogin} />
    : status === "not_installed" ? <p className="mt-2 text-[10px] leading-relaxed text-muted-foreground">{adapter?.unavailableReason ?? `${provider.label} isn't installed.`} Add it in Settings → Harnesses.</p>
    : status === "signed_out" ? <p className="mt-2 text-[10px] leading-relaxed text-muted-foreground">Sign in to see usage, quota, and context for this provider.</p>
    : snapshot?.windows.length ? <div className="mt-2.5 grid gap-2.5">{snapshot.windows.map(window => {
      const clamped = clampPercent(window.usedPercent);
      const reset = window.resetsLabel ?? formatReset(window.resetsInSeconds);
      return <div key={window.id}>
        <div className="mb-1 flex flex-wrap items-center gap-2 text-[10px]"><span className="min-w-0 truncate text-muted-foreground">{window.label}</span><span className="ml-auto whitespace-nowrap font-mono text-foreground">{Math.round(clamped)}% used</span><SourceBadge source={window.source} /></div>
        <UsageBar used={clamped} />
        <div className="mt-1 text-[9px] text-muted-foreground/70">{reset ?? "Reset unknown"}</div>
      </div>;
    })}</div> : <p className="mt-2 text-[10px] leading-relaxed text-muted-foreground">Limit unknown — this provider has not exposed a stable quota value. Token and context history remain available below.</p>}
    {projection && <div className="mt-2.5 flex gap-2 rounded-lg border border-warning/30 bg-warning/10 p-2 text-[9.5px] leading-relaxed text-warning"><AlertTriangle size={12} className="mt-0.5 shrink-0" aria-hidden="true" /><span>{projection.explanation} <b className="font-semibold uppercase tracking-wide">Estimated</b></span></div>}
  </section>;
}

function ProviderLoginPane({ provider, label, onClose }: { provider: UsageProvider; label: string; onClose: () => void }) {
  const [output, setOutput] = useState("");
  const [entry, setEntry] = useState("");
  const [error, setError] = useState<string | null>(null);
  const outputRef = useRef<HTMLPreElement>(null);
  const closeRef = useRef(onClose);
  closeRef.current = onClose;
  // Listen before launch: both subscriptions must be registered before the
  // vendor process starts, or its first output (OAuth URL, initial prompt)
  // can be lost permanently.
  useEffect(() => {
    let alive = true;
    let unlistenOutput: (() => void) | undefined;
    let unlistenExit: (() => void) | undefined;
    void (async () => {
      unlistenOutput = await bridgeApi.onTerminal(chunk => {
        if (!alive || chunk.sessionId !== "provider-login" || chunk.terminalId !== provider) return;
        setOutput(previous => (previous + chunk.data).slice(-8000));
      });
      unlistenExit = await bridgeApi.onTerminalExited(exit => {
        if (!alive || exit.sessionId !== "provider-login" || exit.terminalId !== provider) return;
        closeRef.current();
      });
      if (!alive) return;
      try {
        await bridgeApi.startProviderLogin(provider);
      } catch {
        if (alive) setError(`${label}'s sign-in flow could not be started. Check that the CLI is installed.`);
      }
    })();
    return () => {
      alive = false;
      unlistenOutput?.();
      unlistenExit?.();
    };
  }, [provider, label]);
  useEffect(() => {
    outputRef.current?.scrollTo?.({ top: outputRef.current.scrollHeight });
  }, [output]);
  const submit = (event: React.FormEvent) => {
    event.preventDefault();
    if (!entry.trim()) return;
    void bridgeApi.writeTerminal("provider-login", provider, `${entry}\r`);
    setEntry("");
  };
  return <div className="mt-2 grid gap-1.5">
    <div className="flex items-center gap-2">
      <span className="text-[9px] font-semibold uppercase tracking-[0.13em] text-muted-foreground">{label} sign-in</span>
      <span className="text-[8.5px] text-muted-foreground/70">Runs {label}'s own flow — Bridge never sees the credential.</span>
      <button type="button" onClick={onClose} className="ml-auto rounded-md px-1 py-0.5 text-[9px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">Hide</button>
    </div>
    <pre ref={outputRef} aria-live="polite" aria-label={`${label} sign-in output`} className="max-h-40 overflow-y-auto whitespace-pre-wrap rounded-md border border-border bg-card p-2 font-mono text-[9.5px] leading-relaxed text-foreground">{output || "Starting…"}</pre>
    {error && <p role="alert" className="text-[9.5px] text-destructive">{error}</p>}
    <form onSubmit={submit} className="flex gap-1.5">
      <input
        value={entry}
        onChange={event => setEntry(event.target.value)}
        placeholder="Type a response or paste a code / URL, press Enter"
        aria-label={`Reply to the ${label} sign-in prompt`}
        className="min-w-0 flex-1 rounded-md border border-border bg-card px-2 py-1 font-mono text-[10px] text-foreground placeholder:text-muted-foreground/60 focus:outline-none focus:ring-1 focus:ring-ring"
      />
      <button type="submit" className="rounded-md border border-border px-2 py-1 text-[9px] font-medium text-foreground transition-colors hover:bg-accent">Send</button>
    </form>
  </div>;
}

function HistoryRow({ entry }: { entry: UsageHistoryEntry }) {
  return <div className={cn("border border-border px-3 py-2", PANEL_NESTED)}>
    <div className="flex min-w-0 items-center gap-2">
      <span className="min-w-0 flex-1 truncate font-mono text-[9.5px] text-foreground" title={entry.workUnit}>{entry.workUnit}</span>
      <SourceBadge source={entry.source} />
    </div>
    <div className="mt-1 flex flex-wrap gap-x-2 gap-y-1 text-[9px] text-muted-foreground/70">
      <span>{entry.harness}</span>
      <span>{entry.model ?? "model unknown"}</span>
      <span>{entry.outcome}</span>
      {entry.totalTokens != null && <span>{entry.totalTokens.toLocaleString()} tokens</span>}
      {entry.contextPercent != null && <span>{entry.contextPercent}% context</span>}
    </div>
  </div>;
}
