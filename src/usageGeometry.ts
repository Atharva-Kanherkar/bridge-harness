import type { UsageResolution, UsageSummaryResult } from "./types";
import { buildUsageReport, chartY, periodKey, type ChartGeometry, type HarnessReport, type ModelReport, type PeriodReport, type UsageMetric, type UsageReport } from "./usageReport";

// Pure geometry for the switchable Usage layouts (testing/feat-usage-layouts.md).
// Every helper reads a `UsageReport` that `buildUsageReport` already built, so
// the layouts differ only in how they draw the same numbers. Nothing here
// smooths a series, invents a value, or adds reasoning to processed tokens.

// ---------------------------------------------------------------------------
// Metric accessors

export type TokenKind = "cacheReadTokens" | "cacheWriteTokens" | "uncachedInputTokens" | "outputTokens";

/** The four parts of processed tokens, in the order the layouts draw them. */
export const TOKEN_KINDS: readonly { key: TokenKind; label: string }[] = [
  { key: "cacheReadTokens", label: "Cache read" },
  { key: "cacheWriteTokens", label: "Cache write" },
  { key: "uncachedInputTokens", label: "Uncached input" },
  { key: "outputTokens", label: "Output" },
];

export function harnessValue(entry: HarnessReport, metric: UsageMetric): number {
  return metric === "cost" ? entry.costMicrousd : entry.processedTokens;
}

export function modelValue(model: ModelReport, metric: UsageMetric): number {
  return metric === "cost" ? model.costMicrousd : model.tokens;
}

export function modelSeries(model: ModelReport, metric: UsageMetric): number[] {
  return metric === "cost" ? model.costByPeriod : model.tokensByPeriod;
}

export function periodValue(period: PeriodReport, metric: UsageMetric): number {
  return metric === "cost" ? period.costMicrousd : period.tokens;
}

export function periodHarnessValue(period: PeriodReport, harness: string, metric: UsageMetric): number {
  return (metric === "cost" ? period.costByHarness[harness] : period.tokensByHarness[harness]) ?? 0;
}

/** Harnesses largest first by the displayed metric; the set itself never depends on it. */
export function harnessesByMetric(report: UsageReport, metric: UsageMetric): HarnessReport[] {
  return [...report.harnesses].sort((a, b) => harnessValue(b, metric) - harnessValue(a, metric) || a.harness.localeCompare(b.harness));
}

/** One harness's models, largest first by the displayed metric. */
export function modelsOf(report: UsageReport, harness: string, metric: UsageMetric): ModelReport[] {
  return report.models.filter(model => model.harness === harness).sort((a, b) => modelValue(b, metric) - modelValue(a, metric) || a.model.localeCompare(b.model));
}

// ---------------------------------------------------------------------------
// Lines and axes

const fixed = (value: number) => Number(value.toFixed(2)).toString();

/** Straight segments between the real points. A line that curves would draw values the data never had. */
export function straightPaths(values: readonly number[], max: number, geometry: ChartGeometry): { line: string; area: string } {
  if (values.length === 0) return { line: "", area: "" };
  const baseline = fixed(geometry.height);
  if (values.length === 1) {
    const y = fixed(chartY(values[0], max, geometry));
    return { line: `M0,${y} L${geometry.width},${y}`, area: `M0,${y} L${geometry.width},${y} L${geometry.width},${baseline} L0,${baseline} Z` };
  }
  const points = values.map((value, index) => `${fixed((index / (values.length - 1)) * geometry.width)},${fixed(chartY(value, max, geometry))}`);
  const line = `M${points.join(" L")}`;
  return { line, area: `${line} L${geometry.width},${baseline} L0,${baseline} Z` };
}

/** Up to `target` evenly spaced indices, always including the first and last. */
export function evenTicks(count: number, target = 5): number[] {
  if (count <= 0) return [];
  if (count === 1) return [0];
  const steps = Math.min(target, count);
  const ticks = Array.from({ length: steps }, (_, index) => Math.round((index * (count - 1)) / (steps - 1)));
  return [...new Set(ticks)];
}

