import { useCallback, useEffect, useMemo, useState } from "react";
import { ChevronDown, Gauge, LoaderCircle, RefreshCw, RotateCcw } from "lucide-react";
import { cn } from "@/lib/utils";
import { bridgeApi } from "../api";
import { harnessLabel } from "../utils";
import type { UsageHistorySource, UsagePriceOverride, UsageSummaryResult } from "../types";
import { HarnessMark } from "./harnessMarks";
import { SCREEN_CONTENT, ScreenHeading } from "./ui/screen";
import { UsageChart, seriesDotClass } from "./UsageChart";
import { UsageInsights } from "./UsageInsights";
import { UsageHeatmap } from "./UsageHeatmap";
import { UsageLedger } from "./UsageLedger";
import { UsageStrips } from "./UsageStrips";
import { UsageFlow } from "./UsageFlow";
import { UsageMosaic } from "./UsageMosaic";
import { UsageCalendar } from "./UsageCalendar";
import type { UsageLayoutProps } from "./UsageLayoutParts";
import { useProviderUsageOverviews } from "./UsageDot";
import { UsageResetRow } from "./UsageResetRow";
import {
  buildChartSeries, buildUsageReport, costSourceLabel, enumeratePeriods, formatCount, formatDayShort, formatPercent, formatPeriodLabel, formatTokens, formatUsd, formatWindowLabel,
  makeUsageWindow, microToUsdPerMtok, readUsagePreferences, summaryParams, USAGE_LAYOUT_OPTIONS, USAGE_WINDOW_OPTIONS, usdPerMtokToMicro, writeUsagePreferences,
  type UsageLayout, type UsageMetric, type UsagePreferences, type UsageReport, type UsageWindow, type UsageWindowDays,
} from "../usageReport";

// The usage destination: what Bridge's harnesses processed and what it would
// have cost at API rates, over a window, by harness and model. Every figure
// traces to a ledger row or an imported transcript observation; the honesty
// rules are in testing/feat-usage-frontend.md and the notes below say when a
// number is partial, estimated, or de-duplicated.

const CARD = "u-surface rounded-2xl p-4";
const TABLE_HEAD = "text-left text-[11px] font-medium uppercase tracking-wide text-muted-foreground";
const NUM = "text-right tabular-nums";

/// Upper bound on bounded scan batches per auto-scan. Opening Usage must not
/// fire an open-ended series of IPC round trips on a huge history; hitting
/// the cap surfaces as a partial total and Scan history resumes the cursors.
const MAX_SCAN_PASSES = 25;

type Breakdown = "model" | "time";
type UsageTab = "usage" | "insights";

const LAYOUT_LABELS: Record<UsageLayout, string> = { ledger: "Ledger", strips: "Strips", flow: "Flow", mosaic: "Mosaic", calendar: "Calendar", classic: "Classic" };

/** The five switchable presentations. Classic is the original screen and renders inline below. */
const LAYOUTS: Record<Exclude<UsageLayout, "classic">, (props: UsageLayoutProps) => React.ReactNode> = {
  ledger: UsageLedger,
  strips: UsageStrips,
  flow: UsageFlow,
  mosaic: UsageMosaic,
  calendar: UsageCalendar,
};

function windowLabel(days: UsageWindowDays): string {
  return days === 1 ? "24h" : `${days}d`;
}

function Segmented<T extends string | number>({ label, value, options, onChange }: { label: string; value: T; options: { value: T; label: string }[]; onChange: (value: T) => void }) {
  return <div role="radiogroup" aria-label={label} className="u-segmented">
    {options.map(option => <button key={String(option.value)} type="button" role="radio" aria-checked={option.value === value} data-active={option.value === value} className="u-segmented-item" onClick={() => onChange(option.value)}>{option.label}</button>)}
  </div>;
}

function coverageTone(state: string): string {
  switch (state) {
    case "complete": return "text-success";
    case "partial": case "stale": return "text-warning";
    case "unsupported": case "empty": return "text-muted-foreground";
    default: return "text-destructive";
  }
}

function coverageLabel(state: string): string {
  return state ? state[0].toUpperCase() + state.slice(1) : "Unknown";
}

