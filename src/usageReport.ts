import type { SummaryParams, UsageBucket, UsageCostSource, UsageResolution, UsageSummaryResult } from "./types";

// The usage screen's pure half: the window it asks for, the roll-up it shows,
// the chart geometry, and the persisted controls. Everything here is
// deterministic and Intl-driven with an explicit locale so tests do not depend
// on the machine. The honesty rules from testing/feat-usage-frontend.md live
// here too: reasoning is a breakdown and never an addend, unpriced buckets
// count tokens and zero cost, and no number is ever invented.

export type UsageMetric = "cost" | "tokens";
export type UsageWindowDays = 1 | 7 | 30 | 90;
export const USAGE_WINDOW_OPTIONS: readonly UsageWindowDays[] = [1, 7, 30, 90];

export interface UsagePreferences {
  metric: UsageMetric;
  windowDays: UsageWindowDays;
  includeImported: boolean;
}

export const USAGE_PREFERENCES_KEY = "bridge.usage.preferences.v1";
export const DEFAULT_USAGE_PREFERENCES: UsagePreferences = { metric: "cost", windowDays: 30, includeImported: true };

export function readUsagePreferences(storage: Pick<Storage, "getItem"> | undefined = globalThis.localStorage): UsagePreferences {
  if (!storage) return { ...DEFAULT_USAGE_PREFERENCES };
  try {
    const raw = storage.getItem(USAGE_PREFERENCES_KEY);
    if (!raw) return { ...DEFAULT_USAGE_PREFERENCES };
    const parsed = JSON.parse(raw) as Partial<UsagePreferences> | null;
    const metric = parsed?.metric === "tokens" ? "tokens" : parsed?.metric === "cost" ? "cost" : null;
    const windowDays = USAGE_WINDOW_OPTIONS.find(option => option === parsed?.windowDays) ?? null;
    if (!metric || !windowDays || typeof parsed?.includeImported !== "boolean") return { ...DEFAULT_USAGE_PREFERENCES };
    return { metric, windowDays, includeImported: parsed.includeImported };
  } catch {
    return { ...DEFAULT_USAGE_PREFERENCES };
  }
}

export function writeUsagePreferences(preferences: UsagePreferences, storage: Pick<Storage, "setItem"> | undefined = globalThis.localStorage): void {
  storage?.setItem(USAGE_PREFERENCES_KEY, JSON.stringify(preferences));
}

// ---------------------------------------------------------------------------
// Window

export interface UsageWindow {
  sinceDay: string;
  untilDay: string;
  timeZone: string;
  resolution: UsageResolution;
  sinceTime?: string;
  untilTime?: string;
}

const HOUR_MS = 3_600_000;
const DAY_MS = 86_400_000;

/** The IANA zone the summary buckets by; an offset would drift across DST. */
export function resolveTimeZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
  } catch {
    return "UTC";
  }
}

/** `YYYY-MM-DD` for an instant in a zone, from parts so no local-zone Date math leaks in. */
export function dayInZone(at: Date, timeZone: string): string {
  const parts = new Intl.DateTimeFormat("en-US", { timeZone, year: "numeric", month: "2-digit", day: "2-digit" }).formatToParts(at);
  const read = (type: string) => parts.find(part => part.type === type)?.value ?? "";
  return `${read("year")}-${read("month")}-${read("day")}`;
}

function shiftDay(day: string, deltaDays: number): string {
  const [year, month, dayOfMonth] = day.split("-").map(Number);
  return new Date(Date.UTC(year, month - 1, dayOfMonth + deltaDays)).toISOString().slice(0, 10);
}

/** Second-precision UTC, the shape `usage/summary` echoes back in `hourStart`. */
export function isoSeconds(at: Date): string {
  return at.toISOString().replace(/\.\d{3}Z$/, "Z");
}

export function makeUsageWindow(days: UsageWindowDays, now: Date = new Date(), timeZone: string = resolveTimeZone()): UsageWindow {
  if (days === 1) {
    // Fixed-duration hour buckets, minute-floored so a refresh a few seconds
    // later lands on the same key.
    const until = new Date(Math.floor(now.getTime() / 60_000) * 60_000);
    const since = new Date(until.getTime() - 24 * HOUR_MS);
    return { sinceDay: dayInZone(since, timeZone), untilDay: dayInZone(until, timeZone), timeZone, resolution: "hour", sinceTime: isoSeconds(since), untilTime: isoSeconds(until) };
  }
  // Calendar arithmetic on the local end day: subtracting milliseconds from
  // `now` lands on the wrong calendar day around a DST transition.
  const untilDay = dayInZone(now, timeZone);
  return { sinceDay: shiftDay(untilDay, -(days - 1)), untilDay, timeZone, resolution: "day" };
}