/** Horizontal position of a period on a dense axis, as a 0–1 fraction. */
export function axisFraction(index: number, count: number): number {
  return count <= 1 ? 0.5 : index / (count - 1);
}

// ---------------------------------------------------------------------------
// Stacked columns

export interface StackSegment { harness: string; value: number; start: number; end: number }
export interface StackColumn { period: string; total: number; segments: StackSegment[] }

/** Per period, the harnesses stacked bottom-up in the given order. `end` of the last segment is the period total. */
export function stackColumns(periods: readonly PeriodReport[], harnesses: readonly string[], metric: UsageMetric): StackColumn[] {
  return periods.map(period => {
    let running = 0;
    const segments: StackSegment[] = [];
    for (const harness of harnesses) {
      const value = periodHarnessValue(period, harness, metric);
      if (value <= 0) continue;
      segments.push({ harness, value, start: running, end: running + value });
      running += value;
    }
    return { period: period.period, total: running, segments };
  });
}

// ---------------------------------------------------------------------------
// Treemap

export interface Rect { x: number; y: number; w: number; h: number }
export interface Tile<T> extends Rect { item: T; value: number }

function worstRatio(areas: readonly number[], side: number): number {
  const sum = areas.reduce((total, area) => total + area, 0);
  const most = Math.max(...areas);
  const least = Math.min(...areas);
  return Math.max((side * side * most) / (sum * sum), (sum * sum) / (side * side * least));
}

/** Squarified treemap (Bruls, Huizing, van Wijk): areas proportional to value, tiles as square as the row allows. */
export function squarify<T>(items: readonly { item: T; value: number }[], rect: Rect): Tile<T>[] {
  const positive = items.filter(entry => entry.value > 0).sort((a, b) => b.value - a.value);
  const total = positive.reduce((sum, entry) => sum + entry.value, 0);
  if (total <= 0 || rect.w <= 0 || rect.h <= 0) return [];
  const scale = (rect.w * rect.h) / total;
  let rest = positive.map(entry => ({ ...entry, area: entry.value * scale }));
  let { x, y, w, h } = rect;
  const out: Tile<T>[] = [];
  while (rest.length > 0) {
    const side = Math.min(w, h);
    let count = 1;
    while (count < rest.length && worstRatio(rest.slice(0, count + 1).map(entry => entry.area), side) <= worstRatio(rest.slice(0, count).map(entry => entry.area), side)) count += 1;
    const row = rest.slice(0, count);
    const rowArea = row.reduce((sum, entry) => sum + entry.area, 0);
    // The last row takes whatever is left so floating error never leaves a sliver.
    const last = count === rest.length;
    if (w >= h) {
      const width = last ? w : rowArea / h;
      let cursor = y;
      row.forEach((entry, index) => {
        const height = index === row.length - 1 ? y + h - cursor : entry.area / width;
        out.push({ item: entry.item, value: entry.value, x, y: cursor, w: width, h: height });
        cursor += height;
      });
      x += width; w -= width;
    } else {
      const height = last ? h : rowArea / w;
      let cursor = x;
      row.forEach((entry, index) => {
        const width = index === row.length - 1 ? x + w - cursor : entry.area / height;
        out.push({ item: entry.item, value: entry.value, x: cursor, y, w: width, h: height });
        cursor += width;
      });
      y += height; h -= height;
    }
    rest = rest.slice(count);
  }
  return out;
}

export interface TreemapGroup { harness: string; rect: Rect; tiles: Tile<ModelReport>[] }

/** Harnesses first, then each harness's models inside its own rectangle. */
export function treemap(report: UsageReport, metric: UsageMetric, rect: Rect): TreemapGroup[] {
  const groups = squarify(harnessesByMetric(report, metric).map(entry => ({ item: entry.harness, value: harnessValue(entry, metric) })), rect);
  return groups.map(group => ({
    harness: group.item,
    rect: { x: group.x, y: group.y, w: group.w, h: group.h },
    tiles: squarify(modelsOf(report, group.item, metric).map(model => ({ item: model, value: modelValue(model, metric) })), group),
  }));
}

// ---------------------------------------------------------------------------
// Flow