function relativeStamp(iso: string | null | undefined): string {
  if (!iso) return "never";
  const at = new Date(iso);
  if (Number.isNaN(at.getTime())) return iso;
  const minutes = Math.round((Date.now() - at.getTime()) / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} h ago`;
  return formatDayShort(at.toISOString().slice(0, 10));
}

export function UsageScreen({ onError, onOpenMeter }: { onError: (message: string) => void; onOpenMeter?: () => void }) {
  const providerUsage = useProviderUsageOverviews();
  useEffect(() => { providerUsage.refresh(); }, [providerUsage.refresh]);
  const [preferences, setPreferences] = useState<UsagePreferences>(() => readUsagePreferences());
  const [refreshTick, setRefreshTick] = useState(0);
  const [summaryTick, setSummaryTick] = useState(0);
  const window_ = useMemo(() => makeUsageWindow(preferences.windowDays), [preferences.windowDays, refreshTick]);
  const periods = useMemo(() => enumeratePeriods(window_), [window_]);
  const [summary, setSummary] = useState<UsageSummaryResult | null>(null);
  const [loading, setLoading] = useState(true);
  const [sources, setSources] = useState<UsageHistorySource[]>([]);
  const [overrides, setOverrides] = useState<UsagePriceOverride[]>([]);
  const [tab, setTab] = useState<UsageTab>("usage");
  const [activityOpen, setActivityOpen] = useState(false);
  const [tableOpen, setTableOpen] = useState(false);
  const [scanning, setScanning] = useState(preferences.includeImported);
  const [scanFailed, setScanFailed] = useState(false);
  const [refreshingRates, setRefreshingRates] = useState(false);
  const [scanNote, setScanNote] = useState<string | null>(null);

  const update = useCallback((patch: Partial<UsagePreferences>) => {
    setPreferences(current => {
      const next = { ...current, ...patch };
      writeUsagePreferences(next);
      return next;
    });
  }, []);

  const loadSummary = () => setSummaryTick(tick => tick + 1);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    bridgeApi.usageSummary(summaryParams(window_, preferences.includeImported))
      .then(result => { if (!cancelled) setSummary(result); })
      .catch(error => {
        if (!cancelled) {
          setSummary(null);
          onError(error instanceof Error ? error.message : String(error));
        }
      })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [window_, preferences.includeImported, summaryTick, onError]);

  useEffect(() => {
    bridgeApi.listUsagePriceOverrides().then(setOverrides).catch(() => undefined);
  }, []);

  useEffect(() => {
    let cancelled = false;
    setScanning(preferences.includeImported);
    setScanFailed(false);
    setScanNote(null);
    const scan = async () => {
      try {
        const discovered = await bridgeApi.listUsageHistorySources();
        if (cancelled) return;
        setSources(discovered);
        if (!preferences.includeImported) return;

        let sourceIds: string[] | undefined;
        let imported = 0;
        let durationMs = 0;
        let passes = 0;
        const cursors = new Map<string, string | null>();
        const warnings = new Set<string>();
        // Each call is bounded. A partial source is excluded by the summary
        // until its last batch, so one successful call is not a finished scan.
        // Passes are capped: opening Usage must not fire an open-ended series
        // of IPC round trips on a huge history. Hitting the cap is a partial
        // total, and Scan history resumes from the cursors.
        do {
          passes += 1;
          if (passes > MAX_SCAN_PASSES) {
            warnings.add(`History scan stopped after ${MAX_SCAN_PASSES} batches with more to go. Press Scan history to continue.`);
            break;
          }
          const result = await bridgeApi.scanUsageHistory(sourceIds ? { sourceIds } : {});
          if (cancelled) return;
          imported += result.recordsImported;
          durationMs += result.durationMs;
          sourceIds = [];
          for (const source of result.sources) {
            if (source.capability !== "supported") continue;
            if (source.coverage === "partial") {
              const cursor = source.nextCursor ?? null;
              if (cursors.has(source.sourceId) && cursors.get(source.sourceId) === cursor) {
                warnings.add(`History scan did not advance for ${harnessLabel(source.agent)}. Try Scan history again.`);
              } else {
                cursors.set(source.sourceId, cursor);
                sourceIds.push(source.sourceId);
              }
            } else if (source.coverage === "unreadable" || source.coverage === "stale") {
              warnings.add(source.warning ?? `${harnessLabel(source.agent)} history is ${source.coverage}.`);
            }
          }
          setScanFailed(warnings.size > 0);
          setScanNote(`Loading history. Imported ${formatCount(imported)} records so far.`);
          const updated = await bridgeApi.listUsageHistorySources();
          if (cancelled) return;
          setSources(updated);
        } while (sourceIds.length > 0);
        setScanNote(warnings.size > 0
          ? `Imported ${formatCount(imported)} records. ${[...warnings].join(" ")} Totals are incomplete.`
          : `Imported ${formatCount(imported)} records in ${(durationMs / 1000).toFixed(1)} s.`);
      } catch (error) {
        if (!cancelled) {
          const message = error instanceof Error ? error.message : String(error);
          setScanFailed(true);
          setScanNote(`History could not finish: ${message}. Totals are incomplete.`);
          onError(message);
        }
      } finally {
        if (!cancelled) {
          setScanning(false);
          if (preferences.includeImported) {
            setLoading(true);
            setSummaryTick(tick => tick + 1);
          }
        }
      }
    };
    void scan();
    return () => { cancelled = true; };
  }, [preferences.includeImported, refreshTick, onError]);

  const report = useMemo(() => summary ? buildUsageReport(summary, periods) : null, [summary, periods]);
  const metric = preferences.metric;
  const layout = preferences.layout;
  const LayoutView = layout === "classic" ? null : LAYOUTS[layout];

  const scan = () => {
    update({ includeImported: true });
    setRefreshTick(tick => tick + 1);
  };

  const refreshRates = async () => {
    setRefreshingRates(true);
    try {
      await bridgeApi.refreshUsageRates();
      loadSummary();
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    } finally {
      setRefreshingRates(false);
    }
  };

  const changeOverrides = async (next: Promise<UsagePriceOverride[]>) => {
    try {
      setOverrides(await next);
      loadSummary();
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    }
  };

  const incompleteSources = preferences.includeImported
    ? sources.filter(source => source.origin !== "dashboard" && source.coverageState !== "complete" && source.coverageState !== "empty")
    : [];
  const dashboardSources = preferences.includeImported ? summary?.sources.filter(source => source.origin === "dashboard") ?? [] : [];
  const historyIncomplete = preferences.includeImported && (scanning || scanFailed || incompleteSources.some(source => source.capability === "supported")
    || dashboardSources.some(source => source.coverageState !== "complete" && source.coverageState !== "empty"));
  const partialTotal = loading || historyIncomplete;
  const partialLabel = scanning ? "Loading history" : loading ? "Updating usage" : "History incomplete";

  return <div className="h-full min-h-0 overflow-y-auto">
    <div className={SCREEN_CONTENT}>
      <ScreenHeading
        title="Usage"
        description={tab === "usage" ? "Tokens processed across harnesses and what they would cost at API rates. Not money spent: subscription plans bill separately." : "What your usage says about how you work, written by your harness from Bridge's own records."}
        action={<span className="inline-flex shrink-0 items-center gap-2">
          <Segmented<UsageTab> label="View" value={tab} options={[{ value: "usage", label: "Usage" }, { value: "insights", label: "Insights" }]} onChange={setTab} />
          {onOpenMeter && <button type="button" onClick={onOpenMeter} aria-label="Open usage meter" title="Usage meter" className="inline-flex h-8 w-8 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"><Gauge size={14} aria-hidden="true" /></button>}
          <button type="button" onClick={() => { setRefreshTick(tick => tick + 1); providerUsage.refresh(); }} disabled={loading || scanning} aria-label="Refresh usage" aria-busy={loading || scanning} className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-40"><RefreshCw size={14} className={loading || scanning ? "animate-spin" : ""} /></button>
        </span>}
      />

      {tab === "insights" ? <UsageInsights windowDays={preferences.windowDays} onError={onError} /> : <>
      {providerUsage.overviews?.providers.map(snapshot =>
        <div key={snapshot.provider} className="mb-3 empty:hidden"><UsageResetRow snapshot={snapshot} onUpdated={providerUsage.refresh} /></div>)}
      <div className="mb-5 flex flex-wrap items-start gap-3">
        <Segmented<UsageMetric> label="Metric" value={metric} options={[{ value: "cost", label: "Cost" }, { value: "tokens", label: "Tokens" }]} onChange={value => update({ metric: value })} />
        <Segmented<UsageWindowDays> label="Window" value={preferences.windowDays} options={USAGE_WINDOW_OPTIONS.map(days => ({ value: days, label: windowLabel(days) }))} onChange={value => update({ windowDays: value })} />
        <span className="pt-2 text-caption tabular-nums text-muted-foreground">{formatWindowLabel(window_)}</span>
        <span className="ml-auto"><Segmented<UsageLayout> label="Layout" value={layout} options={USAGE_LAYOUT_OPTIONS.map(option => ({ value: option, label: LAYOUT_LABELS[option] }))} onChange={value => update({ layout: value })} /></span>
      </div>

      {report && (incompleteSources.length > 0 || dashboardSources.length > 0 || summary!.duplicatesDropped > 0 || report.totals.unpricedRecords > 0) && <ul className="mb-5 space-y-1 text-caption text-muted-foreground" aria-label="Coverage notes">
        {incompleteSources.map(source => <li key={source.id}>{source.coverageState === "unsupported"
          ? `${harnessLabel(source.agent)}: ${source.coverageReason ?? "History is unavailable."}`
          : source.coverageState === "stale" || source.coverageState === "unreadable" ? `${harnessLabel(source.agent)} history needs a refresh.`
          : `${harnessLabel(source.agent)} history is still loading.`}</li>)}
        {dashboardSources.map(source => <li key={source.id}>{source.coverageReason}</li>)}
        {summary!.duplicatesDropped > 0 && <li>{formatCount(summary!.duplicatesDropped)} live records were counted once against their imported transcripts.</li>}
        {report.totals.unpricedRecords > 0 && <li>{formatCount(report.totals.unpricedRecords)} records have no known rate or complete pricing inputs; their cost is unknown, not free. Their tokens are included.</li>}
      </ul>}

      {loading && !summary ? <div role="status" className="grid h-56 place-items-center text-caption text-muted-foreground"><LoaderCircle size={16} className="animate-spin" aria-hidden="true" /><span className="sr-only">Loading usage</span></div> : report && <>
        {LayoutView ? <>
          <LayoutView report={report} summary={summary!} periods={periods} window={window_} windowDays={preferences.windowDays} metric={metric} partial={partialTotal ? partialLabel : null} />
          <section className={cn(CARD, "mt-6 py-3")} aria-label="Breakdown table">
            <button type="button" aria-expanded={tableOpen} aria-controls="usage-breakdown-table" onClick={() => setTableOpen(open => !open)} className="flex w-full items-center gap-2 text-left">
              <h2 className="text-ui font-medium text-foreground">Breakdown table</h2>
              <span className="text-caption text-muted-foreground">{formatCount(report.models.length)} {report.models.length === 1 ? "model" : "models"} · {formatCount(report.harnesses.length)} {report.harnesses.length === 1 ? "harness" : "harnesses"}</span>
              <ChevronDown size={14} className={cn("ml-auto text-muted-foreground transition-transform", tableOpen && "rotate-180")} aria-hidden="true" />
            </button>
            {tableOpen && <div id="usage-breakdown-table" className="mt-3"><BreakdownTables report={report} window={window_} /></div>}
          </section>
        </> : <ClassicUsage report={report} periods={periods} window={window_} metric={metric} partialTotal={partialTotal} partialLabel={partialLabel} activityOpen={activityOpen} onToggleActivity={() => setActivityOpen(open => !open)} />}

        <section className={cn(CARD, "mt-4")} aria-label="History sources">
          <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
            <div>
              <h2 className="text-ui font-medium text-foreground">History sources</h2>
              <p className="text-caption text-muted-foreground">Local Claude Code, Codex, and OpenCode history, plus Cursor dashboard usage. Only usage metadata is saved, never prompt or completion text.</p>
            </div>
            <button type="button" onClick={() => void scan()} disabled={scanning} className="inline-flex h-8 items-center gap-2 rounded-lg border border-border px-3 text-caption text-foreground transition-colors hover:bg-accent disabled:opacity-40">
              {scanning ? <LoaderCircle size={13} className="animate-spin" aria-hidden="true" /> : <RefreshCw size={13} aria-hidden="true" />}Scan history
            </button>
          </div>
          {scanNote && <p role="status" className="mb-3 text-caption text-muted-foreground">{scanNote}</p>}
          <ul className="divide-y divide-border">
            {sources.length === 0 && <li className="py-3 text-caption text-muted-foreground">No history sources were found.</li>}
            {sources.map(source => <li key={source.id} className="flex flex-wrap items-start justify-between gap-x-4 gap-y-1 py-2.5">
              <div className="min-w-0">
                <div className="inline-flex items-center gap-2 text-ui text-foreground"><HarnessMark harness={source.agent} size={13} />{harnessLabel(source.agent)}<span className={cn("text-caption", coverageTone(source.coverageState))}>{source.origin === "dashboard" && source.coverageState === "unreadable" ? "Unavailable" : source.origin === "dashboard" && source.coverageState === "unsupported" ? "Disabled" : coverageLabel(source.coverageState)}</span></div>
                <p className="text-caption text-muted-foreground">{source.origin === "dashboard" ? "Dashboard usage · last 30 days" : source.capability === "supported" ? "Local usage history" : "History is not available for this harness"}</p>
                {(source.location || source.coverageReason || source.lastError) && <details className="mt-1 text-[11px] text-muted-foreground">
                  <summary className="w-fit cursor-pointer select-none hover:text-foreground">Technical details</summary>
                  <div className="mt-1 space-y-1 border-l border-border pl-2">
                    {source.location && <div className="break-all font-mono">{source.location}</div>}
                    {(source.coverageReason || source.lastError) && <div>{source.lastError ?? source.coverageReason}</div>}
                  </div>
                </details>}
              </div>
              {source.capability === "supported" && <div className="text-right text-[11px] tabular-nums text-muted-foreground">
                {source.origin === "dashboard" ? <>
                  {source.coverageStartAt && <div>{formatCount(source.recordsImported)} cached events</div>}
                  <div>Last update {relativeStamp(source.lastSuccessfulScanAt)}</div>
                </> : <>
                  <div>{formatCount(source.recordsImported)} imported · {formatCount(source.recordsSkipped)} skipped</div>
                  <div>Last scan {relativeStamp(source.lastSuccessfulScanAt)}</div>
                </>}
              </div>}
            </li>)}
          </ul>
        </section>

        <PriceSection report={report} summary={summary!} overrides={overrides} refreshing={refreshingRates} onRefreshRates={() => void refreshRates()} onChange={changeOverrides} />
      </>}
      </>}
    </div>
  </div>;
}

/** The original screen: summary card, layered chart, activity calendar, totals tiles, breakdown. */
function ClassicUsage({ report, periods, window: window_, metric, partialTotal, partialLabel, activityOpen, onToggleActivity }: {
  report: UsageReport;
  periods: string[];
  window: UsageWindow;
  metric: UsageMetric;
  partialTotal: boolean;
  partialLabel: string;
  activityOpen: boolean;
  onToggleActivity: () => void;
}) {
  const series = useMemo(() => buildChartSeries(report, metric), [report, metric]);
  return <>
    <section className="grid gap-4 lg:grid-cols-[minmax(0,18rem)_minmax(0,1fr)]" aria-label="Summary">
      <div className={CARD}>
        {partialTotal && <span role="status" className="mb-2 inline-flex items-center gap-1.5 rounded-full border border-border bg-muted/40 px-2 py-1 text-[11px] text-muted-foreground"><span className="size-1.5 rounded-full bg-muted-foreground/50" aria-hidden="true" />Partial total · {partialLabel}</span>}
        <div className="font-display text-4xl font-semibold tabular-nums tracking-tight text-foreground">{metric === "cost" ? formatUsd(report.totals.costMicrousd) : formatTokens(report.totals.processedTokens)}</div>
        <p className="mt-1 text-caption text-muted-foreground">{formatCount(report.totals.records)} requests{metric === "cost" ? ` · API estimate · ${costSourceLabel(report.costSource)}` : " · processed tokens"}</p>
        <ul className="mt-4 space-y-2.5" aria-label="By harness">
          {report.harnesses.length === 0 && <li className="text-caption text-muted-foreground">No activity in this window.</li>}
          {report.harnesses.map(entry => <li key={entry.harness} className="flex items-start justify-between gap-3">
            <span className="inline-flex min-w-0 items-center gap-2 text-ui text-foreground"><span className={cn("size-2 shrink-0 rounded-[3px]", seriesDotClass(entry.harness))} aria-hidden="true" /><HarnessMark harness={entry.harness} size={13} /><span className="truncate">{harnessLabel(entry.harness)}</span></span>
            <span className="text-right">
              <span className="block text-ui tabular-nums text-foreground">{metric === "cost" ? formatUsd(entry.costMicrousd) : formatTokens(entry.processedTokens)}</span>
              <span className="block text-[11px] tabular-nums text-muted-foreground">{metric === "cost" ? `${formatPercent(entry.costShare)} of cost · ${formatTokens(entry.processedTokens)} tokens` : `${formatPercent(entry.tokenShare)} of tokens · ${formatUsd(entry.costMicrousd)}`}</span>
            </span>
          </li>)}
        </ul>
      </div>
      <div className={cn(CARD, "min-w-0")}>
        <h2 className="mb-3 text-ui font-medium text-foreground">{window_.resolution === "hour" ? "Hourly" : "Daily"} {metric === "cost" ? "cost" : "processed tokens"}</h2>
        <UsageChart series={series} periods={periods} resolution={window_.resolution} timeZone={window_.timeZone} metric={metric} />
      </div>
    </section>

    <section className={cn(CARD, "mt-4 py-3")} aria-label="Activity">
      <button type="button" aria-expanded={activityOpen} aria-controls="usage-activity" onClick={onToggleActivity} className="flex w-full items-center gap-2 text-left">
        <h2 className="text-ui font-medium text-foreground">Activity</h2>
        <span className="text-caption text-muted-foreground">{window_.resolution === "hour" ? "by hour" : "by day"}, coloured by harness</span>
        <ChevronDown size={14} className={cn("ml-auto text-muted-foreground transition-transform", activityOpen && "rotate-180")} aria-hidden="true" />
      </button>
      {activityOpen && <div id="usage-activity" className="mt-3">
        <UsageHeatmap periods={report.periods} resolution={window_.resolution} timeZone={window_.timeZone} metric={metric} />
      </div>}
    </section>

    <section className="mt-4 grid grid-cols-2 gap-3 md:grid-cols-3 xl:grid-cols-6" aria-label="Totals">
      {[
        ["Processed tokens", formatTokens(report.totals.processedTokens)],
        ["Cached input", formatTokens(report.totals.cacheReadTokens)],
        ["Uncached input", formatTokens(report.totals.uncachedInputTokens)],
        ["Output", formatTokens(report.totals.outputTokens)],
        ["Reasoning (in output)", formatTokens(report.totals.reasoningTokens)],
        ["Local cache savings", formatUsd(report.totals.cacheSavingsMicrousd)],
      ].map(([label, value]) => <div key={label} className={cn(CARD, "py-3")}>
        <div className="text-[11px] text-muted-foreground">{label}</div>
        <div className="mt-0.5 text-ui font-medium tabular-nums text-foreground">{value}</div>
      </div>)}
    </section>

    <section className={cn(CARD, "mt-4")} aria-label="Breakdown">
      <BreakdownTables report={report} window={window_} heading />
    </section>
  </>;
}

/** Model and period tables: the non-visual equivalent of every chart on the screen. */
function BreakdownTables({ report, window: window_, heading }: { report: UsageReport; window: UsageWindow; heading?: boolean }) {
  const [breakdown, setBreakdown] = useState<Breakdown>("model");
  return <>
    <div className="mb-3 flex items-center justify-between gap-3">
      {heading ? <h2 className="text-ui font-medium text-foreground">Breakdown</h2> : <span className="text-caption text-muted-foreground">By model or by {window_.resolution === "hour" ? "hour" : "day"}</span>}
      <Segmented<Breakdown> label="Breakdown by" value={breakdown} options={[{ value: "model", label: "Model" }, { value: "time", label: window_.resolution === "hour" ? "Hour" : "Day" }]} onChange={setBreakdown} />
    </div>
    {breakdown === "model" ? <table className="w-full text-ui">
      <thead><tr className={TABLE_HEAD}><th className="w-2/5 pb-2 font-medium">Model</th><th className={cn("w-1/5 pb-2 font-medium", NUM)}>Cost</th><th className={cn("w-1/5 pb-2 font-medium", NUM)}>Share</th><th className={cn("w-1/5 pb-2 font-medium", NUM)}>Tokens</th></tr></thead>
      <tbody>
        {report.models.length === 0 && <tr><td colSpan={4} className="py-6 text-center text-caption text-muted-foreground">No activity in this window.</td></tr>}
        {report.models.map(row => <tr key={`${row.harness}:${row.model}`} className="border-t border-border">
          <td className="py-2"><span className="inline-flex items-center gap-2"><HarnessMark harness={row.harness} size={12} /><span className="font-mono text-caption text-foreground">{row.model}</span>{row.costSource === "unpriced" && <span className="text-[11px] text-muted-foreground">unpriced</span>}</span></td>
          <td className={cn("py-2 text-foreground", NUM)}>{formatUsd(row.costMicrousd)}</td>
          <td className={cn("py-2 text-muted-foreground", NUM)}>{formatPercent(row.costShare)}</td>
          <td className={cn("py-2 text-muted-foreground", NUM)}>{formatTokens(row.tokens)}</td>
        </tr>)}
      </tbody>
    </table> : <table className="w-full text-ui">
      <thead><tr className={TABLE_HEAD}>
        <th className="w-2/5 pb-2 font-medium">{window_.resolution === "hour" ? "Hour" : "Day"}</th>
        {report.harnesses.map(entry => <th key={entry.harness} className={cn("pb-2 font-medium", NUM)}>{harnessLabel(entry.harness)}</th>)}
        <th className={cn("pb-2 font-medium", NUM)}>Total</th><th className={cn("pb-2 font-medium", NUM)}>Tokens</th>
      </tr></thead>
      <tbody>
        {report.periods.every(period => period.tokens === 0 && period.costMicrousd === 0) && <tr><td colSpan={report.harnesses.length + 3} className="py-6 text-center text-caption text-muted-foreground">No activity in this window.</td></tr>}
        {/* Newest first: a 90-period window puts the interesting end at the top. */}
        {[...report.periods].reverse().filter(period => period.tokens > 0 || period.costMicrousd > 0).map(period => <tr key={period.period} className="border-t border-border">
          <td className="py-2 tabular-nums text-foreground">{formatPeriodLabel(period.period, window_.resolution, window_.timeZone)}</td>
          {report.harnesses.map(entry => <td key={entry.harness} className={cn("py-2 text-muted-foreground", NUM)}>{formatUsd(period.costByHarness[entry.harness] ?? 0)}</td>)}
          <td className={cn("py-2 text-foreground", NUM)}>{formatUsd(period.costMicrousd)}</td>
          <td className={cn("py-2 text-muted-foreground", NUM)}>{formatTokens(period.tokens)}</td>
        </tr>)}
      </tbody>
    </table>}
  </>;
}

interface PriceDraft { input: string; output: string; cacheRead: string; cacheWrite: string }

function PriceSection({ report, summary, overrides, refreshing, onRefreshRates, onChange }: {
  report: UsageReport;
  summary: UsageSummaryResult;
  overrides: UsagePriceOverride[];
  refreshing: boolean;
  onRefreshRates: () => void;
  onChange: (next: Promise<UsagePriceOverride[]>) => Promise<void>;
}) {
  const [open, setOpen] = useState(false);
  const [editing, setEditing] = useState<string | null>(null);
  const [draft, setDraft] = useState<PriceDraft>({ input: "", output: "", cacheRead: "", cacheWrite: "" });
  const [invalid, setInvalid] = useState(false);

  // Every model that showed up in the window plus every override: you edit
  // against models you have really used, not a catalog.
  const rows = useMemo(() => {
    const seen = new Map<string, string | null>();
    for (const model of report.models) if (!seen.has(model.model)) seen.set(model.model, model.harness);
    for (const override of overrides) if (!seen.has(override.model)) seen.set(override.model, null);
    return [...seen.entries()].map(([model, harness]) => ({ model, harness, override: overrides.find(item => item.model === model) ?? null })).sort((a, b) => a.model.localeCompare(b.model));
  }, [report.models, overrides]);

  const startEdit = (model: string, override: UsagePriceOverride | null) => {
    setEditing(model);
    setInvalid(false);
    setDraft({ input: microToUsdPerMtok(override?.inputMicrousdPerMtok ?? null), output: microToUsdPerMtok(override?.outputMicrousdPerMtok ?? null), cacheRead: microToUsdPerMtok(override?.cacheReadMicrousdPerMtok), cacheWrite: microToUsdPerMtok(override?.cacheWriteMicrousdPerMtok) });
  };

  const save = async (model: string) => {
    const input = usdPerMtokToMicro(draft.input);
    const output = usdPerMtokToMicro(draft.output);
    const cacheRead = draft.cacheRead.trim() ? usdPerMtokToMicro(draft.cacheRead) : null;
    const cacheWrite = draft.cacheWrite.trim() ? usdPerMtokToMicro(draft.cacheWrite) : null;
    if (input === null || output === null || (draft.cacheRead.trim() && cacheRead === null) || (draft.cacheWrite.trim() && cacheWrite === null)) { setInvalid(true); return; }
    await onChange(bridgeApi.setUsagePriceOverride({ model, inputMicrousdPerMtok: input, outputMicrousdPerMtok: output, cacheReadMicrousdPerMtok: cacheRead, cacheWriteMicrousdPerMtok: cacheWrite }));
    setEditing(null);
  };

  const field = (key: keyof PriceDraft, label: string) => <input aria-label={label} inputMode="decimal" value={draft[key]} onChange={event => setDraft(current => ({ ...current, [key]: event.target.value }))} className="h-7 w-full rounded-md border border-border bg-background px-2 text-right font-mono text-caption tabular-nums text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring" />;

  // Collapsed by default: prices are a setting you visit, not a figure you read.
  return <section className={cn(CARD, open ? "mt-4" : "mt-4 py-3")} aria-label="Model prices">
    <div className="flex items-center gap-2">
      <button type="button" aria-expanded={open} aria-controls="usage-model-prices" onClick={() => setOpen(current => !current)} className="flex min-w-0 flex-1 items-center gap-2 text-left">
        <h2 className="text-ui font-medium text-foreground">Model prices</h2>
        <span className="truncate text-caption tabular-nums text-muted-foreground">{formatCount(rows.length)} {rows.length === 1 ? "model" : "models"} in this window · snapshot {summary.pricing.snapshotDate}{summary.pricing.overrides > 0 ? ` · ${formatCount(summary.pricing.overrides)} overrides` : ""}</span>
        <ChevronDown size={14} className={cn("ml-auto shrink-0 text-muted-foreground transition-transform", open && "rotate-180")} aria-hidden="true" />
      </button>
      <button type="button" onClick={onRefreshRates} disabled={refreshing} aria-label="Refresh rates" aria-busy={refreshing} title={`Refresh rates from ${summary.pricing.source}`} className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-40">
        <RefreshCw size={13} className={refreshing ? "animate-spin" : ""} aria-hidden="true" />
      </button>
    </div>
    {open && <div id="usage-model-prices" className="mt-3">
    <p className="mb-3 text-caption text-muted-foreground">USD per million tokens, from a {formatCount(summary.pricing.knownModels)}-model rate snapshot. Overrides apply to all past and future usage; blank cache rates use the automatic rate.</p>
    <table className="w-full text-ui">
      <thead><tr className={TABLE_HEAD}><th className="w-2/6 pb-2 font-medium">Model</th><th className={cn("pb-2 font-medium", NUM)}>Input</th><th className={cn("pb-2 font-medium", NUM)}>Output</th><th className={cn("pb-2 font-medium", NUM)}>Cache read</th><th className={cn("pb-2 font-medium", NUM)}>Cache write</th><th className="w-28 pb-2" /></tr></thead>
      <tbody>
        {rows.length === 0 && <tr><td colSpan={6} className="py-6 text-center text-caption text-muted-foreground">No models in this window.</td></tr>}
        {rows.map(row => {
          const isEditing = editing === row.model;
          return <tr key={row.model} className="border-t border-border align-middle">
            <td className="py-2"><span className="inline-flex items-center gap-2">{row.harness && <HarnessMark harness={row.harness} size={12} />}<span className="font-mono text-caption text-foreground">{row.model}</span>{row.override && !isEditing && <span className="text-[11px] text-muted-foreground">override</span>}</span></td>
            {isEditing ? <>
              <td className="py-1.5 pl-3">{field("input", `${row.model} input price`)}</td>
              <td className="py-1.5 pl-3">{field("output", `${row.model} output price`)}</td>
              <td className="py-1.5 pl-3">{field("cacheRead", `${row.model} cache read price`)}</td>
              <td className="py-1.5 pl-3">{field("cacheWrite", `${row.model} cache write price`)}</td>
              <td className="py-1.5 pl-3 text-right">
                <button type="button" onClick={() => void save(row.model)} className="rounded-md px-2 py-1 text-caption text-foreground hover:bg-accent">Save</button>
                <button type="button" onClick={() => setEditing(null)} className="rounded-md px-2 py-1 text-caption text-muted-foreground hover:bg-accent">Cancel</button>
              </td>
            </> : <>
              <td className={`text-caption ${cn("py-2 font-mono", NUM, row.override ? "text-foreground" : "text-muted-foreground")}`}>{row.override ? microToUsdPerMtok(row.override.inputMicrousdPerMtok) : "Automatic"}</td>
              <td className={`text-caption ${cn("py-2 font-mono", NUM, row.override ? "text-foreground" : "text-muted-foreground")}`}>{row.override ? microToUsdPerMtok(row.override.outputMicrousdPerMtok) : "Automatic"}</td>
              <td className={`text-caption ${cn("py-2 font-mono", NUM, row.override?.cacheReadMicrousdPerMtok != null ? "text-foreground" : "text-muted-foreground")}`}>{row.override?.cacheReadMicrousdPerMtok != null ? microToUsdPerMtok(row.override.cacheReadMicrousdPerMtok) : "Automatic"}</td>
              <td className={`text-caption ${cn("py-2 font-mono", NUM, row.override?.cacheWriteMicrousdPerMtok != null ? "text-foreground" : "text-muted-foreground")}`}>{row.override?.cacheWriteMicrousdPerMtok != null ? microToUsdPerMtok(row.override.cacheWriteMicrousdPerMtok) : "Automatic"}</td>
              <td className="py-2 text-right">
                <button type="button" onClick={() => startEdit(row.model, row.override)} className="rounded-md px-2 py-1 text-caption text-foreground hover:bg-accent">Edit</button>
                {row.override && <button type="button" aria-label={`Reset ${row.model} to automatic`} onClick={() => void onChange(bridgeApi.clearUsagePriceOverride(row.model))} className="rounded-md p-1 text-muted-foreground hover:bg-accent hover:text-foreground"><RotateCcw size={13} aria-hidden="true" /></button>}
              </td>
            </>}
          </tr>;
        })}
      </tbody>
    </table>
    {invalid && <p role="alert" className="mt-2 text-caption text-destructive">Enter non-negative numbers. Enter 0 for free tokens.</p>}
    </div>}
  </section>;
}
