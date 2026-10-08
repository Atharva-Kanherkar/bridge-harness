/**
 * Number and time formatting for visuals.
 *
 * Supports the part of d3-format a model actually writes in a Vega-Lite
 * `axis.format` (`$`, `,`, `.Nf`, `%`, `s`, `d`, `~`), and an automatic format
 * that keeps axis ticks short.
 */

const FORMAT = /^(\$)?(,)?(?:\.(\d+))?(~)?([fdse%])?$/;

function trimZeros(text: string): string {
  return text.includes(".") ? text.replace(/\.?0+$/, "") : text;
}

function grouped(value: number, digits: number, group: boolean): string {
  return value.toLocaleString("en-US", { minimumFractionDigits: digits, maximumFractionDigits: digits, useGrouping: group });
}

/** Short SI-style form: 1.2k, 3.4M, 5.6B. */
export function compact(value: number, significant = 3): string {
  const abs = Math.abs(value);
  const units: [number, string][] = [[1e12, "T"], [1e9, "B"], [1e6, "M"], [1e3, "k"]];
  for (const [size, unit] of units) {
    if (abs >= size) return `${trimZeros((value / size).toPrecision(significant))}${unit}`;
  }
  if (abs === 0) return "0";
  if (abs < 1) return trimZeros(value.toPrecision(Math.min(significant, 3)));
  return trimZeros(value.toPrecision(significant));
}

/** Format with a d3-format specifier, or automatically when there is none. */
export function formatNumber(value: number, specifier?: string | null, options: { compactAuto?: boolean } = {}): string {
  if (!Number.isFinite(value)) return "";
  const match = specifier ? FORMAT.exec(specifier.trim()) : null;
  if (!match) return formatAuto(value, options.compactAuto ?? false);
  const [, currency, group, precisionText, trim, type] = match;
  const precision = precisionText === undefined ? undefined : Number(precisionText);
  let body: string;
  switch (type) {
    case "%":
      body = `${grouped(value * 100, precision ?? 0, !!group)}%`;
      break;
    case "s":
      body = compact(value, precision ?? 3);
      break;
    case "d":
      body = grouped(Math.round(value), 0, !!group);
      break;
    case "e":
      body = value.toExponential(precision ?? 2);
      break;
    case "f":
      body = grouped(value, precision ?? 6, !!group);
      break;
    default:
      body = precision === undefined ? formatAuto(value, false, !!group) : grouped(value, precision, !!group);
  }
  if (trim) body = body.endsWith("%") ? `${trimZeros(body.slice(0, -1))}%` : trimZeros(body);
  if (currency) body = value < 0 ? `−$${body.replace(/^-/, "")}` : `$${body}`;
  return body.replace(/^-/, "−");
}

/** Integers grouped, decimals to two places at most, big numbers compact on request. */
export function formatAuto(value: number, compactLarge: boolean, group = true): string {
  if (compactLarge && Math.abs(value) >= 10_000) return compact(value).replace(/^-/, "−");
  const digits = Number.isInteger(value) ? 0 : Math.abs(value) < 1 ? 3 : 2;
  return trimZeros(grouped(value, digits, group)).replace(/^-/, "−");
}

/** A table cell in one of the table column formats. */
export function formatCell(value: unknown, format?: string): string {
  if (value === null || value === undefined) return "–";
  if (typeof value === "boolean") return value ? "Yes" : "No";
  if (typeof value !== "number") return String(value);
  switch (format) {
    case "percent":
      return `${formatAuto(Math.abs(value) <= 1 ? value * 100 : value, false)}%`;
    case "currency":
      return formatNumber(value, "$,.2f");
    case "compact":
      return compact(value).replace(/^-/, "−");
    default:
      return formatAuto(value, false);
  }
}

export type TimeGrain = "year" | "month" | "day" | "time";

const TEMPORAL = /^(\d{4})(?:-(\d{2})(?:-(\d{2})(?:[T ](\d{2}):(\d{2}))?)?)?/;

/** Parse the date forms a model writes (2025, 2025-09, 2025-09-30, ISO) as UTC. */
export function parseTemporal(value: unknown): { time: number; grain: TimeGrain } | null {
  if (typeof value === "number" && Number.isInteger(value) && value >= 1000 && value <= 9999) {
    return { time: Date.UTC(value, 0, 1), grain: "year" };
  }
  if (typeof value !== "string") return null;
  const match = TEMPORAL.exec(value.trim());
  if (!match) return null;
  const [, year, month, day, hour, minute] = match;
  const grain: TimeGrain = hour !== undefined ? "time" : day !== undefined ? "day" : month !== undefined ? "month" : "year";
  return {
    time: Date.UTC(Number(year), month ? Number(month) - 1 : 0, day ? Number(day) : 1, hour ? Number(hour) : 0, minute ? Number(minute) : 0),
    grain,
  };
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

export function formatTime(time: number, grain: TimeGrain): string {
  const date = new Date(time);
  const month = MONTHS[date.getUTCMonth()];
  switch (grain) {
    case "year":
      return String(date.getUTCFullYear());
    case "month":
      return `${month} ${date.getUTCFullYear()}`;
    case "day":
      return `${month} ${date.getUTCDate()}`;
    case "time":
      return `${month} ${date.getUTCDate()}, ${String(date.getUTCHours()).padStart(2, "0")}:${String(date.getUTCMinutes()).padStart(2, "0")}`;
  }
}

/** A category label as it should read: dates formatted, everything else as given. */
export function formatCategory(value: string, temporal: boolean): string {
  if (!temporal) return value;
  const parsed = parseTemporal(value);
  return parsed ? formatTime(parsed.time, parsed.grain) : value;
}
