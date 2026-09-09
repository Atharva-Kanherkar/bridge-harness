import { memo, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence, motion } from "framer-motion";
import { AlertTriangle, ChevronDown, Gauge, Layers, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { bridgeApi } from "../api";
import { MOTION_DURATION, useMotionTransition } from "../motion";
import { MeterReadings, useMeterClock } from "./meter/MeterReadings";
import { HarnessMark } from "./harnessMarks";
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
  compact?: boolean;
}

function sourceLabel(source: MetricSource): string {
  return source.charAt(0).toUpperCase() + source.slice(1);
}

/** Where a figure came from, in a quiet word: context, never louder than the number. */
function SourceBadge({ source }: { source: MetricSource }) {
  return <span className="shrink-0 whitespace-nowrap text-[11px] text-muted-foreground">{sourceLabel(source)}</span>;
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
  unknown: "text-muted-foreground",
  ok: "text-success",
  warning: "text-warning",
  critical: "text-destructive",
};

/** What the ring's color means, in words — the accessible name relies on this
 *  rather than the color alone, since a ring's tier is otherwise conveyed only
 *  by hue. */
const TIER_LABEL: Record<UsageTier, string> = {
  unknown: "unknown",
  ok: "healthy",
  warning: "elevated",
  critical: "critical",
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

export const UsageWidget = memo(function UsageWidget({ usage, adapters, samples = {}, history = [], cacheDiagnostics = [], contextPercent, contextSource = "measured", focusedSessionId = null, onOpenPromptStudio, compact = false }: UsageWidgetProps) {
  const [open, setOpen] = useState(false);
  const nowMs = useMeterClock();
  const [showBreakdown, setShowBreakdown] = useState(false);
  const [showMore, setShowMore] = useState(false);
  const [activeLogin, setActiveLogin] = useState<UsageProvider | null>(null);
  const [frame, setFrame] = useState<HTMLElement | null>(null);
  const [availableHeight, setAvailableHeight] = useState<number>();
  const rootRef = useRef<HTMLDivElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const detailsTransition = useMotionTransition(MOTION_DURATION.reveal);
  const pressure = contextPressure(contextPercent);
  const breakdownState = useContextBreakdown(focusedSessionId, open && showBreakdown);
  const projections = PROVIDERS.map(provider => {
    const projection = projectUsageExhaustion(samples[provider.id] ?? []);
    return projection ? { provider, projection } : null;
  }).filter((value): value is NonNullable<typeof value> => value != null);

  // Compact rides the composer, so its panel belongs to the pill's own box
  // rather than this button: portalling onto `[data-composer-frame]` is what
  // makes the panel exactly as wide as the composer instead of guessing.
  useEffect(() => {
    if (!compact) {
      setFrame(null);
      return;
    }
    setFrame(rootRef.current?.closest<HTMLElement>("[data-composer-frame]") ?? null);
  }, [compact]);

  // A tall composer leaves less room than a viewport percentage allows.
  // Keep the popover below the toolbar and above its actual anchor.
  useEffect(() => {
    if (!compact || !open) return;
    const anchor = frame ?? rootRef.current;
    if (!anchor) return;
    const main = anchor.closest("main");
    const measure = () => {
      let top = Math.max(0, main?.getBoundingClientRect().top ?? 0);
      for (let parent = anchor.parentElement; parent; parent = parent.parentElement) {
        const overflow = getComputedStyle(parent);
        if (/(auto|scroll|hidden|clip)/.test(overflow.overflowY || overflow.overflow)) {
          top = Math.max(top, parent.getBoundingClientRect().top);
        }
      }
      setAvailableHeight(Math.max(0, Math.min(window.innerHeight * 0.8, anchor.getBoundingClientRect().top - top - 12)));
    };
    measure();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    observer?.observe(anchor);
    if (main) observer?.observe(main);
    window.addEventListener("resize", measure);
    return () => { observer?.disconnect(); window.removeEventListener("resize", measure); };
  }, [compact, frame, open]);

  useEffect(() => {
    if (!open) return;
    const dismiss = (event: PointerEvent) => {
      const target = event.target as Node;
      if (rootRef.current?.contains(target) || panelRef.current?.contains(target)) return;
      setOpen(false);
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
  // Not just the percent: the tier word carries the same meaning the ring's
  // color does, so the accessible name doesn't depend on color alone.
  const usageStateLabel = overall == null ? "no reported usage yet" : `${TIER_LABEL[tier]}, ${Math.round(overall)}% used`;
  const indicatorTitle = `Usage health — ${usageStateLabel}`;

  const panel = (
    <div
      ref={panelRef}
      id="usage-health-panel"
      role="dialog"
      aria-label="Usage health details"
      className={cn(
        "absolute z-50 transition-opacity duration-150",
        // Compact takes the composer's width, and sits just clear of it. Butted
        // straight onto the frame it read as one shape with a seam through it:
        // the composer is rounded on all four corners, so a panel resting on it
        // can never continue that outline. It is its own popover instead.
        // The same card as the menu-bar meter: 360pt wide at the composer's
        // left edge, never the composer's full span over the conversation.
        compact
          ? "bottom-full left-0 mb-1.5 w-[min(100vw-1.5rem,22.5rem)]"
          : "right-0 top-full pt-2",
        "transition-[opacity,transform] duration-200 ease-out motion-reduce:transition-none",
        open ? "visible translate-y-0 pointer-events-auto opacity-100" : "invisible translate-y-1 pointer-events-none opacity-0",
      )}
    >
      {/* Content scrolls inside the material, bounded by the space above the composer. */}
      <div
        style={compact && availableHeight !== undefined ? { maxHeight: availableHeight } : undefined}
        className={cn(
          "@container/usage u-glass-popover flex max-h-[70dvh] flex-col overflow-hidden",
          // Every corner, both modes: a popover is a whole shape, and half-round
          // corners read as a rendering fault rather than as a join.
          compact ? "w-full rounded-2xl" : "w-[440px] max-w-[calc(100vw-1.5rem)] rounded-2xl",
        )}
      >
        <div className={cn("min-h-0 flex-1 overflow-y-auto", PANEL_PAD)}>
          {showBreakdown && focusedSessionId ? <ContextBreakdownPanel
            state={breakdownState}
            sessionId={focusedSessionId}
            onClose={() => setShowBreakdown(false)}
            onOpenPromptStudio={onOpenPromptStudio}
          /> : <>
          <div className="mb-2.5 flex items-center gap-2 px-0.5">
            <Gauge size={13} className="shrink-0 text-muted-foreground" aria-hidden="true" />
            <h2 className="font-display text-sm font-semibold text-foreground">Usage health</h2>
            <button type="button" onClick={() => setOpen(false)} className="ml-auto grid size-7 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground" aria-label="Close usage health details"><X size={13} aria-hidden="true" /></button>
          </div>

          {/* Live limits read exactly as the menu-bar meter does; providers
              without a reading follow as one quiet row each. */}
          <div className="px-0.5">
            <MeterReadings usage={usage} nowMs={nowMs} />
            {PROVIDERS.filter(provider => !usage[provider.id]?.windows.length).map(provider => <ProviderDetail key={provider.id} provider={provider} snapshot={usage[provider.id]} adapter={adapters?.find(item => item.id === provider.id)} activeLogin={activeLogin} onStartLogin={setActiveLogin} onCloseLogin={() => setActiveLogin(null)} />)}
            {projections.map(({ provider, projection }) => <p key={provider.id} className="mt-2 flex gap-2 rounded-lg border border-border p-2 text-[11px] leading-relaxed text-muted-foreground"><AlertTriangle size={12} className="mt-0.5 shrink-0" aria-hidden="true" /><span>{provider.label}: {projection.explanation} <span className="text-foreground">Estimated</span></span></p>)}
          </div>

          <section className={cn("mt-2 border border-border p-3", PANEL_NESTED)} aria-label="Context pressure">
            <div className="flex flex-wrap items-center gap-2">
              <b className="text-caption text-foreground">{pressure.label}</b>
              {pressure.percent != null && <span className="font-mono text-caption text-muted-foreground">{Math.round(pressure.percent)}%</span>}
              {pressure.percent != null && <SourceBadge source={contextSource} />}
            </div>
            <p className="mt-1.5 text-caption leading-relaxed text-muted-foreground">{pressure.explanation}</p>
            {focusedSessionId && <button type="button" onClick={() => setShowBreakdown(true)} aria-haspopup="dialog" className="mt-2 inline-flex items-center gap-1.5 min-h-7 rounded-md px-2 text-caption font-medium text-ring transition-colors hover:bg-accent">
              <Layers size={10} aria-hidden="true" />Open context breakdown
            </button>}
          </section>

          <button
            type="button"
            aria-expanded={showMore}
            aria-controls="usage-health-details"
            onClick={() => setShowMore(value => !value)}
            className="mt-2 flex w-full items-center justify-center gap-1 rounded-lg px-2 py-1.5 text-caption font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          >
            {showMore ? "Show less" : "Show more"}
            <ChevronDown size={12} aria-hidden="true" className={cn("transition-transform duration-300 ease-in-out motion-reduce:transition-none", showMore && "rotate-180")} />
          </button>

          {/* `height: auto` is the one transition CSS cannot express, so the
              disclosure body opens and closes through Framer instead of a
              measured max-height that was stale on the first expand. */}
          <AnimatePresence initial={false}>
            {showMore && <motion.div
              id="usage-health-details"
              className="overflow-hidden"
              initial={{ height: 0, opacity: 0 }}
              animate={{ height: "auto", opacity: 1 }}
              exit={{ height: 0, opacity: 0 }}
              transition={detailsTransition}
            >
              <section className="mt-3" aria-label="Prompt cache diagnostics">
                <div className="mb-2 flex flex-wrap items-center justify-between gap-x-2 gap-y-1 px-0.5">
                  <h3 className="text-caption font-semibold uppercase tracking-[0.13em] text-muted-foreground">Prompt cache</h3>
                  <span className="text-caption text-muted-foreground">Provider-reported tokens</span>
                </div>
                {cacheDiagnostics.length ? <>
                  <div className="grid gap-1.5">{cacheDiagnostics.slice(0, 6).map(diagnostic => <CacheRow key={diagnostic.key} diagnostic={diagnostic} />)}</div>
                  {cacheDiagnostics.length > 6 && <p className="mt-2 px-0.5 text-caption text-muted-foreground">Showing 6 of {cacheDiagnostics.length} recent prompt groups.</p>}
                </> : <p className={cn("border border-dashed border-border px-3 py-4 text-center text-caption text-muted-foreground", PANEL_NESTED)}>No prompt-cache telemetry reported yet.</p>}
              </section>

              <section className="mt-3" aria-label="Usage history">
                <div className="mb-2 flex flex-wrap items-center justify-between gap-x-2 gap-y-1 px-0.5">
                  <h3 className="text-caption font-semibold uppercase tracking-[0.13em] text-muted-foreground">Recent work units</h3>
                  <span className="text-caption text-muted-foreground">Newest first</span>
                </div>
                {history.length ? <div className="grid gap-1.5">{history.slice(0, 6).map(entry => <HistoryRow key={entry.id} entry={entry} />)}</div> : <p className={cn("border border-dashed border-border px-3 py-4 text-center text-caption text-muted-foreground", PANEL_NESTED)}>No measured work-unit history yet.</p>}
              </section>
            </motion.div>}
          </AnimatePresence>
          </>}
        </div>
      </div>
    </div>
  );

  return <div ref={rootRef} className="relative">
    <button
      type="button"
      className={cn(
        compact
          ? "relative inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-full transition-colors duration-150 hover:bg-accent hover:text-foreground active:scale-95"
          : "relative grid size-8 shrink-0 cursor-pointer place-items-center rounded-full border border-border bg-card transition-colors hover:bg-accent",
        open && "bg-accent",
      )}
      aria-label={`${open ? "Close" : "Open"} usage health details — ${usageStateLabel}`}
      aria-expanded={open}
      aria-controls="usage-health-panel"
      title={indicatorTitle}
      onClick={() => setOpen(value => !value)}
    >
      <UsageIndicatorRing percent={overall} tier={tier} />
      {projections.length > 0 && <span className="absolute -right-0.5 -top-0.5 grid size-3.5 place-items-center rounded-full bg-warning text-warning-foreground" aria-label="Projected usage exhaustion"><AlertTriangle size={9} strokeWidth={2.5} aria-hidden="true" /></span>}
    </button>
    {compact && frame ? createPortal(panel, frame) : panel}
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
      <b className="shrink-0 text-caption text-foreground">{diagnostic.harness}</b>
      <span className="min-w-0 flex-1 truncate font-mono text-caption text-muted-foreground">{diagnostic.model}</span>
      <span className="shrink-0 whitespace-nowrap font-mono text-caption text-foreground">Hit {hit}</span>
    </div>
    <div className="mt-1 flex flex-wrap gap-x-2 gap-y-1 text-caption text-muted-foreground">
      <span>{diagnostic.cacheReadTokens.toLocaleString()} read</span>
      <span>{diagnostic.cacheWriteTokens.toLocaleString()} write</span>
      <span>{diagnostic.uncachedInputTokens.toLocaleString()} uncached</span>
      <span>write amortization {amortization}</span>
    </div>
    <div className="mt-1 flex min-w-0 flex-wrap gap-x-2 gap-y-1 text-caption text-muted-foreground">
      <span>Role: {humanizeMetric(diagnostic.role)}</span>
      <span>Task: {humanizeMetric(diagnostic.taskFamily)}</span>
      <span>Restore: {humanizeMetric(diagnostic.restorationMode)}</span>
      {diagnostic.crossHarnessReuse.map(marker => <span key={marker}>Reuse: {humanizeMetric(marker)}</span>)}
      {diagnostic.stablePrefixId && <span className="max-w-full truncate font-mono" title={`${diagnostic.stablePrefixId} · ${diagnostic.stablePrefixHash ?? "hash unknown"}`}>{diagnostic.stablePrefixId}</span>}
      {diagnostic.promptSchemaVersion != null && <span>schema v{diagnostic.promptSchemaVersion}</span>}
    </div>
    <div className="mt-1 text-caption text-muted-foreground">{cost}</div>
  </div>;
}

function humanizeMetric(value: string): string {
  const words = value.replaceAll("_", " ").replaceAll(":", " · ");
  return words.charAt(0).toUpperCase() + words.slice(1);
}

function ProviderDetail({ provider, snapshot, adapter, activeLogin, onStartLogin, onCloseLogin }: { provider: { id: UsageProvider; label: string }; snapshot?: UsageSnapshot; adapter?: AdapterDescriptor; activeLogin?: UsageProvider | null; onStartLogin: (provider: UsageProvider) => void; onCloseLogin: () => void }) {
  const status = providerStatus(adapter);
  const loginActive = activeLogin === provider.id;
  return <section className="border-b border-border py-2.5 last:border-0" aria-label={`${provider.label} usage`}>
    <div className="flex min-w-0 items-center gap-1.5 text-ui">
      <HarnessMark harness={provider.id} size={12} />
      <span className="font-medium text-foreground">{provider.label}</span>
      <span className="ml-auto flex shrink-0 items-center gap-1.5 text-[11px] text-muted-foreground">
        {status === "not_installed" && "Not installed"}
        {status === "signed_out" && !loginActive && <>
          Not signed in
          <button type="button" onClick={() => onStartLogin(provider.id)} className="min-h-6 rounded-md border border-border px-2 text-[11px] font-medium text-foreground transition-colors hover:bg-accent">Sign in</button>
        </>}
        {status === "normal" && (snapshot ? <SourceBadge source={snapshot.source} /> : "Limit unknown")}
      </span>
    </div>
    {loginActive ? <ProviderLoginPane provider={provider.id} label={provider.label} onClose={onCloseLogin} />
    : status === "not_installed" ? <p className="mt-1 text-[11px] leading-relaxed text-muted-foreground">{adapter?.unavailableReason ?? `${provider.label} isn't installed.`} Add it in Settings → Harnesses.</p>
    : status === "signed_out" ? <p className="mt-1 text-[11px] leading-relaxed text-muted-foreground">Sign in to see usage, quota, and context for this provider.</p>
    : <p className="mt-1 text-[11px] leading-relaxed text-muted-foreground">This provider has not reported its quota yet.</p>}
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
  const send = () => {
    if (!entry.trim()) return;
    void bridgeApi.writeTerminal("provider-login", provider, `${entry}\r`);
    setEntry("");
  };
  return <div className="mt-2 grid gap-1.5">
    <div className="flex flex-wrap items-center gap-2">
      <span className="text-caption font-semibold uppercase tracking-[0.13em] text-muted-foreground">{label} sign-in</span>
      <span className="text-caption text-muted-foreground">Runs {label}'s own flow — Bridge never sees the credential.</span>
      <button type="button" onClick={onClose} className="ml-auto min-h-7 rounded-md px-2 text-caption text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">Hide</button>
    </div>
    <pre ref={outputRef} aria-live="polite" aria-label={`${label} sign-in output`} className="max-h-40 overflow-y-auto whitespace-pre-wrap rounded-md border border-border bg-card p-2 font-mono text-caption leading-relaxed text-foreground">{output || "Starting…"}</pre>
    {error && <p role="alert" className="text-caption text-destructive">{error}</p>}
    {/* Deliberately not a <form>. This pane renders through the composer's
        `leading` slot — inside the composer's own <form> — and a nested
        form's submit event still bubbles, so an Enter here would also invoke
        the composer's onSubmit and send the draft. A plain row with an
        explicit Enter handler keeps the reply local to the provider terminal. */}
    <div className="flex gap-1.5">
      <input
        value={entry}
        onChange={event => setEntry(event.target.value)}
        onKeyDown={event => { if (event.key === "Enter") { event.preventDefault(); send(); } }}
        placeholder="Type a response or paste a code / URL, press Enter"
        aria-label={`Reply to the ${label} sign-in prompt`}
        className="min-w-0 flex-1 rounded-md border border-border bg-card px-2 py-1 font-mono text-caption text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-ring"
      />
      <button type="button" onClick={send} className="rounded-md border border-border px-2 py-1 text-caption font-medium text-foreground transition-colors hover:bg-accent">Send</button>
    </div>
  </div>;
}

function HistoryRow({ entry }: { entry: UsageHistoryEntry }) {
  return <div className={cn("border border-border px-3 py-2", PANEL_NESTED)}>
    <div className="flex min-w-0 items-center gap-2">
      <span className="min-w-0 flex-1 truncate font-mono text-caption text-foreground" title={entry.workUnit}>{entry.workUnit}</span>
      <SourceBadge source={entry.source} />
    </div>
    <div className="mt-1 flex flex-wrap gap-x-2 gap-y-1 text-caption text-muted-foreground">
      <span>{entry.harness}</span>
      <span>{entry.model ?? "model unknown"}</span>
      <span>{entry.outcome}</span>
      {entry.totalTokens != null && <span>{entry.totalTokens.toLocaleString()} tokens</span>}
      {entry.contextPercent != null && <span>{entry.contextPercent}% context</span>}
    </div>
  </div>;
}
