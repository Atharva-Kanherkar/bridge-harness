// The mark a harness wears while Bridge is starting it. Inline SVG, no external
// asset fetches, every path built here — the same rules `connectorLogos.tsx`
// follows, and for the same reason.
//
// The mark is **never load-bearing**: the row that shows it always also names
// the harness in the label beside it, so a session on an agent Bridge has no
// mark for reads exactly the same. That matters because the harness id space is
// open (see `harnessLabel`) — a mark is a nicety for the three built-ins, not a
// requirement for the fourth.
//
// Every mark is radially symmetric on purpose. `.harness-mark-live` turns it
// slowly, and a symmetric figure under slow rotation scintillates instead of
// looking like a logo being spun.

import { cn } from "@/lib/utils";

/** Arms radiating from the centre: the shape all three built-in marks share. */
function arms(count: number, inner: number, outer: number, offsetDegrees = 0): string {
  return Array.from({ length: count }, (_, index) => {
    const radians = ((360 / count) * index + offsetDegrees) * (Math.PI / 180);
    const [dx, dy] = [Math.cos(radians), Math.sin(radians)];
    return `M${(12 + dx * inner).toFixed(2)} ${(12 + dy * inner).toFixed(2)}L${(12 + dx * outer).toFixed(2)} ${(12 + dy * outer).toFixed(2)}`;
  }).join("");
}

type MarkShape = { path: string; strokeWidth: number; dot?: number };
type HarnessMarkDefinition = { shape: MarkShape; tint: string };

// Arm counts are chosen for how each figure resolves at 14px — the size the
// startup row actually uses — not for how it looks blown up. Ten arms through
// the centre muddies into a blob there; eight stays an asterisk.

/** Eight arms through the centre — the ✳ Claude Code draws for itself. */
const CLAUDE: MarkShape = { path: arms(8, 0, 9.2), strokeWidth: 2 };

/** Six arms off a hollow centre, echoing a six-fold knot without tracing one. */
const CODEX: MarkShape = { path: arms(6, 3.2, 9.2), strokeWidth: 2.4 };

/** A ring of ticks around a solid core. Twelve rather than eight so it never
 *  reads as Claude's asterisk at a glance — detached ticks, not arms. */
const OPENCODE: MarkShape = { path: arms(12, 6.2, 9.2), strokeWidth: 1.9, dot: 2.2 };

/** A gapped ring: a spinner, which needs no brand knowledge to be right. */
const UNKNOWN: MarkShape = {
  path: "M12 3.4A8.6 8.6 0 1 1 3.4 12",
  strokeWidth: 2.2,
};

/** Shape and tint stay together so adding a harness cannot update only one. */
const MARKS: Record<string, HarnessMarkDefinition> = {
  claude: { shape: CLAUDE, tint: "text-harness-claude" },
  codex: { shape: CODEX, tint: "text-harness-codex" },
  opencode: { shape: OPENCODE, tint: "text-harness-opencode" },
};

/** The tint class for a harness id — muted ink for one Bridge does not know. */
export function harnessTintClass(harness?: string | null): string {
  return (harness && MARKS[harness]?.tint) || "text-muted-foreground";
}

/**
 * A harness's mark.
 *
 * `live` adds the turn-and-breathe animation. Callers pass `live={false}` under
 * `prefers-reduced-motion` rather than relying on the stylesheet's global
 * animation freeze, so the frame that shows is the one this file drew and not
 * whichever frame the freeze happened to catch.
 */
export function HarnessMark({ harness, size = 14, live = false, className }: {
  harness?: string | null;
  size?: number;
  live?: boolean;
  className?: string;
}) {
  const shape = (harness && MARKS[harness]?.shape) || UNKNOWN;
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      className={cn("shrink-0", harnessTintClass(harness), live && "harness-mark-live", className)}
      aria-hidden="true"
    >
      <path
        d={shape.path}
        stroke="currentColor"
        strokeWidth={shape.strokeWidth}
        strokeLinecap="round"
        fill="none"
      />
      {shape.dot && <circle cx="12" cy="12" r={shape.dot} fill="currentColor" />}
    </svg>
  );
}
