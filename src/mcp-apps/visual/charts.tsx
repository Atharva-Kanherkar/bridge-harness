/**
 * The chart family, drawn natively in Bridge's chart vocabulary: hairline
 * grids, rounded marks with breathing room between them, tabular numbers, a
 * single highlight on hover, and estimates drawn dashed or hollow so they
 * never read as facts.
 */

import { useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { ChartBlock } from "../spec";
import { cellOf, channel, linear, niceTicks, prepare, seriesColors, SINGLE_SERIES, type Prepared } from "./data";
import { formatCategory, formatNumber, parseTemporal } from "./format";

const AXIS = 11;
const CHAR = 6.4;

function useWidth(fallback = 640) {
  const ref = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(fallback);
  useLayoutEffect(() => {
    const node = ref.current;
    if (!node) return;
    const measure = () => setWidth(Math.max(240, Math.round(node.getBoundingClientRect().width)));
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    return () => observer.disconnect();
  }, []);
  return [ref, width] as const;
}

interface TipRow {
  label: string;
  value: string;
  color?: string;
  estimate?: boolean;
}

interface Tip {
  x: number;
  y: number;
  title: string;
  rows: TipRow[];
  total?: string;
}

function Tooltip({ tip, width }: { tip: Tip | null; width: number }) {
  if (!tip) return null;
  const left = Math.min(Math.max(tip.x + 14, 0), width - 200);
  return (
    <div
      className="u-overlay pointer-events-none absolute z-10 min-w-[160px] rounded-xl px-3 py-2 text-[12.5px]"
      style={{ left, top: Math.max(tip.y - 12, 0) }}
      role="status"
    >
      <div className="mb-1 font-medium text-foreground">{tip.title}</div>
      {tip.rows.map(row => (
        <div key={row.label} className="flex items-center justify-between gap-4 leading-6 text-muted-foreground">
          <span className="flex min-w-0 items-center gap-2">
            {row.color && <svg width="8" height="8" aria-hidden="true"><circle cx="4" cy="4" r="4" fill={row.color} /></svg>}
            <span className="truncate">{row.label}</span>
          </span>
          <span className="font-medium tabular-nums text-foreground">{row.value}{row.estimate ? <span className="ml-1 font-normal text-muted-foreground">est.</span> : null}</span>
        </div>
      ))}
      {tip.total && (
        <div className="mt-1 flex justify-between gap-4 border-t border-border pt-1 leading-6 text-muted-foreground">
          <span>Total</span>
          <span className="font-medium tabular-nums text-foreground">{tip.total}</span>
        </div>
      )}
    </div>
  );
}

function Legend({ prepared, colors, total }: { prepared: Prepared; colors: Map<string, string>; total?: (series: string) => string }) {
  if (prepared.series.length < 2 || prepared.series[0] === SINGLE_SERIES) return null;
  return (
    <div className="mb-3 flex flex-wrap gap-x-5 gap-y-1 text-[12.5px] text-muted-foreground">
      {prepared.series.map(series => (
        <span key={series} className="flex items-center gap-2">
          <svg width="8" height="8" aria-hidden="true"><circle cx="4" cy="4" r="4" fill={colors.get(series)} /></svg>
          {series}
          {total && <b className="font-medium tabular-nums text-foreground">{total(series)}</b>}
        </span>
      ))}
    </div>
  );
}

function ChartFrame({ label, height, children, tip, width, frameRef }: { label: string; height: number; children: ReactNode; tip: Tip | null; width: number; frameRef: React.RefObject<HTMLDivElement> }) {
  return (
    <div ref={frameRef} className="relative w-full">
      <svg width={width} height={height} viewBox={`0 0 ${width} ${height}`} role="img" aria-label={label} className="block overflow-visible font-sans">
        {children}
      </svg>
      <Tooltip tip={tip} width={width} />
    </div>
  );
}

const fmt = (prepared: Prepared, value: number, axis = false) => formatNumber(value, prepared.valueFormat, { compactAuto: axis });

function thin(labels: string[], room: number): Set<number> {
  const widest = Math.max(1, ...labels.map(label => label.length)) * CHAR + 14;
  const every = Math.max(1, Math.ceil((widest * labels.length) / Math.max(room, 1)));
  const shown = new Set<number>();
  for (let at = 0; at < labels.length; at += every) shown.add(at);
  shown.add(labels.length - 1);
  if (labels.length > 1 && every > 1 && shown.has(labels.length - 1) && (labels.length - 1) % every !== 0 && (labels.length - 1) % every < every / 2) {
    shown.delete(labels.length - 1 - ((labels.length - 1) % every));
  }
  return shown;
}

function categoryLabel(prepared: Prepared, category: string) {
  return formatCategory(category, prepared.categoryKind === "temporal");
}

/** Vertical bars: bar, stacked-bar, grouped-bar, and waterfall. */
function VerticalBars({ block, prepared }: { block: ChartBlock; prepared: Prepared }) {
  const [ref, width] = useWidth();
  const [hover, setHover] = useState<number | null>(null);
  const colors = useMemo(() => seriesColors(prepared.series, block.colors), [prepared, block.colors]);
  const grouped = block.form === "grouped-bar";
  const waterfall = block.form === "waterfall";
  const height = 240;
  const pad = { top: 12, right: 8, bottom: 26, left: 48 };
  const innerW = width - pad.left - pad.right;
  const innerH = height - pad.top - pad.bottom;

  // Waterfall: each step floats from the running total; totals stand on zero.
  const steps = useMemo(() => {
    if (!waterfall) return [];
    let running = 0;
    return prepared.categories.map(category => {
      const cell = cellOf(prepared, category, prepared.series[0]);
      const value = cell?.value ?? 0;
      if (cell?.total) {
        running = value;
        return { from: 0, to: value, kind: "total" as const, cell };
      }
      const from = running;
      running += value;
      return { from, to: running, kind: value >= 0 ? ("up" as const) : ("down" as const), cell };
    });
  }, [prepared, waterfall]);

  const extent = useMemo(() => {
    let low = 0;
    let high = 0;
    if (waterfall) {
      for (const step of steps) {
        low = Math.min(low, step.from, step.to);
        high = Math.max(high, step.from, step.to);
      }
      return [low, high] as [number, number];
    }
    for (const category of prepared.categories) {
      let up = 0;
      let down = 0;
      for (const series of prepared.series) {
        const value = cellOf(prepared, category, series)?.value ?? 0;
        if (grouped) {
          high = Math.max(high, value);
          low = Math.min(low, value);
        } else if (value >= 0) up += value;
        else down += value;
      }
      if (!grouped) {
        high = Math.max(high, up);
        low = Math.min(low, down);
      }
    }
    return [low, high] as [number, number];
  }, [prepared, grouped, waterfall, steps]);

  const ticks = niceTicks(extent[0], extent[1]);
  const y = linear([ticks[0], ticks[ticks.length - 1]], [pad.top + innerH, pad.top]);
  const band = innerW / Math.max(1, prepared.categories.length);
  const barW = Math.min(band * 0.64, 56);
  const labels = prepared.categories.map(category => categoryLabel(prepared, category));
  const shown = thin(labels, innerW);
  const dim = (at: number) => (hover === null || hover === at ? 1 : 0.38);

  const tipFor = (at: number): Tip => {
    const category = prepared.categories[at];
    const rows = prepared.series
      .map(series => ({ series, cell: cellOf(prepared, category, series) }))
      .filter(entry => entry.cell)
      .map(({ series, cell }) => ({ label: series === SINGLE_SERIES ? (prepared.valueTitle ?? "Value") : series, value: fmt(prepared, cell!.value), color: colors.get(series), estimate: cell!.estimate }));
    const total = prepared.series.length > 1 && !grouped ? fmt(prepared, prepared.series.reduce((sum, series) => sum + (cellOf(prepared, category, series)?.value ?? 0), 0)) : undefined;
    return { x: pad.left + band * at + band / 2, y: pad.top, title: labels[at], rows, total };
  };

  const marks: ReactNode[] = [];
  prepared.categories.forEach((category, at) => {
    const center = pad.left + band * at + band / 2;
    if (waterfall) {
      const step = steps[at];
      const top = y(Math.max(step.from, step.to));
      const bottom = y(Math.min(step.from, step.to));
      const fill = step.kind === "total" ? "var(--muted-foreground)" : step.kind === "up" ? "var(--success)" : "var(--destructive)";
      marks.push(
        <g key={category} opacity={dim(at)}>
          <rect x={center - barW / 2} y={top} width={barW} height={Math.max(2, bottom - top)} rx={4} fill={fill} opacity={step.kind === "total" ? 0.55 : 0.85} strokeDasharray={step.cell?.estimate ? "4 3" : undefined} stroke={step.cell?.estimate ? fill : undefined} />
          {at < steps.length - 1 && <line x1={center + barW / 2} x2={center + band - barW / 2} y1={y(step.to)} y2={y(step.to)} stroke="var(--muted-foreground)" strokeDasharray="2 3" opacity={0.6} />}
        </g>,
      );
      return;
    }
    let up = 0;
    let down = 0;
    prepared.series.forEach((series, seriesAt) => {
      const cell = cellOf(prepared, category, series);
      if (!cell) return;
      const color = colors.get(series)!;
      let x0 = center - barW / 2;
      let w = barW;
      let from: number;
      let to: number;
      if (grouped) {
        w = barW / prepared.series.length;
        x0 = center - barW / 2 + w * seriesAt;
        from = 0;
        to = cell.value;
      } else if (cell.value >= 0) {
        from = up;
        to = up + cell.value;
        up = to;
      } else {
        from = down;
        to = down + cell.value;
        down = to;
      }
      const top = y(Math.max(from, to));
      const gap = !grouped && seriesAt > 0 ? 2 : 0;
      const h = Math.max(2, y(Math.min(from, to)) - top - gap);
      marks.push(
        <rect
          key={`${category}:${series}`}
          x={x0 + (grouped ? 1 : 0)}
          y={top}
          width={Math.max(1, w - (grouped ? 2 : 0))}
          height={h}
          rx={grouped ? 3 : 4}
          fill={color}
          fillOpacity={cell.estimate ? 0.35 : 0.92}
          stroke={cell.estimate ? color : undefined}
          strokeDasharray={cell.estimate ? "4 3" : undefined}
          opacity={dim(at)}
        />,
      );
    });
  });

  const total = (series: string) => fmt(prepared, prepared.categories.reduce((sum, category) => sum + (cellOf(prepared, category, series)?.value ?? 0), 0));
  return (
    <>
      <Legend prepared={prepared} colors={colors} total={grouped ? undefined : total} />
      <ChartFrame label={`${block.form} of ${prepared.valueTitle ?? "values"} by ${prepared.categoryTitle ?? "category"}`} height={height} tip={hover === null ? null : tipFor(hover)} width={width} frameRef={ref}>
        {ticks.map(tick => (
          <g key={tick}>
            <line x1={pad.left} x2={width - pad.right} y1={y(tick)} y2={y(tick)} stroke="var(--border)" />
            <text x={pad.left - 10} y={y(tick) + 4} textAnchor="end" fontSize={AXIS} fill="var(--muted-foreground)" className="tabular-nums">{fmt(prepared, tick, true)}</text>
          </g>
        ))}
        {marks}
        {labels.map((label, at) => shown.has(at) && (
          <text key={at} x={pad.left + band * at + band / 2} y={height - 6} textAnchor="middle" fontSize={AXIS} fill={hover === at ? "var(--foreground)" : "var(--muted-foreground)"} fontWeight={hover === at ? 600 : 400}>{label}</text>
        ))}
        {prepared.categories.map((category, at) => (
          <rect key={`hit:${category}`} x={pad.left + band * at} y={pad.top} width={band} height={innerH} fill="transparent" onMouseEnter={() => setHover(at)} onMouseLeave={() => setHover(null)} />
        ))}
      </ChartFrame>
    </>
  );
}

/** Horizontal bars: a ranking with the label on the left and the value at the end. */
function HorizontalBars({ block, prepared }: { block: ChartBlock; prepared: Prepared }) {
  const [ref, width] = useWidth();
  const [hover, setHover] = useState<number | null>(null);
  const colors = useMemo(() => seriesColors(prepared.series, block.colors), [prepared, block.colors]);
  const labels = prepared.categories.map(category => categoryLabel(prepared, category));
  const labelW = Math.min(180, Math.max(...labels.map(label => label.length)) * CHAR + 12);
  const valueW = 64;
  const row = 30;
  const height = prepared.categories.length * row + 8;
  const totals = prepared.categories.map(category => prepared.series.reduce((sum, series) => sum + Math.max(0, cellOf(prepared, category, series)?.value ?? 0), 0));
  const max = Math.max(...totals, 0) || 1;
  const x = linear([0, max], [labelW, width - valueW]);
  return (
    <>
      <Legend prepared={prepared} colors={colors} />
      <ChartFrame label={`${block.form} ranking ${prepared.valueTitle ?? "values"}`} height={height} width={width} frameRef={ref} tip={null}>
        {prepared.categories.map((category, at) => {
          let start = 0;
          const top = at * row + 7;
          return (
            <g key={category} opacity={hover === null || hover === at ? 1 : 0.4} onMouseEnter={() => setHover(at)} onMouseLeave={() => setHover(null)}>
              <rect x={0} y={at * row} width={width} height={row} fill="transparent" />
              <text x={labelW - 10} y={top + 12} textAnchor="end" fontSize={12} fill="var(--foreground)">{labels[at].length > 26 ? `${labels[at].slice(0, 25)}…` : labels[at]}<title>{labels[at]}</title></text>
              {prepared.series.map((series, seriesAt) => {
                const cell = cellOf(prepared, category, series);
                if (!cell || cell.value <= 0) return null;
                const x0 = x(start) + (seriesAt > 0 ? 2 : 0);
                start += cell.value;
                const color = colors.get(series)!;
                return <rect key={series} x={x0} y={top} width={Math.max(2, x(start) - x0)} height={16} rx={4} fill={color} fillOpacity={cell.estimate ? 0.35 : 0.92} stroke={cell.estimate ? color : undefined} strokeDasharray={cell.estimate ? "4 3" : undefined} />;
              })}
              <text x={x(totals[at]) + 8} y={top + 12} fontSize={12} fill="var(--foreground)" className="tabular-nums">{fmt(prepared, totals[at])}</text>
            </g>
          );
        })}
      </ChartFrame>
    </>
  );
}

/** Line and area. Temporal and numeric axes keep real spacing; categories sit evenly. */
function Lines({ block, prepared }: { block: ChartBlock; prepared: Prepared }) {
  const [ref, width] = useWidth();
  const [hover, setHover] = useState<number | null>(null);
  const colors = useMemo(() => seriesColors(prepared.series, block.colors), [prepared, block.colors]);
  const area = block.form === "area";
  const stacked = area && prepared.series.length > 1;
  const height = 240;
  const pad = { top: 16, right: 16, bottom: 26, left: 48 };
  const innerW = width - pad.left - pad.right;

  const positions = useMemo(() => {
    const kind = prepared.categoryKind;
    const numeric = prepared.categories.map(category => kind === "temporal" ? parseTemporal(category)?.time ?? NaN : kind === "quantitative" ? Number(category) : NaN);
    if (numeric.every(Number.isFinite) && prepared.categories.length > 1) {
      const scale = linear([Math.min(...numeric), Math.max(...numeric)], [pad.left, pad.left + innerW]);
      return numeric.map(scale);
    }
    const step = innerW / Math.max(1, prepared.categories.length - 1);
    return prepared.categories.map((_, at) => (prepared.categories.length === 1 ? pad.left + innerW / 2 : pad.left + step * at));
  }, [prepared, innerW, pad.left]);

  const stackBase = (category: string, seriesAt: number) => stacked ? prepared.series.slice(0, seriesAt).reduce((sum, series) => sum + (cellOf(prepared, category, series)?.value ?? 0), 0) : 0;
  const values = prepared.categories.flatMap(category => prepared.series.map((series, seriesAt) => (cellOf(prepared, category, series)?.value ?? 0) + stackBase(category, seriesAt)));
  const min = Math.min(0, ...values);
  const max = Math.max(...values, 0);
  const ticks = niceTicks(area ? min : Math.min(min, Math.min(...values)), max);
  const y = linear([ticks[0], ticks[ticks.length - 1]], [height - pad.bottom, pad.top]);
  const labels = prepared.categories.map(category => categoryLabel(prepared, category));
  const shown = thin(labels, innerW);

  const paths = prepared.series.map((series, seriesAt) => {
    const points = prepared.categories
      .map((category, at) => ({ at, cell: cellOf(prepared, category, series), base: stackBase(category, seriesAt) }))
      .filter(point => point.cell);
    const color = colors.get(series)!;
    const solid = points.filter(point => !point.cell!.estimate);
    const line = (list: typeof points) => list.map((point, index) => `${index ? "L" : "M"}${positions[point.at]} ${y(point.cell!.value + point.base)}`).join(" ");
    const firstEstimate = points.findIndex(point => point.cell!.estimate);
    const dashed = firstEstimate > 0 ? points.slice(firstEstimate - 1) : firstEstimate === 0 ? points : [];
    const fill = area && solid.length > 1
      ? `${line(points)} L${positions[points[points.length - 1].at]} ${y(points[points.length - 1].base)} ${[...points].reverse().map(point => `L${positions[point.at]} ${y(point.base)}`).join(" ")} Z`
      : null;
    return (
      <g key={series}>
        {fill && <path d={fill} fill={color} fillOpacity={stacked ? 0.28 : 0.14} />}
        <path d={line(firstEstimate >= 0 ? points.slice(0, Math.max(firstEstimate, 0)) : points)} fill="none" stroke={color} strokeWidth={2.2} strokeLinejoin="round" strokeLinecap="round" />
        {dashed.length > 1 && <path d={line(dashed)} fill="none" stroke={color} strokeWidth={2.2} strokeDasharray="5 4" />}
        {points.length <= 40 && points.map(point => (
          <circle key={point.at} cx={positions[point.at]} cy={y(point.cell!.value + point.base)} r={hover === point.at ? 5 : 3.2} fill={point.cell!.estimate ? "var(--card)" : color} stroke={color} strokeWidth={1.8} />
        ))}
        {points.filter(point => point.cell!.estimate).slice(-1).map(point => (
          <text key="est" x={positions[point.at]} y={y(point.cell!.value + point.base) - 10} textAnchor="end" fontSize={AXIS} fill="var(--muted-foreground)">est.</text>
        ))}
      </g>
    );
  });

  const tip: Tip | null = hover === null ? null : {
    x: positions[hover],
    y: pad.top,
    title: labels[hover],
    rows: prepared.series
      .map(series => ({ series, cell: cellOf(prepared, prepared.categories[hover], series) }))
      .filter(entry => entry.cell)
      .map(({ series, cell }) => ({ label: series === SINGLE_SERIES ? (prepared.valueTitle ?? "Value") : series, value: fmt(prepared, cell!.value), color: colors.get(series), estimate: cell!.estimate })),
  };
  const nearest = (offset: number) => positions.reduce((best, position, at) => Math.abs(position - offset) < Math.abs(positions[best] - offset) ? at : best, 0);

  return (
    <>
      <Legend prepared={prepared} colors={colors} />
      <ChartFrame label={`${block.form} of ${prepared.valueTitle ?? "values"} over ${prepared.categoryTitle ?? "time"}`} height={height} tip={tip} width={width} frameRef={ref}>
        {ticks.map(tick => (
          <g key={tick}>
            <line x1={pad.left} x2={width - pad.right} y1={y(tick)} y2={y(tick)} stroke="var(--border)" />
            <text x={pad.left - 10} y={y(tick) + 4} textAnchor="end" fontSize={AXIS} fill="var(--muted-foreground)" className="tabular-nums">{fmt(prepared, tick, true)}</text>
          </g>
        ))}
        {hover !== null && <line x1={positions[hover]} x2={positions[hover]} y1={pad.top} y2={height - pad.bottom} stroke="var(--foreground)" strokeDasharray="2 3" opacity={0.4} />}
        {paths}
        {labels.map((label, at) => shown.has(at) && (
          <text key={at} x={positions[at]} y={height - 6} textAnchor={at === 0 && positions[at] - pad.left < 20 ? "start" : at === labels.length - 1 && width - positions[at] < 30 ? "end" : "middle"} fontSize={AXIS} fill="var(--muted-foreground)">{label}</text>
        ))}
        <rect x={pad.left} y={pad.top} width={innerW} height={height - pad.top - pad.bottom} fill="transparent" onMouseMove={event => {
          const box = (event.currentTarget.ownerSVGElement ?? event.currentTarget).getBoundingClientRect();
          setHover(nearest(event.clientX - box.left));
        }} onMouseLeave={() => setHover(null)} />
      </ChartFrame>
    </>
  );
}

function Scatter({ block }: { block: ChartBlock }) {
  const [ref, width] = useWidth();
  const [hover, setHover] = useState<number | null>(null);
  const xField = channel(block, "x")?.field ?? "";
  const yField = channel(block, "y")?.field ?? "";
  const seriesField = channel(block, "color")?.field;
  const rows = block.vegaLite.data.values.filter(row => typeof row[xField] === "number" && typeof row[yField] === "number");
  const series = seriesField ? [...new Set(rows.map(row => String(row[seriesField])))] : [SINGLE_SERIES];
  const colors = seriesColors(series, block.colors);
  const height = 260;
  const pad = { top: 12, right: 16, bottom: 34, left: 48 };
  const xs = rows.map(row => row[xField] as number);
  const ys = rows.map(row => row[yField] as number);
  const xTicks = niceTicks(Math.min(...xs), Math.max(...xs));
  const yTicks = niceTicks(Math.min(...ys), Math.max(...ys));
  const x = linear([xTicks[0], xTicks[xTicks.length - 1]], [pad.left, width - pad.right]);
  const y = linear([yTicks[0], yTicks[yTicks.length - 1]], [height - pad.bottom, pad.top]);
  const xFormat = channel(block, "x")?.axis?.format;
  const yFormat = channel(block, "y")?.axis?.format;
  const hovered = hover === null ? null : rows[hover];
  const tip: Tip | null = hovered ? {
    x: x(hovered[xField] as number),
    y: y(hovered[yField] as number) - 30,
    title: seriesField ? String(hovered[seriesField]) : "Point",
    rows: [
      { label: channel(block, "x")?.title ?? xField, value: formatNumber(hovered[xField] as number, xFormat) },
      { label: channel(block, "y")?.title ?? yField, value: formatNumber(hovered[yField] as number, yFormat), estimate: hovered.estimate === true },
    ],
  } : null;
  const prepared = { series, seriesField } as unknown as Prepared;
  return (
    <>
      <Legend prepared={prepared} colors={colors} />
      <ChartFrame label={`scatter of ${yField} against ${xField}`} height={height} tip={tip} width={width} frameRef={ref}>
        {yTicks.map(tick => (
          <g key={`y${tick}`}>
            <line x1={pad.left} x2={width - pad.right} y1={y(tick)} y2={y(tick)} stroke="var(--border)" />
            <text x={pad.left - 10} y={y(tick) + 4} textAnchor="end" fontSize={AXIS} fill="var(--muted-foreground)" className="tabular-nums">{formatNumber(tick, yFormat, { compactAuto: true })}</text>
          </g>
        ))}
        {xTicks.map(tick => <text key={`x${tick}`} x={x(tick)} y={height - 18} textAnchor="middle" fontSize={AXIS} fill="var(--muted-foreground)" className="tabular-nums">{formatNumber(tick, xFormat, { compactAuto: true })}</text>)}
        <text x={width - pad.right} y={height - 2} textAnchor="end" fontSize={AXIS} fill="var(--muted-foreground)">{channel(block, "x")?.title ?? xField}</text>
        {rows.map((row, at) => {
          const color = colors.get(seriesField ? String(row[seriesField]) : SINGLE_SERIES)!;
          const estimate = row.estimate === true;
          return <circle key={at} cx={x(row[xField] as number)} cy={y(row[yField] as number)} r={hover === at ? 6 : 4.2} fill={estimate ? "var(--card)" : color} fillOpacity={estimate ? 1 : 0.85} stroke={color} strokeWidth={estimate ? 1.6 : 0} opacity={hover === null || hover === at ? 1 : 0.45} onMouseEnter={() => setHover(at)} onMouseLeave={() => setHover(null)} />;
        })}
      </ChartFrame>
    </>
  );
}

function Heatmap({ block }: { block: ChartBlock }) {
  const [ref, width] = useWidth();
  const [hover, setHover] = useState<string | null>(null);
  const xField = channel(block, "x")?.field ?? "";
  const yField = channel(block, "y")?.field ?? "";
  const valueField = channel(block, "color")?.field ?? "";
  const rows = block.vegaLite.data.values;
  const columns = [...new Set(rows.map(row => String(row[xField])))];
  const lines = [...new Set(rows.map(row => String(row[yField])))];
  const values = new Map(rows.map(row => [`${row[xField]}|${row[yField]}`, row]));
  const numbers = rows.map(row => row[valueField]).filter((value): value is number => typeof value === "number");
  const low = Math.min(...numbers, 0);
  const high = Math.max(...numbers, 1);
  const labelW = Math.min(120, Math.max(...lines.map(line => line.length)) * CHAR + 12);
  const gap = 4;
  // Cells fill the width; their height stops at a comfortable square-ish size.
  const cellW = Math.max(14, (width - labelW) / columns.length - gap);
  const cell = Math.max(14, Math.min(36, cellW));
  const height = lines.length * (cell + gap) + 24;
  const shown = thin(columns, columns.length * (cellW + gap));
  const format = channel(block, "color")?.axis?.format;
  const hoveredRow = hover ? values.get(hover) : undefined;
  const [hx, hy] = hover ? hover.split("|") : ["", ""];
  const tip: Tip | null = hoveredRow ? {
    x: labelW + columns.indexOf(hx) * (cellW + gap) + cellW,
    y: lines.indexOf(hy) * (cell + gap),
    title: `${hy} · ${hx}`,
    rows: [{ label: channel(block, "color")?.title ?? valueField, value: formatNumber(Number(hoveredRow[valueField]), format), estimate: hoveredRow.estimate === true }],
  } : null;
  return (
    <ChartFrame label={`heatmap of ${valueField} by ${xField} and ${yField}`} height={height} tip={tip} width={width} frameRef={ref}>
      {lines.map((line, row) => (
        <text key={line} x={labelW - 10} y={row * (cell + gap) + cell / 2 + 4} textAnchor="end" fontSize={AXIS} fill="var(--muted-foreground)">{line}</text>
      ))}
      {lines.flatMap((line, row) => columns.map((column, col) => {
        const entry = values.get(`${column}|${line}`);
        const value = typeof entry?.[valueField] === "number" ? (entry[valueField] as number) : null;
        const strength = value === null ? 0 : 0.08 + 0.92 * ((value - low) / (high - low || 1));
        return (
          <rect key={`${line}|${column}`} x={labelW + col * (cellW + gap)} y={row * (cell + gap)} width={cellW} height={cell} rx={Math.min(6, cell / 4)}
            fill={value === null ? "var(--muted)" : "var(--ring)"} fillOpacity={value === null ? 1 : strength}
            stroke={entry?.estimate === true ? "var(--ring)" : hover === `${column}|${line}` ? "var(--foreground)" : undefined} strokeDasharray={entry?.estimate === true ? "3 2" : undefined}
            onMouseEnter={() => setHover(`${column}|${line}`)} onMouseLeave={() => setHover(null)} />
        );
      }))}
      {columns.map((column, col) => shown.has(col) && (
        <text key={column} x={labelW + col * (cellW + gap) + cellW / 2} y={height - 6} textAnchor="middle" fontSize={AXIS} fill="var(--muted-foreground)">{column}</text>
      ))}
    </ChartFrame>
  );
}

function Proportion({ block }: { block: ChartBlock }) {
  const [ref, width] = useWidth();
  const [hover, setHover] = useState<string | null>(null);
  const partField = channel(block, "color")?.field ?? "";
  const sizeField = (channel(block, "x") ?? channel(block, "theta"))?.field ?? "";
  const totals = new Map<string, { value: number; estimate: boolean }>();
  for (const row of block.vegaLite.data.values) {
    const part = String(row[partField]);
    const value = typeof row[sizeField] === "number" ? (row[sizeField] as number) : 0;
    const prior = totals.get(part) ?? { value: 0, estimate: false };
    totals.set(part, { value: prior.value + Math.max(0, value), estimate: prior.estimate || row.estimate === true });
  }
  const parts = [...totals.entries()].sort((a, b) => b[1].value - a[1].value);
  const sum = parts.reduce((total, [, entry]) => total + entry.value, 0) || 1;
  const colors = seriesColors(parts.map(([part]) => part), block.colors);
  const format = (channel(block, "x") ?? channel(block, "theta"))?.axis?.format;
  const gap = 3;
  const usable = width - gap * (parts.length - 1);
  let cursor = 0;
  return (
    <div ref={ref} className="w-full">
      <svg width={width} height={14} viewBox={`0 0 ${width} 14`} role="img" aria-label={`proportion of ${sizeField} by ${partField}`} className="block">
        {parts.map(([part, entry]) => {
          const w = Math.max(3, (entry.value / sum) * usable);
          const x = cursor;
          cursor += w + gap;
          const color = colors.get(part)!;
          return <rect key={part} x={x} y={0} width={w} height={14} rx={5} fill={color} fillOpacity={entry.estimate ? 0.35 : 0.95} stroke={entry.estimate ? color : undefined} strokeDasharray={entry.estimate ? "4 3" : undefined} opacity={hover === null || hover === part ? 1 : 0.4} onMouseEnter={() => setHover(part)} onMouseLeave={() => setHover(null)} />;
        })}
      </svg>
      <div className="mt-4 grid grid-cols-[repeat(auto-fill,minmax(140px,1fr))] gap-x-6 gap-y-3">
        {parts.map(([part, entry]) => (
          <div key={part} className="min-w-0" onMouseEnter={() => setHover(part)} onMouseLeave={() => setHover(null)}>
            <div className="flex items-center gap-2 text-[12.5px] text-muted-foreground">
              <svg width="8" height="8" aria-hidden="true"><circle cx="4" cy="4" r="4" fill={colors.get(part)} /></svg>
              <span className="truncate">{part}</span>
            </div>
            <div className="mt-1 text-[22px] font-light tracking-[-0.02em] text-foreground tabular-nums">{formatNumber(entry.value, format)}{entry.estimate && <span className="ml-1 text-[12px] text-muted-foreground">est.</span>}</div>
            <div className="text-[12px] text-muted-foreground tabular-nums">{((entry.value / sum) * 100).toFixed(1)}%</div>
          </div>
        ))}
      </div>
    </div>
  );
}

function Funnel({ block, prepared }: { block: ChartBlock; prepared: Prepared }) {
  const [ref, width] = useWidth();
  const steps = prepared.categories.map(category => ({ category, cell: cellOf(prepared, category, prepared.series[0]) }));
  const first = steps[0]?.cell?.value || 1;
  const max = Math.max(...steps.map(step => step.cell?.value ?? 0), 1);
  const labelW = Math.min(170, Math.max(...steps.map(step => step.category.length)) * CHAR + 16);
  const valueW = 92;
  const room = width - labelW - valueW;
  const row = 40;
  return (
    <ChartFrame label={`funnel of ${prepared.valueTitle ?? "values"}`} height={steps.length * row} tip={null} width={width} frameRef={ref}>
      {steps.map((step, at) => {
        const value = step.cell?.value ?? 0;
        const w = Math.max(3, (value / max) * room);
        const center = labelW + room / 2;
        const prior = at > 0 ? steps[at - 1].cell?.value ?? 0 : 0;
        return (
          <g key={step.category}>
            <text x={0} y={at * row + 22} fontSize={12.5} fill="var(--foreground)">{step.category}</text>
            <rect x={center - w / 2} y={at * row + 6} width={w} height={26} rx={6} fill="var(--ring)" fillOpacity={step.cell?.estimate ? 0.3 : 0.9 - at * (0.5 / Math.max(1, steps.length - 1))} stroke={step.cell?.estimate ? "var(--ring)" : undefined} strokeDasharray={step.cell?.estimate ? "4 3" : undefined} />
            <text x={width} y={at * row + 18} textAnchor="end" fontSize={12.5} fill="var(--foreground)" className="tabular-nums">{fmt(prepared, value)}</text>
            <text x={width} y={at * row + 32} textAnchor="end" fontSize={11} fill="var(--muted-foreground)" className="tabular-nums">{at === 0 ? "start" : at === 1 || !prior ? `${((value / first) * 100).toFixed(0)}% of start` : `${((value / first) * 100).toFixed(0)}% · ${((value / prior) * 100).toFixed(0)}% of previous`}</text>
          </g>
        );
      })}
    </ChartFrame>
  );
}

export function ChartView({ block }: { block: ChartBlock }) {
  const prepared = useMemo(() => prepare(block), [block]);
  switch (block.form) {
    case "scatter":
      return <Scatter block={block} />;
    case "heatmap":
      return <Heatmap block={block} />;
    case "proportion":
      return <Proportion block={block} />;
    case "funnel":
      return <Funnel block={block} prepared={prepared} />;
    case "line":
    case "area":
      return <Lines block={block} prepared={prepared} />;
    default:
      return prepared.horizontal && block.form !== "waterfall"
        ? <HorizontalBars block={block} prepared={prepared} />
        : <VerticalBars block={block} prepared={prepared} />;
  }
}