export function summaryParams(window: UsageWindow, includeImported: boolean): SummaryParams {
  return {
    sinceDay: window.sinceDay,
    untilDay: window.untilDay,
    timeZone: window.timeZone,
    resolution: window.resolution,
    sinceTime: window.sinceTime ?? null,
    untilTime: window.untilTime ?? null,
    includeImported,
    workspaceId: null,
  };
}

/** Every period key in the window, dense, so gaps chart as zero rather than vanish. */
export function enumeratePeriods(window: UsageWindow): string[] {
  if (window.resolution === "hour" && window.sinceTime && window.untilTime) {
    const since = Date.parse(window.sinceTime);
    const until = Date.parse(window.untilTime);
    const periods: string[] = [];
    for (let at = since; at < until; at += HOUR_MS) periods.push(isoSeconds(new Date(at)));
    return periods;
  }
  const periods: string[] = [];
  const last = Date.parse(`${window.untilDay}T00:00:00Z`);
  for (let at = Date.parse(`${window.sinceDay}T00:00:00Z`); at <= last; at += DAY_MS) periods.push(new Date(at).toISOString().slice(0, 10));
  return periods;
}

export function periodKey(bucket: UsageBucket, resolution: UsageResolution): string {
  if (resolution === "hour" && bucket.hourStart) {
    const parsed = Date.parse(bucket.hourStart);
    return Number.isNaN(parsed) ? bucket.hourStart : isoSeconds(new Date(parsed));
  }
  return bucket.day;
}

// ---------------------------------------------------------------------------
// Formatting

const CURRENCY = new Intl.NumberFormat("en-US", { style: "currency", currency: "USD", minimumFractionDigits: 2, maximumFractionDigits: 2 });
const INTEGER = new Intl.NumberFormat("en-US");
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

export function microToUsd(micro: number): number {
  return micro / 1_000_000;
}

/** Two decimals always; a positive amount under a cent floors to `<$0.01` rather than lying as `$0.00`. */
export function formatUsd(micro: number): string {
  const usd = microToUsd(micro);
  if (usd > 0 && usd < 0.005) return "<$0.01";
  return CURRENCY.format(usd);
}

export function formatCount(value: number): string {
  return INTEGER.format(Math.round(value));
}

function trim(value: number): string {
  const abs = Math.abs(value);
  const digits = abs >= 100 ? 0 : abs >= 10 ? 1 : 2;
  return value.toFixed(digits).replace(/\.0+$/, "").replace(/(\.\d)0$/, "$1");
}

/** Three significant figures with a unit suffix so columns line up: `19.9B`, `76.7M`, `804K`. */
export function formatTokens(value: number): string {
  const abs = Math.abs(value);
  if (abs >= 1e12) return `${trim(value / 1e12)}T`;
  if (abs >= 1e9) return `${trim(value / 1e9)}B`;
  if (abs >= 1e6) return `${trim(value / 1e6)}M`;
  if (abs >= 1e3) return `${trim(value / 1e3)}K`;
  return INTEGER.format(Math.round(value));
}

export function formatPercent(share: number, digits = 1): string {
  return `${(share * 100).toFixed(digits)}%`;
}

export function formatDayShort(day: string): string {
  const [, month, dayOfMonth] = day.split("-").map(Number);
  return `${MONTHS[(month || 1) - 1]} ${dayOfMonth || ""}`.trim();
}

export function formatHourShort(iso: string, timeZone: string): string {
  const at = new Date(iso);
  if (Number.isNaN(at.getTime())) return iso;
  return new Intl.DateTimeFormat("en-US", { timeZone, hour: "numeric", hour12: true }).format(at);
}

export function formatPeriodLabel(period: string, resolution: UsageResolution, timeZone: string): string {
  return resolution === "hour" ? formatHourShort(period, timeZone) : formatDayShort(period);
}

