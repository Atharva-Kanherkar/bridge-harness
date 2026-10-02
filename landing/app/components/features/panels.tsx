"use client";

import HarnessMark from "../app/HarnessMark";
import { useInView } from "./useInView";

/*
 * The feature visuals. Each one is the mechanism running rather than a photograph of it:
 * parts animate on a scroll timeline as the panel comes into view, so the page shows the
 * thing happening. Markup and type scale are lifted from the app, and every value is
 * deterministic so the server and the client draw the same frame.
 *
 * A `.stage` part animates when its panel scrolls into view, staggered by `--i`, and replays
 * if you scroll back. Without JavaScript the parts render plainly.
 */
const frame = "relative overflow-hidden rounded-xl border border-border-card bg-background shadow-[0_0_0_1px_#000,0_30px_90px_-30px_rgba(0,0,0,0.9)]";

/** A model list that opens, moves its tick from Codex to Claude, and updates the composer. */
export function SwitchHarness() {
  const { ref, play } = useInView<HTMLDivElement>();
  const codex = ["GPT Luna", "GPT Terra", "GPT Sol"];
  const claude = ["Claude Sonnet", "Claude Opus", "Claude Haiku"];

  return (
    <div ref={ref} data-play={play} className={`${frame} flex h-[380px] flex-col justify-end p-4 sm:h-[440px]`} aria-hidden="true">
      <div className="flex flex-col gap-3 px-1 pb-4 opacity-60">
        <p className="text-[13px] leading-6 text-body">Rotating on read means two concurrent reads can both mint a token.</p>
        <div className="ml-auto w-fit rounded-2xl border border-border bg-accent/70 px-3.5 py-2 text-[13px] text-foreground">
          Keep going, but switch to a stronger model.
        </div>
      </div>

      <div className="relative">
        <div
          style={{ "--i": 0, "--ill": "rise" } as React.CSSProperties}
          className="stage absolute bottom-12 left-0 z-10 w-[280px] overflow-hidden rounded-xl border border-border bg-popover shadow-2xl shadow-black/60"
        >
          <div className="border-b border-border px-3 py-2 text-[12px] text-muted-foreground">Search models</div>

          <div className="px-2 py-2">
            <div className="flex items-center gap-1.5 px-1.5 pb-1.5">
              <HarnessMark harness="codex" size={11} />
              <span className="font-mono text-[10px] uppercase tracking-[0.08em] text-muted-foreground">Codex</span>
            </div>
            {codex.map((model, i) => (
              <div key={model} className={`flex h-7 items-center justify-between rounded-md px-1.5 text-[12.5px] ${i === 0 ? "text-muted-foreground" : "text-muted-foreground"}`}>
                {model}
                {i === 0 && (
                  <span style={{ "--i": 2, "--ill": "light" } as React.CSSProperties} className="stage text-[11px] text-faint [animation-direction:reverse]">
                    ✓
                  </span>
                )}
              </div>
            ))}

            <div className="mt-2 flex items-center gap-1.5 px-1.5 pb-1.5">
              <HarnessMark harness="claude" size={11} />
              <span className="font-mono text-[10px] uppercase tracking-[0.08em] text-muted-foreground">Claude Code</span>
            </div>
            {claude.map((model, i) => (
              <div
                key={model}
                style={i === 1 ? ({ "--i": 3, "--ill": "light" } as React.CSSProperties) : undefined}
                className={`flex h-7 items-center justify-between rounded-md px-1.5 text-[12.5px] ${
                  i === 1 ? "stage bg-accent font-medium text-foreground" : "text-muted-foreground"
                }`}
              >
                {model}
                {i === 1 && <span className="text-[11px] text-foreground">✓</span>}
              </div>
            ))}
          </div>

          <p style={{ "--i": 4, "--ill": "rise" } as React.CSSProperties} className="stage border-t border-border px-3 py-2 text-[11px] text-muted-foreground">
            Switching restarts the provider session. History stays.
          </p>
        </div>

        <div className="flex items-center gap-2 rounded-xl border border-border-card bg-card px-3 py-2.5">
          <span className="flex-1 text-[13px] text-muted-foreground">Send a follow-up…</span>
          <span style={{ "--i": 5, "--ill": "light" } as React.CSSProperties} className="stage inline-flex items-center gap-1.5 rounded-md border border-border px-2 py-1 text-[12px] text-foreground">
            <HarnessMark harness="claude" size={12} />
            Claude Code · Claude Opus
          </span>
        </div>
      </div>
    </div>
  );
}

