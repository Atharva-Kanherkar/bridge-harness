import { useCallback, useEffect, useMemo, useState } from "react";
import { ChevronDown, Gauge, RefreshCw, RotateCcw } from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { RowButton } from "@/components/ui/row";
import { Segmented } from "@/components/ui/segmented";
import { SelectField } from "@/components/ui/select";
import { bridgeApi } from "../api";
import { friendlyError } from "../errors";
import { harnessLabel } from "../utils";
import type { UsageHistorySource, UsagePriceOverride, UsageSummaryResult } from "../types";
import { HarnessMark } from "./harnessMarks";
import { SCREEN_CONTENT, ScreenHeading } from "./ui/screen";
import { UsageInsights } from "./UsageInsights";
import { UsageScreenTime } from "./UsageScreenTime";
import { UsageLedger } from "./UsageLedger";
import { UsageStrips } from "./UsageStrips";
import { UsageFlow } from "./UsageFlow";
import { UsageMosaic } from "./UsageMosaic";
import { UsageCalendar } from "./UsageCalendar";
import type { UsageLayoutProps } from "./UsageLayoutParts";
import { UsageLoadingState } from "./UsageLayoutParts";
import { Skeleton } from "@/components/ui/skeleton";
import { useProviderUsageOverviews } from "./UsageDot";
import { UsageResetRow } from "./UsageResetRow";
import {
  buildUsageReport, enumeratePeriods, formatCount, formatDayShort, formatPercent, formatPeriodLabel, formatTokens, formatUsd, formatWindowLabel,
  makeUsageWindow, microToUsdPerMtok, readUsagePreferences, summaryParams, USAGE_LAYOUT_OPTIONS, USAGE_WINDOW_OPTIONS, usdPerMtokToMicro, writeUsagePreferences,
  type UsageLayout, type UsageMetric, type UsagePreferences, type UsageReport, type UsageWindow, type UsageWindowDays,
} from "../usageReport";

// The usage destination: what Bridge's harnesses processed and what it would
// have cost at API rates, over a window, by harness and model. Every figure
// traces to a ledger row or an imported transcript observation; the honesty
// rules are in testing/feat-usage-frontend.md and the notes below say when a
// number is partial, estimated, or de-duplicated.

const CARD = "u-surface rounded-2xl p-4";
const TABLE_HEAD = "text-left text-caption font-medium text-muted-foreground";
const NUM = "text-right tabular-nums";

/// Upper bound on bounded scan batches per auto-scan. Opening Usage must not
/// fire an open-ended series of IPC round trips on a huge history; hitting
/// the cap surfaces as a partial total and Scan history resumes the cursors.
const MAX_SCAN_PASSES = 25;

/// A clean automatic scan this recent is reused when Usage reopens. Each scan
/// stats every transcript and rewrites the source cursors, so rescanning on
/// every visit is the slow part. Refresh and Scan history always scan.
const AUTO_SCAN_FRESH_MS = 5 * 60_000;

// Kept across visits so reopening Usage draws the last figures at once and
// the refetch updates them in place.
const summaryCache = new Map<string, UsageSummaryResult>();
let cachedSources: UsageHistorySource[] = [];
let lastAutoScanAt = 0;

const autoScanDue = () => Date.now() - lastAutoScanAt >= AUTO_SCAN_FRESH_MS;
const summaryKey = (preferences: UsagePreferences) => `${preferences.windowDays}:${preferences.includeImported}`;

export function clearUsageCacheForTests(): void {
  summaryCache.clear();
  cachedSources = [];
  lastAutoScanAt = 0;
}

type Breakdown = "model" | "time";
type UsageTab = "usage" | "insights" | "time";

const LAYOUT_LABELS: Record<UsageLayout, string> = { ledger: "Ledger", strips: "Strips", flow: "Flow", mosaic: "Mosaic", calendar: "Calendar" };