/** `Aug 7 to Sep 6` for days, `Aug 11, 2 PM to Aug 12, 2 PM` for the rolling day. */
export function formatWindowLabel(window: UsageWindow): string {
  if (window.resolution === "hour" && window.sinceTime && window.untilTime) {
    const stamp = (iso: string) => `${formatDayShort(dayInZone(new Date(iso), window.timeZone))}, ${formatHourShort(iso, window.timeZone)}`;
    return `${stamp(window.sinceTime)} to ${stamp(window.untilTime)}`;
  }
  if (window.sinceDay === window.untilDay) return formatDayShort(window.untilDay);
  return `${formatDayShort(window.sinceDay)} to ${formatDayShort(window.untilDay)}`;
}

// ---------------------------------------------------------------------------
// Roll-up

export interface UsageTotals {
  uncachedInputTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
  outputTokens: number;
  reasoningTokens: number;
  /** uncached + cache read + cache write + output. Reasoning is inside output. */
  processedTokens: number;
  costMicrousd: number;
  cacheSavingsMicrousd: number;
  records: number;
  unpricedRecords: number;
}

export interface HarnessReport extends UsageTotals {
  harness: string;
  costShare: number;
  tokenShare: number;
}

export interface ModelReport {
  harness: string;
  model: string;
  tokens: number;
  costMicrousd: number;
  costShare: number;
  costSource: UsageCostSource;
}

export interface PeriodReport {
  period: string;
  costByHarness: Record<string, number>;
  tokensByHarness: Record<string, number>;
  costMicrousd: number;
  tokens: number;
}

/** The weakest provenance in the window: one unpriced bucket makes the whole cost partly unpriced. */
export type WindowCostSource = UsageCostSource | "none";

export interface UsageReport {
  totals: UsageTotals;
  harnesses: HarnessReport[];
  models: ModelReport[];
  periods: PeriodReport[];
  costSource: WindowCostSource;
}

function emptyTotals(): UsageTotals {
  return { uncachedInputTokens: 0, cacheReadTokens: 0, cacheWriteTokens: 0, outputTokens: 0, reasoningTokens: 0, processedTokens: 0, costMicrousd: 0, cacheSavingsMicrousd: 0, records: 0, unpricedRecords: 0 };
}

export function bucketTokens(bucket: UsageBucket): number {
  const { uncachedInputTokens, cacheReadTokens, cacheWriteTokens, outputTokens } = bucket.totals;
  return uncachedInputTokens + cacheReadTokens + cacheWriteTokens + outputTokens;
}

function accumulate(into: UsageTotals, bucket: UsageBucket): void {
  into.uncachedInputTokens += bucket.totals.uncachedInputTokens;
  into.cacheReadTokens += bucket.totals.cacheReadTokens;
  into.cacheWriteTokens += bucket.totals.cacheWriteTokens;
  into.outputTokens += bucket.totals.outputTokens;
  into.reasoningTokens += bucket.totals.reasoningTokens;
  into.processedTokens += bucketTokens(bucket);
  into.costMicrousd += bucket.costMicrousd;
  into.cacheSavingsMicrousd += bucket.cacheSavingsMicrousd;
  into.records += bucket.records;
  into.unpricedRecords += bucket.unpricedRecords;
}

const SOURCE_RANK: Record<UsageCostSource, number> = { provider_reported: 0, model_priced: 1, unpriced: 2 };

export function weakestCostSource(buckets: readonly UsageBucket[]): WindowCostSource {
  let weakest: UsageCostSource | null = null;
  for (const bucket of buckets) {
    if (!weakest || SOURCE_RANK[bucket.costSource] > SOURCE_RANK[weakest]) weakest = bucket.costSource;
  }
  return weakest ?? "none";
}

function share(part: number, whole: number): number {
  return whole > 0 ? part / whole : 0;
}