export interface FlowNode {
  id: string;
  column: number;
  label: string;
  value: number;
  harness: string | null;
  kind: TokenKind | null;
  model: ModelReport | null;
  y: number;
  height: number;
}

export interface FlowLink {
  source: string;
  target: string;
  value: number;
  harness: string;
  sourceY: number;
  sourceHeight: number;
  targetY: number;
  targetHeight: number;
}

export interface FlowLayout { nodes: FlowNode[]; links: FlowLink[]; columns: number; height: number }

/**
 * Harness → model → token kind in token mode; harness → model in cost mode,
 * because a bucket's cost is not split by token kind. A link's height at each
 * end is its share of that node, so every node is exactly filled by its links.
 */
export function flowLayout(report: UsageReport, metric: UsageMetric, options: { height: number; padding: number; minNode: number }): FlowLayout {
  const harnessNodes: FlowNode[] = [];
  const modelNodes: FlowNode[] = [];
  const raw: { source: string; target: string; value: number; harness: string }[] = [];
  for (const entry of harnessesByMetric(report, metric)) {
    const value = harnessValue(entry, metric);
    if (value <= 0) continue;
    harnessNodes.push({ id: `h:${entry.harness}`, column: 0, label: entry.harness, value, harness: entry.harness, kind: null, model: null, y: 0, height: 0 });
    for (const model of modelsOf(report, entry.harness, metric)) {
      const modelTotal = modelValue(model, metric);
      if (modelTotal <= 0) continue;
      const id = `m:${model.harness}:${model.model}`;
      modelNodes.push({ id, column: 1, label: model.model, value: modelTotal, harness: model.harness, kind: null, model, y: 0, height: 0 });
      raw.push({ source: `h:${entry.harness}`, target: id, value: modelTotal, harness: entry.harness });
    }
  }
  const columns: FlowNode[][] = [harnessNodes, modelNodes];
  if (metric === "tokens") {
    const kindNodes: FlowNode[] = [];
    for (const kind of TOKEN_KINDS) {
      const value = modelNodes.reduce((sum, node) => sum + (node.model?.[kind.key] ?? 0), 0);
      if (value > 0) kindNodes.push({ id: `k:${kind.key}`, column: 2, label: kind.label, value, harness: null, kind: kind.key, model: null, y: 0, height: 0 });
    }
    for (const node of modelNodes) for (const kind of kindNodes) {
      const value = node.model?.[kind.kind!] ?? 0;
      if (value > 0) raw.push({ source: node.id, target: kind.id, value, harness: node.harness! });
    }
    columns.push(kindNodes);
  }

  const usable = columns.filter(column => column.length > 0);
  if (usable.length === 0) return { nodes: [], links: [], columns: columns.length, height: options.height };
  const scale = Math.min(...usable.map(column => (options.height - options.padding * (column.length - 1)) / column.reduce((sum, node) => sum + node.value, 0)));
  let height = options.height;
  for (const column of usable) {
    for (const node of column) node.height = Math.max(node.value * scale, options.minNode);
    height = Math.max(height, column.reduce((sum, node) => sum + node.height, 0) + options.padding * (column.length - 1));
  }
  for (const column of usable) {
    const extent = column.reduce((sum, node) => sum + node.height, 0) + options.padding * (column.length - 1);
    let cursor = (height - extent) / 2;
    for (const node of column) { node.y = cursor; cursor += node.height + options.padding; }
  }

  const byId = new Map(columns.flat().map(node => [node.id, node]));
  const links: FlowLink[] = raw.map(link => ({ ...link, sourceY: 0, sourceHeight: 0, targetY: 0, targetHeight: 0 }));
  // Stack each node's links in the order of the node at the other end, so bands leave and arrive without crossing inside a node.
  for (const node of byId.values()) {
    let out = node.y;
    for (const link of links.filter(entry => entry.source === node.id).sort((a, b) => byId.get(a.target)!.y - byId.get(b.target)!.y)) {
      link.sourceY = out;
      link.sourceHeight = (link.value / node.value) * node.height;
      out += link.sourceHeight;
    }
    let into = node.y;
    for (const link of links.filter(entry => entry.target === node.id).sort((a, b) => byId.get(a.source)!.y - byId.get(b.source)!.y)) {
      link.targetY = into;
      link.targetHeight = (link.value / node.value) * node.height;
      into += link.targetHeight;
    }
  }
  return { nodes: columns.flat(), links, columns: columns.length, height };
}