/** The five switchable presentations. */
const LAYOUTS: Record<UsageLayout, (props: UsageLayoutProps) => React.ReactNode> = {
  ledger: UsageLedger,
  strips: UsageStrips,
  flow: UsageFlow,
  mosaic: UsageMosaic,
  calendar: UsageCalendar,
};

function windowLabel(days: UsageWindowDays): string {
  return days === 1 ? "24h" : `${days}d`;
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
  const key = summaryKey(preferences);
  // The figures on screen and the window they were fetched for. A cached
  // window draws at once; another window keeps its figures dimmed until its
  // own summary lands.
  const [shown, setShown] = useState<{ key: string; summary: UsageSummaryResult } | null>(() => {
    const cached = summaryCache.get(key);
    return cached ? { key, summary: cached } : null;
  });
  const summary = shown?.summary ?? null;
  const [loading, setLoading] = useState(true);
  const [sources, setSources] = useState<UsageHistorySource[]>(cachedSources);
  const [overrides, setOverrides] = useState<UsagePriceOverride[]>([]);
  const [tab, setTab] = useState<UsageTab>("usage");
  const [tableOpen, setTableOpen] = useState(false);
  const [scanning, setScanning] = useState(() => preferences.includeImported && autoScanDue());
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
    const cached = summaryCache.get(key);
    if (cached) setShown(current => current?.summary === cached ? current : { key, summary: cached });
    bridgeApi.usageSummary(summaryParams(window_, preferences.includeImported))
      .then(result => {
        if (cancelled) return;
        summaryCache.set(key, result);
        setShown({ key, summary: result });
      })
      .catch(error => {
        if (!cancelled) {
          summaryCache.delete(key);
          setShown(null);
          onError(friendlyError(error, "Couldn't load your usage."));
        }
      })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [window_, key, preferences.includeImported, summaryTick, onError]);

  useEffect(() => {
    bridgeApi.listUsagePriceOverrides().then(setOverrides).catch(() => undefined);
  }, []);

  useEffect(() => {
    let cancelled = false;
    const scanNow = preferences.includeImported && (refreshTick > 0 || autoScanDue());
    setScanning(scanNow);
    setScanFailed(false);
    setScanNote(null);
    const scan = async () => {
      try {
        const discovered = await bridgeApi.listUsageHistorySources();
        if (cancelled) return;
        cachedSources = discovered;
        setSources(discovered);
        if (!scanNow) return;

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
            warnings.add(`History scan stopped after ${MAX_SCAN_PASSES} batches. Scan again to continue.`);
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
              warnings.add(friendlyError(source.warning, `${harnessLabel(source.agent)} history is ${source.coverage}.`));
            }
          }
          setScanFailed(warnings.size > 0);
          setScanNote(`Scanning history. Imported ${formatCount(imported)} records so far.`);
          const updated = await bridgeApi.listUsageHistorySources();
          if (cancelled) return;
          cachedSources = updated;
          setSources(updated);
        } while (sourceIds.length > 0);
        if (warnings.size === 0) lastAutoScanAt = Date.now();
        setScanNote(warnings.size > 0
          ? `Imported ${formatCount(imported)} records. ${[...warnings].join(" ")}`
          : `Imported ${formatCount(imported)} records in ${(durationMs / 1000).toFixed(1)} s.`);
      } catch (error) {
        if (!cancelled) {
          setScanFailed(true);
          setScanNote(`History could not finish. ${friendlyError(error, "")}`.trim());
          onError(friendlyError(error, "Couldn't scan your usage history."));
        }
      } finally {
        if (!cancelled) {
          setScanning(false);
          if (scanNow) {
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
  const LayoutView = LAYOUTS[layout];

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
      onError(friendlyError(error, "Couldn't refresh the prices."));
    } finally {
      setRefreshingRates(false);
    }
  };

  const changeOverrides = async (next: Promise<UsagePriceOverride[]>) => {
    try {
      setOverrides(await next);
      loadSummary();
    } catch (error) {
      onError(friendlyError(error, "Couldn't save the price change."));
    }
  };

  const incompleteSources = preferences.includeImported
    ? sources.filter(source => source.origin !== "dashboard" && source.coverageState !== "complete" && source.coverageState !== "empty")
    : [];
  const blockedSources = incompleteSources.filter(source => source.coverageState === "unsupported" || source.coverageState === "stale" || source.coverageState === "unreadable");
  const otherWindow = shown !== null && shown.key !== key;

  return <div className="h-full min-h-0 overflow-y-auto">
    <div className={SCREEN_CONTENT}>
      <ScreenHeading
        title="Usage"
        action={<span className="inline-flex shrink-0 items-center gap-2">
          <Segmented<UsageTab> label="View" value={tab} options={[{ value: "usage", label: "Usage" }, { value: "insights", label: "Insights" }, { value: "time", label: "Time" }]} onChange={setTab} />
          {onOpenMeter && <Button variant="quiet" size="icon-sm" onClick={onOpenMeter} aria-label="Open usage meter" tooltip="Usage meter"><Gauge aria-hidden="true" /></Button>}
          <Button variant="quiet" size="icon-sm" onClick={() => { setRefreshTick(tick => tick + 1); providerUsage.refresh(); }} disabled={loading || scanning} loading={loading || scanning} aria-label="Refresh usage" tooltip="Refresh usage"><RefreshCw aria-hidden="true" /></Button>
        </span>}
      />

      {/* The column these layouts sit in is the window minus the sidebar, and
          zoom shrinks it further, so their grids break on this container's
          width. Viewport breakpoints put five columns in room for two. */}
      <div className="@container/usage">
      {tab === "time" ? <UsageScreenTime onError={onError} refreshKey={refreshTick} /> : tab === "insights" ? <UsageInsights windowDays={preferences.windowDays} onError={onError} /> : <>
      {providerUsage.overviews?.providers.filter(snapshot => !snapshot.accountId).map(snapshot =>
        <div key={snapshot.provider} className="mb-3 empty:hidden"><UsageResetRow snapshot={snapshot} onUpdated={providerUsage.refresh} /></div>)}
      <div className="mb-5 flex flex-wrap items-start gap-3">
        <Segmented<UsageMetric> label="Metric" value={metric} options={[{ value: "cost", label: "Cost" }, { value: "tokens", label: "Tokens" }]} onChange={value => update({ metric: value })} />
        <Segmented<UsageWindowDays> label="Window" value={preferences.windowDays} options={USAGE_WINDOW_OPTIONS.map(days => ({ value: days, label: windowLabel(days) }))} onChange={value => update({ windowDays: value })} />
        <span className="pt-2 text-caption tabular-nums text-muted-foreground">{formatWindowLabel(window_)}</span>
        <SelectField label="Layout" size="sm" className="ml-auto w-auto" value={layout} onChange={next => update({ layout: next as UsageLayout })} options={USAGE_LAYOUT_OPTIONS.map(option => ({ value: option, label: LAYOUT_LABELS[option] }))} />
      </div>

      {(loading || scanning) && summary && <div role="status" aria-label="Loading usage" className="mb-5"><Skeleton className="h-0.5 w-full rounded-full" /></div>}

      {report && !otherWindow && (scanFailed || blockedSources.length > 0 || report.totals.unpricedRecords > 0) && <ul className="mb-5 space-y-1 text-caption text-muted-foreground" aria-label="Coverage notes">
        {scanFailed && <li>History scan failed</li>}
        {blockedSources.map(source => <li key={source.id}>{harnessLabel(source.agent)}: {source.coverageState === "unsupported" ? "unavailable" : "needs a refresh"}</li>)}
        {report.totals.unpricedRecords > 0 && <li>{formatCount(report.totals.unpricedRecords)} unpriced {report.totals.unpricedRecords === 1 ? "record" : "records"}</li>}
      </ul>}

      {loading && !summary ? <UsageLoadingState /> : report && <>
        <div aria-busy={loading || scanning} className={cn("transition-opacity", otherWindow && "opacity-50")}>
          <LayoutView report={report} summary={summary!} periods={periods} window={window_} windowDays={preferences.windowDays} metric={metric} />
          <section className={cn(CARD, "mt-6 py-3")} aria-label="Breakdown table">
            <RowButton size="bare" tone="plain" aria-expanded={tableOpen} aria-controls="usage-breakdown-table" onClick={() => setTableOpen(open => !open)}>
              <h2 className="text-ui font-medium text-foreground">Breakdown table</h2>
              <span className="text-caption text-muted-foreground">{formatCount(report.models.length)} {report.models.length === 1 ? "model" : "models"} · {formatCount(report.harnesses.length)} {report.harnesses.length === 1 ? "harness" : "harnesses"}</span>
              <ChevronDown size={14} className={cn("ml-auto text-muted-foreground transition-transform", tableOpen && "rotate-180")} aria-hidden="true" />
            </RowButton>
            {tableOpen && <div id="usage-breakdown-table" className="mt-3"><BreakdownTables report={report} window={window_} /></div>}
          </section>
        </div>

        <section className={cn(CARD, "mt-4")} aria-label="History sources">
          <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
            <div>
              <h2 className="text-ui font-medium text-foreground">History sources</h2>
            </div>
            <Button variant="outline" size="sm" onClick={() => void scan()} disabled={scanning} loading={scanning}>
              <RefreshCw aria-hidden="true" />Scan history
            </Button>
          </div>
          {scanNote && <p role="status" className="mb-3 text-caption text-muted-foreground">{scanNote}</p>}
          <ul className="divide-y divide-border">
            {sources.length === 0 && <li className="py-3 text-caption text-muted-foreground">No history sources were found.</li>}
            {sources.map(source => <li key={source.id} className="flex flex-wrap items-start justify-between gap-x-4 gap-y-1 py-2.5">
              <div className="min-w-0">
                <div className="inline-flex items-center gap-2 text-ui text-foreground"><HarnessMark harness={source.agent} size={13} />{harnessLabel(source.agent)}<span className={cn("text-caption", coverageTone(source.coverageState))}>{source.origin === "dashboard" && source.coverageState === "unreadable" ? "Unavailable" : source.origin === "dashboard" && source.coverageState === "unsupported" ? "Disabled" : coverageLabel(source.coverageState)}</span></div>
                <p className="text-caption text-muted-foreground">{source.origin === "dashboard" ? "Dashboard usage · last 30 days" : source.capability === "supported" ? "Local usage history" : "History is not available for this harness"}</p>
                {(source.location || source.coverageReason || source.lastError) && <details className="mt-1 text-caption text-muted-foreground">
                  <summary className="w-fit cursor-pointer select-none hover:text-foreground">Technical details</summary>
                  <div className="mt-1 space-y-1 border-l border-border pl-2">
                    {source.location && <div className="break-all font-mono">{source.location}</div>}
                    {(source.coverageReason || source.lastError) && <div>{source.lastError ?? source.coverageReason}</div>}
                  </div>
                </details>}
              </div>
              {source.capability === "supported" && <div className="text-right text-caption tabular-nums text-muted-foreground">
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
    </div>
  </div>;
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
          <td className="py-2"><span className="inline-flex items-center gap-2"><HarnessMark harness={row.harness} size={12} /><span className="font-mono text-caption text-foreground">{row.model}</span>{row.costSource === "unpriced" && <span className="text-caption text-muted-foreground">unpriced</span>}</span></td>
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

  const field = (key: keyof PriceDraft, label: string) => <Input size="sm" mono aria-label={label} inputMode="decimal" value={draft[key]} onChange={event => setDraft(current => ({ ...current, [key]: event.target.value }))} />;

  // Collapsed by default: prices are a setting you visit, not a figure you read.
  return <section className={cn(CARD, open ? "mt-4" : "mt-4 py-3")} aria-label="Model prices">
    <div className="flex items-center gap-2">
      <RowButton size="bare" tone="plain" aria-expanded={open} aria-controls="usage-model-prices" onClick={() => setOpen(current => !current)} className="flex-1">
        <h2 className="text-ui font-medium text-foreground">Model prices</h2>
        <span className="truncate text-caption tabular-nums text-muted-foreground">{formatCount(rows.length)} {rows.length === 1 ? "model" : "models"} in this window · snapshot {summary.pricing.snapshotDate}{summary.pricing.overrides > 0 ? ` · ${formatCount(summary.pricing.overrides)} overrides` : ""}</span>
        <ChevronDown size={14} className={cn("ml-auto shrink-0 text-muted-foreground transition-transform", open && "rotate-180")} aria-hidden="true" />
      </RowButton>
      <Button variant="quiet" size="icon-sm" onClick={onRefreshRates} disabled={refreshing} loading={refreshing} aria-label="Refresh rates" tooltip={`Refresh rates from ${summary.pricing.source}`}>
        <RefreshCw aria-hidden="true" />
      </Button>
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
            <td className="py-2"><span className="inline-flex items-center gap-2">{row.harness && <HarnessMark harness={row.harness} size={12} />}<span className="font-mono text-caption text-foreground">{row.model}</span>{row.override && !isEditing && <span className="text-caption text-muted-foreground">override</span>}</span></td>
            {isEditing ? <>
              <td className="py-1.5 pl-3">{field("input", `${row.model} input price`)}</td>
              <td className="py-1.5 pl-3">{field("output", `${row.model} output price`)}</td>
              <td className="py-1.5 pl-3">{field("cacheRead", `${row.model} cache read price`)}</td>
              <td className="py-1.5 pl-3">{field("cacheWrite", `${row.model} cache write price`)}</td>
              <td className="py-1.5 pl-3 text-right">
                <Button variant="ghost" size="2xs" onClick={() => void save(row.model)}>Save</Button>
                <Button variant="quiet" size="2xs" onClick={() => setEditing(null)}>Cancel</Button>
              </td>
            </> : <>
              <td className={`text-caption ${cn("py-2 font-mono", NUM, row.override ? "text-foreground" : "text-muted-foreground")}`}>{row.override ? microToUsdPerMtok(row.override.inputMicrousdPerMtok) : "Automatic"}</td>
              <td className={`text-caption ${cn("py-2 font-mono", NUM, row.override ? "text-foreground" : "text-muted-foreground")}`}>{row.override ? microToUsdPerMtok(row.override.outputMicrousdPerMtok) : "Automatic"}</td>
              <td className={`text-caption ${cn("py-2 font-mono", NUM, row.override?.cacheReadMicrousdPerMtok != null ? "text-foreground" : "text-muted-foreground")}`}>{row.override?.cacheReadMicrousdPerMtok != null ? microToUsdPerMtok(row.override.cacheReadMicrousdPerMtok) : "Automatic"}</td>
              <td className={`text-caption ${cn("py-2 font-mono", NUM, row.override?.cacheWriteMicrousdPerMtok != null ? "text-foreground" : "text-muted-foreground")}`}>{row.override?.cacheWriteMicrousdPerMtok != null ? microToUsdPerMtok(row.override.cacheWriteMicrousdPerMtok) : "Automatic"}</td>
              <td className="py-2 text-right">
                <Button variant="ghost" size="2xs" onClick={() => startEdit(row.model, row.override)}>Edit</Button>
                {row.override && <Button variant="quiet" size="icon-2xs" aria-label={`Reset ${row.model} to automatic`} tooltip="Reset to automatic" onClick={() => void onChange(bridgeApi.clearUsagePriceOverride(row.model))}><RotateCcw aria-hidden="true" /></Button>}
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
