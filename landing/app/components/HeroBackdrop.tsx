/*
 * The hero backdrop is Bridge's own metaphor rather than decoration: a trunk with worktree
 * branches peeling off, running in parallel, and merging back — the thing the product does,
 * drawn in hairlines. It traces itself once on load, then holds still.
 *
 * Everything is deterministic so the server and the client draw the same figure, and every
 * value is low enough contrast that the headline stays the brightest thing on the page.
 */

const W = 1680;
const H = 560;
const TRUNK = 300;
const STEP = 84;

type Branch = { from: number; to: number; y: number; live?: boolean };

/** Where each worktree leaves the trunk and where it lands again. */
const branches: Branch[] = [
  { from: 2, to: 7, y: 196 },
  { from: 4, to: 11, y: 404, live: true },
  { from: 8, to: 14, y: 132 },
  { from: 10, to: 17, y: 460 },
  { from: 13, to: 19, y: 240 },
];

const x = (i: number) => i * STEP;

/** Out of the trunk with a shoulder, along the branch, then back in the same way. */
function branchPath({ from, to, y }: Branch) {
  const x1 = x(from);
  const x2 = x(to);
  const shoulder = 46;
  return `M${x1} ${TRUNK} C${x1 + shoulder} ${TRUNK} ${x1 + shoulder} ${y} ${x1 + shoulder * 2} ${y} L${x2 - shoulder * 2} ${y} C${x2 - shoulder} ${y} ${x2 - shoulder} ${TRUNK} ${x2} ${TRUNK}`;
}

/** Commits: every trunk stop, plus two along each branch. */
const trunkNodes = Array.from({ length: 21 }, (_, i) => ({ cx: x(i), cy: TRUNK }));
const branchNodes = branches.flatMap(branch => {
  const a = x(branch.from) + 92;
  const b = x(branch.to) - 92;
  return [a, (a + b) / 2, b].map(cx => ({ cx, cy: branch.y, live: branch.live }));
});

export default function HeroBackdrop() {
  return (
    <div aria-hidden="true" className="pointer-events-none absolute inset-0 overflow-hidden">
      <div className="absolute inset-0 bg-grid opacity-40" />

      {/* Line-number gutters, the way an editor frames a file. */}
      <div className="absolute inset-y-0 left-0 w-14 bg-gutter opacity-70 [mask-image:linear-gradient(to_bottom,transparent,black_20%,black_80%,transparent)]" />
      <div className="absolute inset-y-0 right-0 w-14 bg-gutter opacity-70 [mask-image:linear-gradient(to_bottom,transparent,black_20%,black_80%,transparent)]" />

      <div className="absolute left-1/2 top-0 -translate-x-1/2">
        <svg
          width={W}
          height={H}
          viewBox={`0 0 ${W} ${H}`}
          fill="none"
          className="[mask-image:radial-gradient(80%_70%_at_50%_38%,black_35%,transparent_78%)]"
        >
          {/* The trunk. */}
          <path
            d={`M0 ${TRUNK} H${W}`}
            className="trace stroke-foreground/[0.14]"
            strokeWidth="1"
            style={{ "--draw": W, "--i": 0, strokeDasharray: W } as React.CSSProperties}
          />

          {branches.map((branch, i) => {
            const length = (x(branch.to) - x(branch.from)) + Math.abs(branch.y - TRUNK) * 2;
            return (
              <path
                key={i}
                d={branchPath(branch)}
                className={`trace ${branch.live ? "stroke-foreground/[0.22]" : "stroke-foreground/[0.09]"}`}
                strokeWidth="1"
                style={{ "--draw": length, "--i": i + 1, strokeDasharray: length } as React.CSSProperties}
              />
            );
          })}

          {trunkNodes.map((node, i) => (
            <circle
              key={`t${i}`}
              cx={node.cx}
              cy={node.cy}
              r="2.5"
              className="node-pop fill-background stroke-foreground/25"
              strokeWidth="1"
              style={{ "--i": i } as React.CSSProperties}
            />
          ))}

          {branchNodes.map((node, i) => (
            <circle
              key={`b${i}`}
              cx={node.cx}
              cy={node.cy}
              r="2.5"
              className={`node-pop ${node.live ? "fill-foreground/40" : "fill-background stroke-foreground/15"}`}
              strokeWidth="1"
              style={{ "--i": i + 6 } as React.CSSProperties}
            />
          ))}
        </svg>
      </div>

      <div className="absolute inset-x-0 bottom-0 h-36 bg-linear-to-t from-background to-transparent" />
    </div>
  );
}
