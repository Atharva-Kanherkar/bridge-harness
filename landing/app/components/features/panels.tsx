import HarnessMark from "../app/HarnessMark";

/*
 * The feature visuals. Each one is the mechanism running rather than a photograph of it:
 * parts animate on a scroll timeline as the panel comes into view, so the page shows the
 * thing happening. Markup and type scale are lifted from the app, and every value is
 * deterministic so the server and the client draw the same frame.
 *
 * `ill-part` is the scroll-timeline utility in globals.css. Where it is unsupported the
 * parts simply render in their finished state.
 */
const frame = "relative overflow-hidden rounded-xl border border-border-card bg-background shadow-[0_0_0_1px_#000,0_30px_90px_-30px_rgba(0,0,0,0.9)]";

/** A model list that opens, moves its tick from Codex to Claude, and updates the composer. */
export function SwitchHarness() {
  const codex = ["GPT Luna", "GPT Terra", "GPT Sol"];
  const claude = ["Claude Sonnet", "Claude Opus", "Claude Haiku"];

  return (
    <div className={`${frame} flex h-[380px] flex-col justify-end p-4 sm:h-[440px]`} aria-hidden="true">
      <div className="flex flex-col gap-3 px-1 pb-4 opacity-60">
        <p className="text-[13px] leading-6 text-body">Rotating on read means two concurrent reads can both mint a token.</p>
        <div className="ml-auto w-fit rounded-2xl border border-border bg-accent/70 px-3.5 py-2 text-[13px] text-foreground">
          Keep going, but switch to a stronger model.
        </div>
      </div>

      <div className="relative">
        <div
          style={{ "--i": 0, "--ill": "rise" } as React.CSSProperties}
          className="ill-part absolute bottom-12 left-0 z-10 w-[280px] overflow-hidden rounded-xl border border-border bg-popover shadow-2xl shadow-black/60"
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
                  <span style={{ "--i": 2, "--ill": "light" } as React.CSSProperties} className="ill-part text-[11px] text-faint [animation-direction:reverse]">
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
                  i === 1 ? "ill-part bg-accent font-medium text-foreground" : "text-muted-foreground"
                }`}
              >
                {model}
                {i === 1 && <span className="text-[11px] text-foreground">✓</span>}
              </div>
            ))}
          </div>

          <p style={{ "--i": 4, "--ill": "rise" } as React.CSSProperties} className="ill-part border-t border-border px-3 py-2 text-[11px] text-muted-foreground">
            Switching restarts the provider session. History stays.
          </p>
        </div>

        <div className="flex items-center gap-2 rounded-xl border border-border-card bg-card px-3 py-2.5">
          <span className="flex-1 text-[13px] text-muted-foreground">Send a follow-up…</span>
          <span style={{ "--i": 5, "--ill": "light" } as React.CSSProperties} className="ill-part inline-flex items-center gap-1.5 rounded-md border border-border px-2 py-1 text-[12px] text-foreground">
            <HarnessMark harness="claude" size={12} />
            Claude Code · Claude Opus
          </span>
        </div>
      </div>
    </div>
  );
}

/** The append-only ledger, filling row by row. */
export function SessionStorage() {
  const rows = [
    ["message.completed", "completed"],
    ["plan.updated", "inProgress"],
    ["tool.started", "completed"],
    ["message.completed", "completed"],
    ["delegation.spawned", "working"],
    ["delegation.spawned", "working"],
    ["delegation.result", "completed"],
    ["delegation.result", "completed"],
  ];

  return (
    <div className={frame} aria-hidden="true">
      <div className="flex items-center gap-2 border-b border-border px-4 py-2.5">
        <span className="inline-flex items-center gap-1 rounded-md border border-border bg-card px-2 py-1 text-[12px] text-foreground">{"{ }"} Transcript</span>
        <span className="inline-flex items-center gap-1 rounded-md bg-muted px-2 py-1 text-[12px] text-foreground">Stream</span>
        <span className="px-2 py-1 text-[12px] text-muted-foreground">Entries</span>
        <span className="ml-auto text-[11px] text-faint">Append only</span>
      </div>

      <div className="flex flex-col px-4 py-2 font-mono text-[11.5px]">
        {rows.map(([kind, status], i) => (
          <div
            key={i}
            style={{ "--i": i, "--ill": "rise" } as React.CSSProperties}
            className="ill-part flex items-center gap-3 border-b border-border/60 py-2 last:border-b-0"
          >
            <span className="w-5 tabular-nums text-faint-2">{i + 1}</span>
            <span className="flex-1 text-foreground">{kind}</span>
            <span className={status === "completed" ? "text-muted-foreground" : status === "working" ? "text-success" : "text-warning"}>{status}</span>
            <span className="tabular-nums text-faint">12:58:13</span>
          </div>
        ))}
      </div>

      <div className="flex items-center justify-between border-t border-border px-4 py-2.5 font-mono text-[11px] text-muted-foreground">
        <span style={{ "--i": 8, "--ill": "light" } as React.CSSProperties} className="ill-part">
          8 of 8 events
        </span>
        <span className="text-faint">session-1</span>
      </div>
    </div>
  );
}

/** Marketplace rows landing one at a time, with an install completing. */
export function Agents() {
  const agents = [
    { id: "claude", name: "Claude Code", note: "Anthropic's coding agent · 0.3.209", installed: true },
    { id: "codex", name: "Codex", note: "OpenAI's coding agent · 0.147.0" },
    { id: "cursor", name: "Cursor", note: "Coding agent" },
    { id: "opencode", name: "OpenCode", note: "Open-source coding agent" },
  ];

  return (
    <div className={`${frame} p-4`} aria-hidden="true">
      <div className="px-1 pb-3">
        <h4 className="font-display text-[20px] font-semibold tracking-[-0.02em] text-foreground">Agents</h4>
        <p className="text-[12px] text-muted-foreground">Install and manage coding agents.</p>
      </div>

      <div className="overflow-hidden rounded-lg border border-border-card">
        {agents.map((agent, i) => (
          <div
            key={agent.id}
            style={{ "--i": i, "--ill": "rise" } as React.CSSProperties}
            className="ill-part flex items-center gap-3 border-b border-border bg-card px-3.5 py-3 last:border-b-0"
          >
            <span className="grid size-8 shrink-0 place-items-center rounded-md border border-border bg-background">
              <HarnessMark harness={agent.id} size={15} />
            </span>
            <span className="min-w-0 flex-1">
              <span className="block text-[13px] font-medium text-foreground">{agent.name}</span>
              <span className="block truncate text-[11.5px] text-muted-foreground">{agent.note}</span>
            </span>
            <span
              style={{ "--i": i + 4, "--ill": "light" } as React.CSSProperties}
              className={`ill-part inline-flex h-7 shrink-0 items-center rounded-md px-2.5 text-[12px] ${
                agent.installed ? "border border-border text-muted-foreground" : "bg-primary text-primary-foreground"
              }`}
            >
              {agent.installed ? "Uninstall" : "Install"}
            </span>
          </div>
        ))}
      </div>

      <p style={{ "--i": 8, "--ill": "rise" } as React.CSSProperties} className="ill-part mt-3 px-1 text-[11.5px] text-faint">
        Plugins and skills install the same way, and your own roles route beside them.
      </p>
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
    <div className={`${frame} p-4`} aria-hidden="true">
      <div className="grid gap-3 lg:grid-cols-[minmax(0,210px)_minmax(0,1fr)]">
        <div className="rounded-lg border border-border-card bg-card p-3.5">
          <div style={{ "--i": 0, "--ill": "rise" } as React.CSSProperties} className="ill-part font-heading text-[26px] leading-none tracking-[-0.02em] text-foreground tabular-nums">
            $7,237.52
          </div>
          <p className="mt-1.5 text-[10.5px] text-muted-foreground">61,176 requests · API estimate</p>
          <div className="mt-3 flex flex-col gap-2.5">
            {harnesses.map((harness, i) => (
              <div key={harness.id} style={{ "--i": i + 1, "--ill": "rise" } as React.CSSProperties} className="ill-part">
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
                <path d={path(s.points, true)} className={`ill-part ${s.fill}`} style={{ "--i": i + 1, "--ill": "light" } as React.CSSProperties} />
                <path
                  d={path(s.points, false)}
                  className={`ill-part ${s.stroke}`}
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
            className="ill-part rounded-lg border border-border-card bg-card px-3 py-2"
          >
            <div className="truncate text-[10px] text-muted-foreground">{label}</div>
            <div className="mt-0.5 text-[14px] tabular-nums text-foreground">{value}</div>
          </div>
        ))}
      </div>
    </div>
  );
}