/* The daily-cost series, one per harness. Fixed values so the curve never shifts. */
const series = [
  { id: "claude", stroke: "stroke-harness-claude", fill: "fill-harness-claude/20", points: [6, 9, 22, 74, 38, 30, 52, 28, 18, 41, 15, 12, 9, 26, 58, 22, 16, 34, 12, 8, 18, 46, 68, 44, 30, 55, 38, 62, 24, 12] },
  { id: "codex", stroke: "stroke-harness-codex", fill: "fill-harness-codex/20", points: [2, 3, 4, 6, 5, 4, 8, 6, 3, 5, 4, 3, 2, 4, 6, 5, 4, 22, 14, 6, 4, 8, 12, 9, 6, 18, 11, 24, 9, 5] },
  { id: "opencode", stroke: "stroke-harness-opencode", fill: "fill-harness-opencode/20", points: [1, 2, 2, 3, 4, 14, 8, 4, 2, 3, 2, 2, 1, 3, 4, 3, 2, 5, 4, 3, 2, 4, 16, 7, 4, 9, 6, 13, 22, 6] },
];

const W = 560;
const H = 190;
const peak = 80;

function path(points: number[], close: boolean) {
  const step = W / (points.length - 1);
  const y = (v: number) => H - (v / peak) * H;
  const line = points.map((v, i) => `${i === 0 ? "M" : "L"}${(i * step).toFixed(1)} ${y(v).toFixed(1)}`).join(" ");
  return close ? `${line} L${W} ${H} L0 ${H} Z` : line;
}

/** The usage screen: a total that counts, harness shares, and three series drawing in. */
export function Usage() {
  const { ref, play } = useInView<HTMLDivElement>();
  const stats = [
    ["Processed tokens", "10.4B"],
    ["Cached input", "10.1B"],
    ["Output", "40.6M"],
    ["Cache savings", "$43,267.54"],
  ];
  const harnesses = [
    { id: "claude", name: "Claude", cost: "$5,535.39", note: "76.5% of cost · 7.76B tokens" },
    { id: "codex", name: "Codex", cost: "$1,092.75", note: "15.1% of cost · 1.73B tokens" },
    { id: "opencode", name: "OpenCode", cost: "$609.39", note: "8.4% of cost · 952M tokens" },
  ];

  return (
    <div ref={ref} data-play={play} className={`${frame} p-4`} aria-hidden="true">
      <div className="grid gap-3 lg:grid-cols-[minmax(0,210px)_minmax(0,1fr)]">
        <div className="rounded-lg border border-border-card bg-card p-3.5">
          <div style={{ "--i": 0, "--ill": "rise" } as React.CSSProperties} className="stage font-heading text-[26px] leading-none tracking-[-0.02em] text-foreground tabular-nums">
            $7,237.52
          </div>
          <p className="mt-1.5 text-[10.5px] text-muted-foreground">61,176 requests · API estimate</p>
          <div className="mt-3 flex flex-col gap-2.5">
            {harnesses.map((harness, i) => (
              <div key={harness.id} style={{ "--i": i + 1, "--ill": "rise" } as React.CSSProperties} className="stage">
                <div className="flex items-center gap-1.5 text-[12px]">
                  <HarnessMark harness={harness.id} size={11} />
                  <span className="text-foreground">{harness.name}</span>
                  <span className="ml-auto tabular-nums text-foreground">{harness.cost}</span>
                </div>
                <p className="mt-0.5 pl-4 text-[10.5px] text-muted-foreground">{harness.note}</p>
              </div>
            ))}
          </div>
        </div>

        <div className="rounded-lg border border-border-card bg-card p-3.5">
          <div className="text-[12px] font-medium text-foreground">Daily cost</div>
          <svg viewBox={`0 0 ${W} ${H}`} className="mt-2 h-[140px] w-full" fill="none" preserveAspectRatio="none">
            {[0, 0.5, 1].map(at => (
              <line key={at} x1="0" x2={W} y1={H * at} y2={H * at} className="stroke-border" strokeWidth="1" vectorEffect="non-scaling-stroke" />
            ))}
            {series.map((s, i) => (
              <g key={s.id}>
                <path d={path(s.points, true)} className={`stage ${s.fill}`} style={{ "--i": i + 1, "--ill": "light" } as React.CSSProperties} />
                <path
                  d={path(s.points, false)}
                  className={`stage ${s.stroke}`}
                  style={{ "--i": i, "--ill": "draw", "--draw": 900, strokeDasharray: 900 } as React.CSSProperties}
                  strokeWidth="1.5"
                  vectorEffect="non-scaling-stroke"
                />
              </g>
            ))}
          </svg>
          <div className="mt-1.5 flex justify-between font-mono text-[10px] text-faint">
            <span>Aug 13</span>
            <span>Aug 27</span>
            <span>Sep 11</span>
          </div>
        </div>
      </div>

      <div className="mt-3 grid grid-cols-2 gap-2 lg:grid-cols-4">
        {stats.map(([label, value], i) => (
          <div
            key={label}
            style={{ "--i": i + 4, "--ill": "rise" } as React.CSSProperties}
            className="stage rounded-lg border border-border-card bg-card px-3 py-2"
          >
            <div className="truncate text-[10px] text-muted-foreground">{label}</div>
            <div className="mt-0.5 text-[14px] tabular-nums text-foreground">{value}</div>
          </div>
        ))}
      </div>
    </div>
  );
}