/** A band from one node's right edge to another's left edge, each end as tall as its share of that node. */
export function flowBandPath(link: FlowLink, x0: number, x1: number): string {
  const mid = (x0 + x1) / 2;
  const { sourceY: a, sourceHeight: ah, targetY: b, targetHeight: bh } = link;
  return `M${fixed(x0)},${fixed(a)} C${fixed(mid)},${fixed(a)} ${fixed(mid)},${fixed(b)} ${fixed(x1)},${fixed(b)} L${fixed(x1)},${fixed(b + bh)} C${fixed(mid)},${fixed(b + bh)} ${fixed(mid)},${fixed(a + ah)} ${fixed(x0)},${fixed(a + ah)} Z`;
}

// ---------------------------------------------------------------------------
// Calendar

/** Monday is 0. The day is parsed as UTC so the local zone cannot move it across midnight. */
export function mondayIndex(day: string): number {
  return (new Date(`${day.slice(0, 10)}T00:00:00Z`).getUTCDay() + 6) % 7;
}

/** Rows of seven Monday-first slots covering every day in order; slots outside the window are null. */
export function calendarWeeks(days: readonly string[]): (number | null)[][] {
  if (days.length === 0) return [];
  const lead = mondayIndex(days[0]);
  const slots: (number | null)[] = [...Array.from({ length: lead }, () => null), ...days.map((_, index) => index)];
  while (slots.length % 7 !== 0) slots.push(null);
  const weeks: (number | null)[][] = [];
  for (let start = 0; start < slots.length; start += 7) weeks.push(slots.slice(start, start + 7));
  return weeks;
}

/**
 * The calendar's own axis: the window's periods plus every bucket key that
 * fell outside them (a zone edge, which `buildUsageReport` keeps rather than
 * drops), so each counted bucket has a cell to click. Days are filled densely
 * so the Monday-first grid keeps its shape; hours are merged in order.
 */
export function calendarAxis(periods: readonly string[], keys: readonly string[], resolution: UsageResolution): string[] {
  const known = new Set(periods);
  const extra = [...new Set(keys.filter(key => !known.has(key)))];
  if (extra.length === 0) return [...periods];
  const all = [...periods, ...extra].sort();
  if (resolution === "hour") return all;
  const day = (key: string) => Date.parse(`${key.slice(0, 10)}T00:00:00Z`);
  const first = day(all[0]);
  const last = day(all[all.length - 1]);
  if (!Number.isFinite(first) || !Number.isFinite(last)) return all;
  const dense: string[] = [];
  for (let at = first; at <= last; at += 86_400_000) dense.push(new Date(at).toISOString().slice(0, 10));
  return dense;
}

/** The bucket keys a summary carries, in the form `periodKey` gives them. */
export function bucketKeys(summary: Pick<UsageSummaryResult, "buckets" | "resolution">): string[] {
  return summary.buckets.map(bucket => periodKey(bucket, summary.resolution));
}

/** An inclusive index range, whichever end was picked first. */
export function orderedRange(anchor: number, end: number): [number, number] {
  return anchor <= end ? [anchor, end] : [end, anchor];
}

/** The report for a sub-range of the window, rebuilt from the buckets in it; `null` is the whole window. */
export function scopeReport(summary: Pick<UsageSummaryResult, "buckets" | "resolution">, periods: readonly string[], range: [number, number] | null): { report: UsageReport; periods: string[] } {
  if (!range) return { report: buildUsageReport(summary, periods), periods: [...periods] };
  const [from, to] = range;
  const scoped = periods.slice(Math.max(0, from), Math.min(periods.length, to + 1));
  const keep = new Set(scoped);
  const buckets = summary.buckets.filter(bucket => keep.has(periodKey(bucket, summary.resolution)));
  return { report: buildUsageReport({ buckets, resolution: summary.resolution }, scoped), periods: scoped };
}
