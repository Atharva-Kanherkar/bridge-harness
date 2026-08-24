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
// The floor for column pitch — widened per-diagram below when a label needs
// more room than this to clear its neighbor. A fixed 66px works for the
// common case (short labels, or nodes without one) but three consecutive
// labeled nodes at that pitch collide regardless of which side the label
// renders on — there's no label placement that rescues a gap that's just
// too narrow, so the fix has to be the gap itself, not the placement.
const COL_STEP_MIN = 66;
const PAD = 20;
const BELOW_LABEL_HEIGHT = 26;
const CONTINUES_BUDGET = 56;
const R_NORMAL = 5;
const R_HEAVY = 6;
const HALO_R = 11;

// Geist Mono is monospace, so character count is a reliable width proxy —
// no DOM measurement needed. A right-side label that would run into the next
// same-row node is illegible, not just untidy, so this isn't an authoring
// nicety: an authored spec (frequently model-generated, with no way to
// preview its own output) gets a correctness fallback, not just a rule to
// follow. LABEL_MAX_CHARS is a second, independent net for a label that's
// simply too long regardless of neighbors.
const MONO_CHAR_WIDTH = 6.6;
const LABEL_RIGHT_OFFSET = 26;
const LABEL_MIN_GAP = 10;
const LABEL_MAX_CHARS = 18;

export function truncateLabel(label: string): string {
  return label.length > LABEL_MAX_CHARS ? `${label.slice(0, LABEL_MAX_CHARS - 1)}…` : label;
}

function estimateLabelWidth(label: string): number {
  return label.length * MONO_CHAR_WIDTH;
}

/**
 * A right-side label sits at the node's own y — exactly where a horizontal
 * edge to a same-row neighbor runs. No amount of column spacing rescues that:
 * the edge spans the whole gap, so the text renders struck-through. Any label
 * a same-row edge would cross drops below the line instead (safe, because
 * column pitch is already widened to fit the widest label). A label that
 * would run into the next node's text is the second, rarer trigger.
 */
function computeLabelSides(
  spec: DiagramSpec,
  positions: Record<string, { x: number; y: number }>,
): Record<string, "right" | "below"> {
  const sides: Record<string, "right" | "below"> = {};
  const rowOf = new Map(spec.nodes.map(node => [node.id, node.row]));
  const sameRowEdges = spec.edges.filter(edge => rowOf.get(edge.from) === rowOf.get(edge.to));

  const byRow = new Map<number, DiagramNode[]>();
  for (const node of spec.nodes) {
    if (!node.label) continue;
    const list = byRow.get(node.row) ?? [];
    list.push(node);
    byRow.set(node.row, list);
  }
  for (const nodesInRow of byRow.values()) {
    const sorted = [...nodesInRow].sort((a, b) => a.col - b.col);
    sorted.forEach((node, index) => {
      if (node.labelSide === "below") {
        sides[node.id] = "below";
        return;
      }
      const labelStart = positions[node.id].x + 8;
      const labelEnd = positions[node.id].x + LABEL_RIGHT_OFFSET + estimateLabelWidth(truncateLabel(node.label!));
      const edgeCrosses = sameRowEdges.some(edge => {
        if (rowOf.get(edge.from) !== node.row) return false;
        const lo = Math.min(positions[edge.from].x, positions[edge.to].x);
        const hi = Math.max(positions[edge.from].x, positions[edge.to].x);
        return lo < labelEnd && hi > labelStart;
      });
      const next = sorted[index + 1];
      const textCollides = !!next && labelEnd + LABEL_MIN_GAP > positions[next.id].x;
      sides[node.id] = edgeCrosses || textCollides ? "below" : "right";
    });
  }
  return sides;
}

export interface DiagramLayout {
  viewBox: string;
  width: number;
  height: number;
  positions: Record<string, { x: number; y: number }>;
  outDegree: Record<string, number>;
  labelSides: Record<string, "right" | "below">;
}