export function buildUsageReport(summary: Pick<UsageSummaryResult, "buckets" | "resolution">, periods: readonly string[]): UsageReport {
  const totals = emptyTotals();
  const byHarness = new Map<string, UsageTotals>();
  const byModel = new Map<string, ModelReport>();
  const byPeriod = new Map<string, PeriodReport>(periods.map(period => [period, { period, costByHarness: {}, tokensByHarness: {}, costMicrousd: 0, tokens: 0 }]));

  for (const bucket of summary.buckets) {
    accumulate(totals, bucket);
    const harness = byHarness.get(bucket.harness) ?? emptyTotals();
    accumulate(harness, bucket);
    byHarness.set(bucket.harness, harness);

    const modelKey = `${bucket.harness}:${bucket.model}`;
    const model = byModel.get(modelKey) ?? { harness: bucket.harness, model: bucket.model, tokens: 0, costMicrousd: 0, costShare: 0, costSource: bucket.costSource };
    model.tokens += bucketTokens(bucket);
    model.costMicrousd += bucket.costMicrousd;
    if (SOURCE_RANK[bucket.costSource] > SOURCE_RANK[model.costSource]) model.costSource = bucket.costSource;
    byModel.set(modelKey, model);

    const key = periodKey(bucket, summary.resolution);
    const period = byPeriod.get(key) ?? { period: key, costByHarness: {}, tokensByHarness: {}, costMicrousd: 0, tokens: 0 };
    period.costByHarness[bucket.harness] = (period.costByHarness[bucket.harness] ?? 0) + bucket.costMicrousd;
    period.tokensByHarness[bucket.harness] = (period.tokensByHarness[bucket.harness] ?? 0) + bucketTokens(bucket);
    period.costMicrousd += bucket.costMicrousd;
    period.tokens += bucketTokens(bucket);
    byPeriod.set(key, period);
  }

  // Active means any tokens or any cost, independent of the displayed metric,
  // so switching Cost and Tokens never adds or removes a row or a series.
  const harnesses = [...byHarness.entries()]
    .filter(([, value]) => value.processedTokens > 0 || value.costMicrousd > 0)
    .map(([harness, value]) => ({ harness, ...value, costShare: share(value.costMicrousd, totals.costMicrousd), tokenShare: share(value.processedTokens, totals.processedTokens) }))
    .sort((a, b) => b.costMicrousd - a.costMicrousd || b.processedTokens - a.processedTokens || a.harness.localeCompare(b.harness));

  const models = [...byModel.values()]
    .filter(model => model.tokens > 0 || model.costMicrousd > 0)
    .map(model => ({ ...model, costShare: share(model.costMicrousd, totals.costMicrousd) }))
    .sort((a, b) => b.costMicrousd - a.costMicrousd || b.tokens - a.tokens || a.model.localeCompare(b.model));

  // Keep the dense order the caller enumerated; a bucket outside the window
  // (a zone edge) lands at the end rather than being dropped.
  const ordered = periods.map(period => byPeriod.get(period)!);
  for (const [key, period] of byPeriod) if (!periods.includes(key)) ordered.push(period);

  return { totals, harnesses, models, periods: ordered, costSource: weakestCostSource(summary.buckets) };
}

export function costSourceLabel(source: WindowCostSource): string {
  switch (source) {
    case "provider_reported": return "Provider reported";
    case "model_priced": return "Model priced";
    case "unpriced": return "Partly unpriced";
    default: return "No cost data";
  }
}

// ---------------------------------------------------------------------------
// Chart geometry

export interface ChartSeries {
  harness: string;
  values: number[];
  total: number;
}

export function buildChartSeries(report: UsageReport, metric: UsageMetric): ChartSeries[] {
  return report.harnesses
    .map(entry => {
      const values = report.periods.map(period => (metric === "cost" ? period.costByHarness[entry.harness] : period.tokensByHarness[entry.harness]) ?? 0);
      return { harness: entry.harness, values, total: values.reduce((sum, value) => sum + value, 0) };
    })
    .sort((a, b) => b.total - a.total || a.harness.localeCompare(b.harness));
}

/** Round the axis up to a 1/2/5 step so the ticks read as numbers a person would pick. */
export function niceScale(peak: number, count: number): { max: number; ticks: number[] } {
  if (!(peak > 0) || !Number.isFinite(peak)) return { max: 0, ticks: [0] };
  const rawStep = peak / Math.max(1, count);
  const magnitude = 10 ** Math.floor(Math.log10(rawStep));
  const normalized = rawStep / magnitude;
  const step = (normalized > 5 ? 10 : normalized > 2 ? 5 : normalized > 1 ? 2 : 1) * magnitude;
  const max = Math.ceil(peak / step) * step;
  const ticks: number[] = [];
  for (let value = 0; value <= max + step * 1e-6; value += step) ticks.push(Number(value.toPrecision(12)));
  return { max, ticks };
}

