import { useReducedMotion } from "framer-motion";
import { cn } from "@/lib/utils";
import type { MergeStrategy } from "@/protocol/generated/protocol";

// The receipt's branch graph (issue #1028): head rail, base rail, and the
// purple choreography a merge performs on them. Squash gathers the branch's
// commits into one, which travels the curve into main and lands with a ring;
// rebase replays the commits one by one onto main; a merge commit draws the
// curve and lands a merge node. `celebrate` runs the choreography once
// (about 1.6s on one clock); otherwise the graph renders its final frame —
// a refetch that reports merged, and reduced motion, never replay it.

const CURVE = "M244 26 C 300 26, 300 60, 356 60";
const CURVE_LONG = "M244 26 C 312 26, 322 60, 384 60";

type MergeGraphProps = {
  strategy: MergeStrategy;
  commits: number;
  baseBranch: string;
  headBranch: string;
  /** Run the landing choreography once. False renders the settled frame. */
  celebrate: boolean;
};

export function MergeGraph({ strategy, commits, baseBranch, headBranch, celebrate }: MergeGraphProps) {
  const reduced = useReducedMotion();
  const play = celebrate && !reduced;

  // At most two commit nodes are drawn; the receipt's text line carries the
  // real count. One commit sits at the fork; two spread as in the mockup.
  const nodes = (commits || 1) > 1 ? [174, 244] : [244];

  return (
    <svg
      className="merge-graph"
      viewBox="0 0 460 84"
      role="img"
      aria-label={`The commits merged into ${baseBranch}`}
    >
      <path className="main-rail" d="M8 60 H452" />
      <path className="rail" d="M60 26 H244" />
      <path className="rail" d="M30 60 C 46 60, 44 26, 60 26" />
      <path className="curve" d={CURVE} />
      {strategy === "rebase" && <path className="curve" d={CURVE_LONG} />}

      {/* main's own history */}
      <circle className="node" cx="120" cy="60" r="4.5" />
      <circle className="node" cx="200" cy="60" r="4.5" />

      {/* the branch's commits: gather onto the fork (squash), depart one by
          one (rebase), or shift toward the junction (merge commit) */}
      {nodes.map((x, index) => (
        <circle
          key={index}
          className={cn(
            "node",
            strategy === "squash" && "c-gather c-vanish",
            strategy === "rebase" && (index === 0 ? "c-vanish d1" : "c-vanish d2"),
            strategy === "merge" && "c-gather",
          )}
          cx={x}
          cy="26"
          r="5"
          style={strategy === "squash"
            ? ({ "--merge-gather-x": `${244 - x}px` } as React.CSSProperties)
            : strategy === "merge"
              ? ({ "--merge-gather-x": "24px" } as React.CSSProperties)
              : undefined}
        />
      ))}

      {/* the travelling commits */}
      {(strategy === "squash" || strategy === "rebase") && play && (
        <circle className="traveller" r="5.5" style={{ offsetPath: `path("${CURVE}")` }} />
      )}
      {strategy === "rebase" && play && (
        <circle className="traveller b" r="5.5" style={{ offsetPath: `path("${CURVE_LONG}")` }} />
      )}

      {/* the landing */}
      {strategy === "rebase" ? (
        <>
          <circle className="landed" cx="356" cy="60" r="5.5" />
          <circle className="landed b" cx="384" cy="60" r="5.5" />
          <circle className="ring b" cx="384" cy="60" r="7" style={{ transformOrigin: "384px 60px" }} />
        </>
      ) : (
        <>
          <circle className="ring" cx="356" cy="60" r="7" style={{ transformOrigin: "356px 60px" }} />
          <circle className="landed" cx="356" cy="60" r={strategy === "merge" ? 6.5 : 6} />
        </>
      )}

      <text className="lbl lbl-old" x="62" y="16">{headBranch}</text>
      <text className="lbl" x="8" y="78">{baseBranch}</text>
    </svg>
  );
}
