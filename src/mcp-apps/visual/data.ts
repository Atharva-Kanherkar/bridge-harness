/**
 * From a chart block's Vega-Lite subset to the shapes the renderers draw:
 * ordered categories, ordered series, and one value per (category, series).
 *
 * Pure, so the rules for ordering, aggregation and estimates are tested as
 * data. Ordering departs from Vega-Lite's alphabetical default on purpose:
 * a model lists Mon..Fri or steps of a funnel in the order it means, so
 * unsorted categories keep their order of appearance.
 */

import type { ChartBlock, EncodingChannel, Row, VisualColor } from "../spec";
import { parseTemporal } from "./format";

export type FieldKind = "quantitative" | "temporal" | "ordinal" | "nominal";

export interface Cell {
  value: number;
  estimate: boolean;
  total: boolean;
}

export interface Prepared {
  /** The category axis: x for vertical charts, y for horizontal ones. */
  categoryField?: string;
  valueField?: string;
  seriesField?: string;
  categoryKind: FieldKind;
  horizontal: boolean;
  categories: string[];
  series: string[];
  cells: Map<string, Map<string, Cell>>;
  valueFormat?: string;
  categoryTitle?: string;
  valueTitle?: string;
}

export const SINGLE_SERIES = "";

const PALETTE: VisualColor[] = ["accent", "codex", "claude", "opencode", "neutral", "cursor"];

export function colorVar(color: VisualColor): string {
  switch (color) {
    case "accent": return "var(--ring)";
    case "neutral": return "var(--muted-foreground)";
    case "positive": return "var(--success)";
    case "negative": return "var(--destructive)";
    default: return `var(--chart-${color})`;
  }
}

/** Each series' colour: the block's map first, then the palette in order. */
export function seriesColors(series: string[], colors: Record<string, VisualColor> | undefined): Map<string, string> {
  const out = new Map<string, string>();
  let next = 0;
  for (const name of series) {
    const chosen = colors?.[name];
    out.set(name, colorVar(chosen ?? PALETTE[next++ % PALETTE.length]));
  }
  return out;
}

export function channel(block: ChartBlock, name: "x" | "y" | "color" | "theta" | "size"): EncodingChannel | undefined {
  const value = block.vegaLite.encoding[name];
  return value && typeof value === "object" && !Array.isArray(value) ? value : undefined;
}

export function fieldKind(block: ChartBlock, name: "x" | "y" | "color" | "theta"): FieldKind | undefined {
  const definition = channel(block, name);
  if (!definition) return undefined;
  if (definition.type) return definition.type;
  if (definition.aggregate === "count") return "quantitative";
  const field = definition.field;
  if (!field) return undefined;
  const values = block.vegaLite.data.values.map(row => row[field]).filter(value => value !== null && value !== undefined);
  if (values.length === 0) return undefined;
  if (values.every(value => typeof value === "number")) return "quantitative";
  if (values.every(value => typeof value === "string" && parseTemporal(value) !== null)) return "temporal";
  return "nominal";
}

const key = (value: unknown) => (value === null || value === undefined ? "" : String(value));

function aggregate(values: number[], how: EncodingChannel["aggregate"]): number {
  if (values.length === 0) return 0;
  switch (how) {
    case "count": return values.length;
    case "mean": case "average": return values.reduce((sum, value) => sum + value, 0) / values.length;
    case "min": return Math.min(...values);
    case "max": return Math.max(...values);
    case "median": {
      const sorted = [...values].sort((a, b) => a - b);
      const middle = Math.floor(sorted.length / 2);
      return sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
    }
    default: return values.reduce((sum, value) => sum + value, 0);
  }
}

function order(values: string[], kind: FieldKind, sort: unknown, valueOf: (category: string) => number): string[] {
  if (Array.isArray(sort)) {
    const listed = sort.map(key).filter(value => values.includes(value));
    return [...listed, ...values.filter(value => !listed.includes(value))];
  }
  if (sort === "-y" || sort === "-x" || sort === "descending") return [...values].sort((a, b) => valueOf(b) - valueOf(a));
  if (sort === "y" || sort === "x" || sort === "ascending") {
    return sort === "ascending" ? [...values].sort() : [...values].sort((a, b) => valueOf(a) - valueOf(b));
  }
  if (kind === "temporal") {
    return [...values].sort((a, b) => (parseTemporal(a)?.time ?? 0) - (parseTemporal(b)?.time ?? 0));
  }
  if (kind === "quantitative") return [...values].sort((a, b) => Number(a) - Number(b));
  return values;
}