// Fritsch–Carlson monotone cubic tangents: shape-preserving, so a spike in one
// period cannot make the curve dip below zero or overshoot its neighbours.
function monotoneTangents(points: readonly { x: number; y: number }[]): number[] {
  const n = points.length;
  if (n < 2) return points.map(() => 0);
  const slopes: number[] = [];
  for (let i = 0; i < n - 1; i += 1) {
    const dx = points[i + 1].x - points[i].x;
    slopes.push(dx === 0 ? 0 : (points[i + 1].y - points[i].y) / dx);
  }
  const tangents: number[] = [slopes[0]];
  for (let i = 1; i < n - 1; i += 1) {
    const a = slopes[i - 1];
    const b = slopes[i];
    tangents.push(a * b <= 0 ? 0 : (a + b) / 2);
  }
  tangents.push(slopes[n - 2]);
  for (let i = 0; i < n - 1; i += 1) {
    if (slopes[i] === 0) { tangents[i] = 0; tangents[i + 1] = 0; continue; }
    const alpha = tangents[i] / slopes[i];
    const beta = tangents[i + 1] / slopes[i];
    const magnitude = alpha * alpha + beta * beta;
    if (magnitude > 9) {
      const scale = 3 / Math.sqrt(magnitude);
      tangents[i] = scale * alpha * slopes[i];
      tangents[i + 1] = scale * beta * slopes[i];
    }
  }
  return tangents;
}

export interface ChartGeometry {
  width: number;
  height: number;
  plotTop: number;
}

export const CHART_GEOMETRY: ChartGeometry = { width: 960, height: 260, plotTop: 8 };

export function chartX(index: number, count: number, geometry: ChartGeometry = CHART_GEOMETRY): number {
  return count <= 1 ? 0 : (index / (count - 1)) * geometry.width;
}

export function chartY(value: number, max: number, geometry: ChartGeometry = CHART_GEOMETRY): number {
  if (!(max > 0)) return geometry.height;
  return geometry.height - (value / max) * (geometry.height - geometry.plotTop);
}

const fixed = (value: number) => Number(value.toFixed(2)).toString();

/** The line and closed area paths for one series over the dense period axis. */
export function seriesPaths(values: readonly number[], max: number, geometry: ChartGeometry = CHART_GEOMETRY): { line: string; area: string } {
  if (values.length === 0) return { line: "", area: "" };
  const points = values.map((value, index) => ({ x: chartX(index, values.length, geometry), y: chartY(value, max, geometry) }));
  if (points.length === 1) {
    const y = fixed(points[0].y);
    return { line: `M0,${y} L${geometry.width},${y}`, area: `M0,${y} L${geometry.width},${y} L${geometry.width},${geometry.height} L0,${geometry.height} Z` };
  }
  const tangents = monotoneTangents(points);
  let line = `M${fixed(points[0].x)},${fixed(points[0].y)}`;
  for (let i = 0; i < points.length - 1; i += 1) {
    const p0 = points[i];
    const p1 = points[i + 1];
    const dx = (p1.x - p0.x) / 3;
    line += ` C${fixed(p0.x + dx)},${fixed(p0.y + tangents[i] * dx)} ${fixed(p1.x - dx)},${fixed(p1.y - tangents[i + 1] * dx)} ${fixed(p1.x)},${fixed(p1.y)}`;
  }
  const area = `${line} L${geometry.width},${geometry.height} L0,${geometry.height} Z`;
  return { line, area };
}

/** Snap a pointer fraction across the plot to the nearest period index. */
export function hoverIndex(fraction: number, count: number): number {
  if (count <= 1) return 0;
  return Math.min(count - 1, Math.max(0, Math.round(fraction * (count - 1))));
}

/** First, middle, last: the only x labels a 90-period axis can carry legibly. */
export function axisLabelIndices(count: number): number[] {
  if (count <= 0) return [];
  if (count === 1) return [0];
  if (count === 2) return [0, 1];
  return [0, Math.floor((count - 1) / 2), count - 1];
}

// ---------------------------------------------------------------------------
// Price overrides

export const MICRO_PER_MTOK_PER_USD = 1_000_000;

/** USD per million tokens as typed → integer micro-USD per million tokens on the wire. */
export function usdPerMtokToMicro(text: string): number | null {
  const trimmed = text.trim();
  if (!trimmed) return null;
  const value = Number(trimmed);
  if (!Number.isFinite(value) || value < 0) return null;
  return Math.round(value * MICRO_PER_MTOK_PER_USD);
}

export function microToUsdPerMtok(micro: number | null | undefined): string {
  if (micro === null || micro === undefined) return "";
  return (micro / MICRO_PER_MTOK_PER_USD).toFixed(2).replace(/\.?0+$/, "") || "0";
}
