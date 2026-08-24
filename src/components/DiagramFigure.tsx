import { useMemo } from "react";

/**
 * Bridge's diagram format: a small node/edge grid, not a general-purpose
 * graphing language. `row`/`col` place nodes on a fixed grid (no auto-layout
 * to fight with); `emphasis: "active"` is the one accent color a diagram gets
 * — reserve it for the thing the reader should follow, everything else stays
 * achromatic. Out-degree (computed, not authored) decides which nodes draw
 * slightly heavier, so a fork point looks like one without a manual flag.
 */
export type DiagramEmphasis = "default" | "muted" | "active";
export type DiagramMarker = "none" | "checkpoint" | "tip" | "continues";

export interface DiagramNode {
  id: string;
  label?: string;
  row: number;
  col: number;
  emphasis?: DiagramEmphasis;
  marker?: DiagramMarker;
  labelSide?: "right" | "below";
}

export interface DiagramEdge {
  from: string;
  to: string;
  /** true peels the edge off to the side (a smooth S-curve); default is a straight line. */
  curve?: boolean;
  emphasis?: DiagramEmphasis;
}

export interface DiagramSpec {
  nodes: DiagramNode[];
  edges: DiagramEdge[];
  /** The one claim this figure supports — rendered as the figcaption. */
  caption: string;
  ariaLabel: string;
}

const ROW_STEP = 56;
const COL_STEP = 66;
const PAD = 20;
const RIGHT_LABEL_BUDGET = 170;
const BELOW_LABEL_HEIGHT = 26;
// A "below" label is centered on its node, so it can overhang either side —
// unlike a "right" label, which only ever extends into RIGHT_LABEL_BUDGET's
// space. Any node at the grid's left or right edge needs this much extra
// margin reserved, or a centered label there clips against the viewBox edge.
const BELOW_LABEL_HALF_WIDTH = 40;
const CONTINUES_BUDGET = 56;
const R_NORMAL = 5;
const R_HEAVY = 6;
const HALO_R = 11;

export interface DiagramLayout {
  viewBox: string;
  positions: Record<string, { x: number; y: number }>;
  outDegree: Record<string, number>;
}

/** Pure grid → pixel layout, kept separate from rendering so the math is unit-testable on its own. */
export function layoutDiagram(spec: DiagramSpec): DiagramLayout {
  const rows = spec.nodes.map(node => node.row);
  const cols = spec.nodes.map(node => node.col);
  const minRow = Math.min(0, ...rows);
  const maxRow = Math.max(0, ...rows);
  const minCol = Math.min(0, ...cols);
  const maxCol = Math.max(0, ...cols);

  const edgeHasBelowLabel = (col: number) =>
    spec.nodes.some(node => node.col === col && node.label && node.labelSide === "below");
  const leftPad = PAD + (edgeHasBelowLabel(minCol) ? BELOW_LABEL_HALF_WIDTH : 0);
  const rightPad = PAD + RIGHT_LABEL_BUDGET + (edgeHasBelowLabel(maxCol) ? BELOW_LABEL_HALF_WIDTH : 0);

  const positions: Record<string, { x: number; y: number }> = {};
  for (const node of spec.nodes) {
    positions[node.id] = {
      x: leftPad + (node.col - minCol) * COL_STEP,
      y: PAD + (node.row - minRow) * ROW_STEP,
    };
  }

  const outDegree: Record<string, number> = {};
  for (const edge of spec.edges) outDegree[edge.from] = (outDegree[edge.from] ?? 0) + 1;

  const hasBelowLabel = spec.nodes.some(node => node.label && node.labelSide === "below");
  const hasContinues = spec.nodes.some(node => node.marker === "continues");

  const width = leftPad + (maxCol - minCol) * COL_STEP + rightPad;
  const height =
    PAD * 2 +
    (maxRow - minRow) * ROW_STEP +
    (hasBelowLabel ? BELOW_LABEL_HEIGHT : 0) +
    (hasContinues ? CONTINUES_BUDGET : 0);

  return { viewBox: `0 0 ${width} ${height}`, positions, outDegree };
}

const EMPHASIS_VALUES = new Set<string>(["default", "muted", "active"]);
const MARKER_VALUES = new Set<string>(["none", "checkpoint", "tip", "continues"]);
const LABEL_SIDE_VALUES = new Set<string>(["right", "below"]);