/** Categories × series for bar-like and line-like charts. */
export function prepare(block: ChartBlock): Prepared {
  const xKind = fieldKind(block, "x");
  const yKind = fieldKind(block, "y");
  // Horizontal when the measure is on x and the categories on y.
  const horizontal = xKind === "quantitative" && yKind !== undefined && yKind !== "quantitative";
  const categoryChannel = channel(block, horizontal ? "y" : "x");
  const valueChannel = channel(block, horizontal ? "x" : "y");
  const seriesChannel = channel(block, "color");
  const categoryField = categoryChannel?.field;
  const valueField = valueChannel?.field;
  const seriesField = seriesChannel?.field && fieldKind(block, "color") !== "quantitative" ? seriesChannel.field : undefined;
  const rows: Row[] = block.vegaLite.data.values;

  const categoryOrder: string[] = [];
  const seriesOrder: string[] = [];
  const buckets = new Map<string, Map<string, { values: number[]; estimate: boolean; total: boolean }>>();
  for (const row of rows) {
    const category = key(categoryField ? row[categoryField] : "");
    const series = seriesField ? key(row[seriesField]) : SINGLE_SERIES;
    if (!categoryOrder.includes(category)) categoryOrder.push(category);
    if (!seriesOrder.includes(series)) seriesOrder.push(series);
    const raw = valueField ? row[valueField] : 1;
    const value = typeof raw === "number" ? raw : valueChannel?.aggregate === "count" ? 1 : Number(raw);
    if (!Number.isFinite(value)) continue;
    const byCategory = buckets.get(category) ?? new Map();
    const bucket = byCategory.get(series) ?? { values: [], estimate: false, total: false };
    bucket.values.push(value);
    bucket.estimate ||= row.estimate === true;
    bucket.total ||= row.total === true;
    byCategory.set(series, bucket);
    buckets.set(category, byCategory);
  }

  const cells = new Map<string, Map<string, Cell>>();
  for (const [category, byCategory] of buckets) {
    const out = new Map<string, Cell>();
    for (const [series, bucket] of byCategory) {
      out.set(series, { value: aggregate(bucket.values, valueChannel?.aggregate), estimate: bucket.estimate, total: bucket.total });
    }
    cells.set(category, out);
  }
  const categoryKind = (horizontal ? yKind : xKind) ?? "nominal";
  const sum = (category: string) => [...(cells.get(category)?.values() ?? [])].reduce((total, cell) => total + cell.value, 0);
  return {
    categoryField,
    valueField,
    seriesField,
    categoryKind,
    horizontal,
    categories: order(categoryOrder, categoryKind, categoryChannel?.sort, sum),
    series: order(seriesOrder, "nominal", seriesChannel?.sort, () => 0),
    cells,
    valueFormat: valueChannel?.axis?.format ?? undefined,
    categoryTitle: categoryChannel?.title ?? categoryField,
    valueTitle: valueChannel?.title ?? valueField,
  };
}

export function cellOf(prepared: Prepared, category: string, series: string): Cell | undefined {
  return prepared.cells.get(category)?.get(series);
}

/** Nice round ticks spanning [min, max], d3-style. */
export function niceTicks(min: number, max: number, count = 4): number[] {
  if (min === max) {
    if (min === 0) return [0, 1];
    return min > 0 ? [0, min] : [min, 0];
  }
  const span = max - min;
  const rough = span / count;
  const power = 10 ** Math.floor(Math.log10(rough));
  const step = [1, 2, 2.5, 5, 10].map(factor => factor * power).find(candidate => span / candidate <= count) ?? 10 * power;
  const start = Math.floor(min / step) * step;
  const end = Math.ceil(max / step) * step;
  const ticks: number[] = [];
  for (let value = start; value <= end + step / 2; value += step) ticks.push(Number(value.toPrecision(12)));
  return ticks;
}

export function linear(domain: [number, number], range: [number, number]) {
  const [d0, d1] = domain;
  const [r0, r1] = range;
  const span = d1 - d0 || 1;
  return (value: number) => r0 + ((value - d0) / span) * (r1 - r0);
}