/** Four chats running at once, each tile pinned to its own branch and worktree. */
export function Parallel() {
  const { ref, play } = useInView<HTMLDivElement>();
  const tiles = [
    { harness: "claude", title: "Rotate refresh tokens", branch: "feat/token-rotation", state: "working", diff: "+142 −38" },
    { harness: "codex", title: "Flaky checkout test", branch: "fix/checkout-flake", state: "needs you", diff: "+12 −4" },
    { harness: "opencode", title: "Paginate audit log", branch: "feat/audit-pages", state: "working", diff: "+88 −21" },
    { harness: "cursor", title: "Settings copy pass", branch: "chore/settings-copy", state: "done", diff: "+30 −30" },
  ];

  return (
    <div ref={ref} data-play={play} className={`${frame} p-3`} aria-hidden="true">
      <div className="flex items-center gap-2 px-1 pb-3">
        <span className="text-[13px] font-semibold text-foreground">Mission Control</span>
        <span className="rounded bg-muted px-1.5 py-0.5 font-mono text-[10.5px] text-foreground">4 live</span>
        <span className="ml-auto text-[11px] text-faint">one worktree each</span>
      </div>

      <div className="grid grid-cols-2 gap-2">
        {tiles.map((tile, i) => (
          <div
            key={tile.branch}
            style={{ "--i": i, "--ill": "rise" } as React.CSSProperties}
            className={`stage flex h-[150px] flex-col rounded-md border bg-card p-3 sm:h-[170px] ${tile.state === "needs you" ? "border-ring/65" : "border-border"}`}
          >
            <div className="flex items-center gap-1.5">
              <HarnessMark harness={tile.harness} size={12} />
              <span className="min-w-0 flex-1 truncate text-[12px] font-medium text-foreground">{tile.title}</span>
            </div>
            <span className="mt-1.5 truncate font-mono text-[10.5px] text-faint">{tile.branch}</span>
            <div className="mt-auto flex items-center justify-between">
              <span
                style={{ "--i": i + 4, "--ill": "light" } as React.CSSProperties}
                className={`stage text-[10px] uppercase tracking-[0.06em] ${
                  tile.state === "needs you" ? "text-warning" : tile.state === "done" ? "text-muted-foreground" : "text-success"
                }`}
              >
                {tile.state}
              </span>
              <span className="font-mono text-[10.5px] tabular-nums text-muted-foreground">{tile.diff}</span>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

/** An agent asks for a signed-in tab, you approve it, and it drives a throwaway copy. */
export function Browser() {
  const { ref, play } = useInView<HTMLDivElement>();

  return (
    <div ref={ref} data-play={play} className={`${frame} p-4`} aria-hidden="true">
      <div style={{ "--i": 0, "--ill": "rise" } as React.CSSProperties} className="stage rounded-lg border border-border-card bg-card p-3.5">
        <div className="flex items-center gap-1.5 text-[12px] text-muted-foreground">
          <HarnessMark harness="claude" size={12} />
          Claude Code wants a browser
        </div>
        <p className="mt-1.5 text-[13px] text-foreground">Signed in to github.com, to check the deploy preview on the PR.</p>
        <div className="mt-3 flex gap-2">
          <span style={{ "--i": 2, "--ill": "light" } as React.CSSProperties} className="stage inline-flex h-7 items-center rounded-md bg-primary px-2.5 text-[12px] text-primary-foreground">
            Allow once
          </span>
          <span className="inline-flex h-7 items-center rounded-md border border-border px-2.5 text-[12px] text-muted-foreground">Deny</span>
        </div>
      </div>

      <div style={{ "--i": 3, "--ill": "rise" } as React.CSSProperties} className="stage mt-3 overflow-hidden rounded-lg border border-border-card bg-background">
        <div className="flex items-center gap-2 border-b border-border bg-card px-3 py-2">
          <span className="flex gap-1">
            <span className="size-2 rounded-full bg-faint-2" />
            <span className="size-2 rounded-full bg-faint-2" />
            <span className="size-2 rounded-full bg-faint-2" />
          </span>
          <span className="flex-1 truncate rounded bg-muted px-2 py-0.5 font-mono text-[10.5px] text-muted-foreground">github.com/acme/web/pull/214</span>
          <span className="text-[10.5px] text-faint">throwaway copy</span>
        </div>
        <div className="flex flex-col gap-2 p-3.5">
          <div className="h-2.5 w-2/3 rounded bg-muted" />
          <div className="h-2 w-full rounded bg-muted/60" />
          <div className="h-2 w-5/6 rounded bg-muted/60" />
          <div style={{ "--i": 5, "--ill": "light" } as React.CSSProperties} className="stage mt-2 flex items-center gap-2 rounded-md border border-border px-2.5 py-2">
            <span className="size-1.5 rounded-full bg-success" />
            <span className="text-[12px] text-foreground">Preview deployed</span>
            <span className="ml-auto font-mono text-[10.5px] text-faint">agent clicked · 2s ago</span>
          </div>
        </div>
      </div>

      <p style={{ "--i": 6, "--ill": "rise" } as React.CSSProperties} className="stage mt-3 px-1 text-[11.5px] text-faint">
        Gone when the turn ends. Take the wheel any time.
      </p>
    </div>
  );
}

/** The ledger fills, a run folds into a compaction checkpoint, and the session resumes from it. */
export function History() {
  const { ref, play } = useInView<HTMLDivElement>();
  const rows = [
    ["message.completed", "completed"],
    ["plan.updated", "inProgress"],
    ["tool.started", "completed"],
    ["delegation.spawned", "working"],
    ["delegation.result", "completed"],
  ];

  return (
    <div ref={ref} data-play={play} className={frame} aria-hidden="true">
      <div className="flex items-center gap-2 border-b border-border px-4 py-2.5">
        <span className="inline-flex items-center gap-1 rounded-md border border-border bg-card px-2 py-1 text-[12px] text-foreground">{"{ }"} Transcript</span>
        <span className="ml-auto text-[11px] text-faint">Append only</span>
      </div>

      <div className="flex flex-col px-4 py-2 font-mono text-[11.5px]">
        {rows.map(([kind, status], i) => (
          <div key={i} style={{ "--i": i, "--ill": "rise" } as React.CSSProperties} className="stage flex items-center gap-3 border-b border-border/60 py-2 last:border-b-0">
            <span className="w-5 tabular-nums text-faint-2">{i + 1}</span>
            <span className="flex-1 text-foreground">{kind}</span>
            <span className={status === "completed" ? "text-muted-foreground" : status === "working" ? "text-success" : "text-warning"}>{status}</span>
          </div>
        ))}
      </div>

      <div className="px-4 pb-4">
        <div style={{ "--i": 5 } as React.CSSProperties} className="stage overflow-hidden rounded-lg border border-l-2 border-border border-l-info bg-card">
          <div className="flex items-baseline gap-2 px-4 pt-3">
            <b className="text-[13px] font-semibold text-foreground">Context compacted</b>
            <small className="text-[11px] text-info">184k → 23k tokens</small>
          </div>
          <p className="px-4 pb-3 pt-1 text-[12.5px] text-muted-foreground">The five events above are still here. Only the model&rsquo;s view got shorter.</p>
        </div>

        <div style={{ "--i": 7 } as React.CSSProperties} className="stage mt-3 flex items-center gap-2 rounded-lg border border-border bg-code px-3 py-2.5">
          <span className="size-1.5 rounded-full bg-success" />
          <span className="font-mono text-[11.5px] text-foreground">Resumed from checkpoint</span>
          <span className="ml-auto font-mono text-[11px] text-faint">after restart · turn 14</span>
        </div>
      </div>
    </div>
  );
}

/** A completion gate that refuses the author's own verdict and waits for another model family. */
export function Verify() {
  const { ref, play } = useInView<HTMLDivElement>();
  const checks = [
    { label: "cargo test -p bridge-core", by: "deterministic", result: "passed" },
    { label: "Review by Claude Code", by: "same family as author", result: "rejected" },
    { label: "Review by Codex", by: "different family", result: "passed" },
  ];

  return (
    <div ref={ref} data-play={play} className={`${frame} p-4`} aria-hidden="true">
      <div className="flex items-center gap-2 px-1 pb-3">
        <HarnessMark harness="claude" size={13} />
        <span className="text-[13px] font-medium text-foreground">Rotate refresh tokens</span>
        <span className="ml-auto font-mono text-[11px] text-faint">written by Claude Opus</span>
      </div>

      <div className="flex flex-col gap-2">
        {checks.map((check, i) => (
          <div
            key={check.label}
            style={{ "--i": i * 2, "--ill": "rise" } as React.CSSProperties}
            className="stage flex items-center gap-3 rounded-lg border border-border-card bg-card px-3.5 py-2.5"
          >
            <span className={`size-1.5 shrink-0 rounded-full ${check.result === "passed" ? "bg-success" : "bg-destructive"}`} />
            <span className="min-w-0 flex-1">
              <span className="block truncate text-[13px] text-foreground">{check.label}</span>
              <span className="block text-[11px] text-faint">{check.by}</span>
            </span>
            <span
              style={{ "--i": i * 2 + 1, "--ill": "light" } as React.CSSProperties}
              className={`stage text-[11.5px] ${check.result === "passed" ? "text-muted-foreground" : "text-destructive"}`}
            >
              {check.result === "rejected" ? "doesn’t count" : "passed"}
            </span>
          </div>
        ))}
      </div>

      <div style={{ "--i": 6 } as React.CSSProperties} className="stage mt-3 flex items-center gap-2 rounded-lg border border-border bg-code px-3 py-2.5">
        <span className="size-1.5 rounded-full bg-success" />
        <span className="font-mono text-[11.5px] text-foreground">Ready to merge</span>
        <span className="ml-auto font-mono text-[11px] text-faint">2 of 2 gates</span>
      </div>
    </div>
  );
}

/** Memories saved in one chat being recalled into another harness in another repo. */
export function Memory() {
  const { ref, play } = useInView<HTMLDivElement>();
  const memories = [
    { kind: "preference", text: "Use bun, never npm, in every repo." },
    { kind: "decision", text: "Auth tokens rotate on write, not on read." },
    { kind: "constraint", text: "Never push straight to main." },
  ];

  return (
    <div ref={ref} data-play={play} className={`${frame} p-4`} aria-hidden="true">
      <div className="flex items-baseline gap-2 px-1 pb-3">
        <h4 className="font-display text-[18px] font-semibold tracking-[-0.02em] text-foreground">Memory</h4>
        <span className="text-[11.5px] text-muted-foreground">saved from a Claude chat in api/</span>
      </div>

      <div className="flex flex-col gap-2">
        {memories.map((memory, i) => (
          <div
            key={memory.kind}
            style={{ "--i": i, "--ill": "rise" } as React.CSSProperties}
            className="stage flex items-center gap-3 rounded-lg border border-border-card bg-card px-3.5 py-2.5"
          >
            <span className="w-[74px] shrink-0 font-mono text-[10.5px] text-faint">{memory.kind}</span>
            <span className="min-w-0 flex-1 truncate text-[13px] text-foreground">{memory.text}</span>
          </div>
        ))}
      </div>

      <div style={{ "--i": 4, "--ill": "rise" } as React.CSSProperties} className="stage mt-4 rounded-lg border border-border-card bg-background p-3">
        <div className="flex items-center gap-1.5 text-[11.5px] text-muted-foreground">
          <HarnessMark harness="codex" size={12} />
          Codex · web/
          <span style={{ "--i": 6, "--ill": "light" } as React.CSSProperties} className="stage ml-auto rounded bg-muted px-1.5 py-0.5 font-mono text-[10.5px] text-foreground">
            3 memories used
          </span>
        </div>
        <p className="mt-2 text-[13px] text-body">Switched the install step to bun and kept token rotation on write.</p>
      </div>
    </div>
  );
}