/** Guards against malformed LLM-authored JSON so a bad spec falls back to source, not a crash. */
export function isValidDiagramSpec(value: unknown): value is DiagramSpec {
  if (typeof value !== "object" || value === null) return false;
  const spec = value as Record<string, unknown>;
  if (typeof spec.caption !== "string" || spec.caption.trim() === "") return false;
  if (typeof spec.ariaLabel !== "string" || spec.ariaLabel.trim() === "") return false;
  if (!Array.isArray(spec.nodes) || spec.nodes.length === 0) return false;

  const ids = new Set<string>();
  for (const raw of spec.nodes) {
    if (typeof raw !== "object" || raw === null) return false;
    const node = raw as Record<string, unknown>;
    if (typeof node.id !== "string" || node.id === "" || ids.has(node.id)) return false;
    ids.add(node.id);
    if (typeof node.row !== "number" || typeof node.col !== "number") return false;
    if (node.label !== undefined && typeof node.label !== "string") return false;
    if (node.emphasis !== undefined && !EMPHASIS_VALUES.has(node.emphasis as string)) return false;
    if (node.marker !== undefined && !MARKER_VALUES.has(node.marker as string)) return false;
    if (node.labelSide !== undefined && !LABEL_SIDE_VALUES.has(node.labelSide as string)) return false;
  }

  if (!Array.isArray(spec.edges)) return false;
  for (const raw of spec.edges) {
    if (typeof raw !== "object" || raw === null) return false;
    const edge = raw as Record<string, unknown>;
    if (typeof edge.from !== "string" || typeof edge.to !== "string") return false;
    if (!ids.has(edge.from) || !ids.has(edge.to)) return false;
    if (edge.curve !== undefined && typeof edge.curve !== "boolean") return false;
    if (edge.emphasis !== undefined && !EMPHASIS_VALUES.has(edge.emphasis as string)) return false;
  }
  return true;
}

function markColor(emphasis: DiagramEmphasis | undefined): string {
  return emphasis === "active" ? "var(--ring)" : "currentColor";
}

/**
 * Renders a locked DiagramSpec as inline SVG. No theme-tracking hook needed —
 * every mark is `currentColor` or `var(--ring)`, so it repaints for free when
 * the `dark` class flips, unlike the Mermaid output it replaces.
 */
export function DiagramFigure({ spec }: { spec: DiagramSpec }) {
  const layout = useMemo(() => layoutDiagram(spec), [spec]);
  const { positions, outDegree } = layout;

  return (
    <figure className="my-[0.9em] flex flex-col items-start gap-[0.6em] [&_svg]:h-auto [&_svg]:w-full [&_svg]:max-w-[280px]">
      <svg viewBox={layout.viewBox} role="img" aria-label={spec.ariaLabel}>
        <g fill="none" strokeWidth={1.75}>
          {spec.edges.map((edge, index) => {
            const from = positions[edge.from];
            const to = positions[edge.to];
            const mid = (from.y + to.y) / 2;
            const d = edge.curve
              ? `M${from.x},${from.y} C${from.x},${mid} ${to.x},${mid} ${to.x},${to.y}`
              : `M${from.x},${from.y} L${to.x},${to.y}`;
            return (
              <path key={index} d={d} stroke={markColor(edge.emphasis)} opacity={edge.emphasis === "muted" ? 0.45 : 1} />
            );
          })}
        </g>

        {spec.nodes.map(node => {
          const pos = positions[node.id];
          const color = markColor(node.emphasis);
          const muted = node.emphasis === "muted";
          const radius = (outDegree[node.id] ?? 0) > 1 ? R_HEAVY : R_NORMAL;
          return (
            <g key={node.id} opacity={muted ? 0.45 : 1}>
              <circle cx={pos.x} cy={pos.y} r={radius} fill={color} />
              {node.marker === "checkpoint" && (
                <circle cx={pos.x} cy={pos.y} r={HALO_R} fill="none" stroke="var(--ring)" strokeWidth={1.5} opacity={0.55} />
              )}
              {node.marker === "tip" && (
                <circle
                  cx={pos.x} cy={pos.y} r={HALO_R} fill="none" stroke="var(--ring)" strokeWidth={1.5}
                  strokeDasharray="2.5 3" opacity={0.6}
                />
              )}
              {node.marker === "continues" && (
                <g fill={color}>
                  <circle cx={pos.x} cy={pos.y + 24} r={2} opacity={0.45} />
                  <circle cx={pos.x} cy={pos.y + 38} r={2} opacity={0.3} />
                  <circle cx={pos.x} cy={pos.y + 52} r={2} opacity={0.18} />
                </g>
              )}
            </g>
          );
        })}

        <g style={{ fontFamily: "var(--font-mono)", fontSize: 11 }}>
          {spec.nodes.filter(node => node.label).map(node => {
            const pos = positions[node.id];
            const color = markColor(node.emphasis);
            const muted = node.emphasis === "muted";
            if (node.labelSide === "below") {
              return (
                <text key={node.id} x={pos.x} y={pos.y + 22} textAnchor="middle" fill={color} opacity={muted ? 0.6 : 1}>
                  {node.label}
                </text>
              );
            }
            return (
              <g key={node.id}>
                <line x1={pos.x + 8} y1={pos.y} x2={pos.x + 20} y2={pos.y} stroke={color} strokeWidth={1.5} opacity={0.5} />
                <text x={pos.x + 26} y={pos.y + 3.5} fill={color} opacity={muted ? 0.6 : 1}>
                  {node.label}
                </text>
              </g>
            );
          })}
        </g>
      </svg>
      <figcaption className="text-[0.82em] leading-[1.55] text-muted-foreground">{spec.caption}</figcaption>
    </figure>
  );
}