/** Pure grid → pixel layout, kept separate from rendering so the math is unit-testable on its own. */
export function layoutDiagram(spec: DiagramSpec): DiagramLayout {
  const rows = spec.nodes.map(node => node.row);
  const cols = spec.nodes.map(node => node.col);
  const minRow = Math.min(0, ...rows);
  const maxRow = Math.max(0, ...rows);
  const minCol = Math.min(0, ...cols);

  // The column pitch widens once, for the whole diagram, to whatever its
  // widest label needs — guaranteeing no two same-row neighbors can collide
  // regardless of which side either label renders on. A single long label
  // among otherwise-short ones costs some sparseness elsewhere in the grid;
  // that's a cheaper price than a diagram that's illegible where it counts.
  const widestLabel = Math.max(
    0,
    ...spec.nodes.filter(node => node.label).map(node => estimateLabelWidth(truncateLabel(node.label!))),
  );
  const colStep = Math.max(COL_STEP_MIN, widestLabel + LABEL_RIGHT_OFFSET + LABEL_MIN_GAP);

  // Collision detection only needs relative gaps between columns, which a
  // uniform left-padding shift never changes — so it's safe to compute
  // against this unpadded pass before the padding it depends on is known.
  const rawPositions: Record<string, { x: number; y: number }> = {};
  for (const node of spec.nodes) {
    rawPositions[node.id] = {
      x: PAD + (node.col - minCol) * colStep,
      y: PAD + (node.row - minRow) * ROW_STEP,
    };
  }
  const labelSides = computeLabelSides(spec, rawPositions);

  // Margins come from what each node actually draws — a centered below-label
  // can overhang its node's x on both sides, a right-label extends only
  // rightward — instead of fixed budgets, which either clip a wide label at
  // the grid's edge or pad empty space the drawing never uses.
  let leftOverhang = 0;
  let rightExtent = 0;
  for (const node of spec.nodes) {
    const gridX = (node.col - minCol) * colStep;
    const labelWidth = node.label ? estimateLabelWidth(truncateLabel(node.label)) : 0;
    const below = !!node.label && labelSides[node.id] === "below";
    if (below) leftOverhang = Math.max(leftOverhang, labelWidth / 2 - gridX);
    const nodeRight = below
      ? Math.max(gridX + labelWidth / 2, gridX + HALO_R)
      : node.label
        ? gridX + LABEL_RIGHT_OFFSET + labelWidth
        : gridX + HALO_R;
    rightExtent = Math.max(rightExtent, nodeRight);
  }
  const leftPad = PAD + Math.max(0, leftOverhang);

  const positions: Record<string, { x: number; y: number }> = {};
  for (const node of spec.nodes) {
    positions[node.id] = {
      x: leftPad + (node.col - minCol) * colStep,
      y: PAD + (node.row - minRow) * ROW_STEP,
    };
  }

  const outDegree: Record<string, number> = {};
  for (const edge of spec.edges) outDegree[edge.from] = (outDegree[edge.from] ?? 0) + 1;

  const hasBelowLabel = spec.nodes.some(node => node.label && labelSides[node.id] === "below");
  const hasContinues = spec.nodes.some(node => node.marker === "continues");

  const width = leftPad + rightExtent + PAD;
  const height =
    PAD * 2 +
    (maxRow - minRow) * ROW_STEP +
    (hasBelowLabel ? BELOW_LABEL_HEIGHT : 0) +
    (hasContinues ? CONTINUES_BUDGET : 0);

  return { viewBox: `0 0 ${width} ${height}`, width, height, positions, outDegree, labelSides };
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
  const { positions, outDegree, labelSides } = layout;

  return (
    // Sized to its own content at 1:1 (the 11px label text means something
    // specific only at native scale) via width/height attributes, not CSS —
    // max-width only ever shrinks an oversized diagram to fit its column,
    // it never stretches a small one to fill it.
    <figure className="my-[0.9em] flex flex-col items-start gap-[0.6em] [&_svg]:h-auto [&_svg]:max-w-[min(100%,480px)]">
      <svg viewBox={layout.viewBox} width={layout.width} height={layout.height} role="img" aria-label={spec.ariaLabel}>
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
            const label = truncateLabel(node.label!);
            if (labelSides[node.id] === "below") {
              return (
                <text key={node.id} x={pos.x} y={pos.y + 22} textAnchor="middle" fill={color} opacity={muted ? 0.6 : 1}>
                  {label}
                </text>
              );
            }
            return (
              <g key={node.id}>
                <line x1={pos.x + 8} y1={pos.y} x2={pos.x + 20} y2={pos.y} stroke={color} strokeWidth={1.5} opacity={0.5} />
                <text x={pos.x + 26} y={pos.y + 3.5} fill={color} opacity={muted ? 0.6 : 1}>
                  {label}
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
