/*
 * Small DOM/SVG illustrations for the capabilities grid. Each is achromatic except where
 * color carries meaning, and every part is in its resting state without JavaScript.
 * The `ill-part` utility animates parts in as the card scrolls into view.
 */

const frame = "relative h-40 w-full overflow-hidden rounded-lg border border-border bg-code";

export function PolicyGates() {
  const gates = ["tier", "scope", "isolation", "concurrency", "budget", "approval"];
  return (
    <div className={`${frame} flex flex-col justify-center gap-2 px-5`} aria-hidden="true">
      <div className="flex flex-wrap gap-1.5">
        {gates.map((gate, i) => (
          <span
            key={gate}
            style={{ "--i": i } as React.CSSProperties}
            className="ill-part inline-flex h-7 items-center gap-1.5 rounded-md border border-border-card bg-card px-2 font-mono text-[11px] text-muted-foreground"
          >
            <span className="size-1.5 rounded-full bg-success" />
            {gate}
          </span>
        ))}
      </div>
      <div className="mt-2 flex items-center gap-2 font-mono text-[11px]">
        <span className="text-faint">request</span>
        <span
          style={{ "--i": 6, "--ill": "grow-x" } as React.CSSProperties}
          className="ill-part h-px flex-1 origin-left bg-linear-to-r from-border-card to-success/70"
        />
        <span style={{ "--i": 7 } as React.CSSProperties} className="ill-part rounded border border-success/40 px-1.5 py-0.5 text-success">
          spawn isolated
        </span>
      </div>
    </div>
  );
}

export function Worktrees() {
  return (
    <div className={frame} aria-hidden="true">
      <svg viewBox="0 0 320 160" className="absolute inset-0 h-full w-full" fill="none">
        <path d="M24 80 H296" className="stroke-border-card" strokeWidth="2" />
        <path
          d="M80 80 C110 80 110 44 140 44 H296"
          className="ill-part stroke-foreground/70"
          style={{ "--i": 0, "--ill": "draw", "--draw": 320, strokeDasharray: 320 } as React.CSSProperties}
          strokeWidth="1.5"
        />
        <path
          d="M120 80 C150 80 150 116 180 116 H296"
          className="ill-part stroke-foreground/45"
          style={{ "--i": 1, "--ill": "draw", "--draw": 320, strokeDasharray: 320 } as React.CSSProperties}
          strokeWidth="1.5"
        />
        <circle cx="24" cy="80" r="4" className="fill-background stroke-faint" strokeWidth="1.5" />
        <circle cx="80" cy="80" r="4" className="fill-background stroke-faint" strokeWidth="1.5" />
        <circle cx="120" cy="80" r="4" className="fill-background stroke-faint" strokeWidth="1.5" />
        <circle cx="200" cy="44" r="4" style={{ "--i": 2 } as React.CSSProperties} className="ill-part fill-foreground" />
        <circle cx="240" cy="116" r="4" style={{ "--i": 3 } as React.CSSProperties} className="ill-part fill-foreground/60" />
      </svg>
      <span className="absolute left-5 top-4 font-mono text-[11px] text-faint">main</span>
      <span style={{ "--i": 2 } as React.CSSProperties} className="ill-part absolute right-5 top-[22px] font-mono text-[11px] text-muted-foreground">
        .worktrees/worker-2f9a
      </span>
      <span style={{ "--i": 3 } as React.CSSProperties} className="ill-part absolute bottom-3 right-5 font-mono text-[11px] text-muted-foreground">
        .worktrees/worker-8c11
      </span>
    </div>
  );
}

export function SessionForest() {
  const nodes = [
    { x: 40, y: 80 },
    { x: 90, y: 80 },
    { x: 140, y: 80 },
    { x: 190, y: 56 },
    { x: 240, y: 56 },
    { x: 290, y: 56 },
    { x: 190, y: 104 },
    { x: 240, y: 104 },
  ];
  const edges = [
    [0, 1],
    [1, 2],
    [2, 3],
    [3, 4],
    [4, 5],
    [2, 6],
    [6, 7],
  ];
  return (
    <div className={frame} aria-hidden="true">
      <svg viewBox="0 0 320 160" className="absolute inset-0 h-full w-full" fill="none">
        {edges.map(([a, b], i) => (
          <line
            key={i}
            x1={nodes[a].x}
            y1={nodes[a].y}
            x2={nodes[b].x}
            y2={nodes[b].y}
            className={`ill-part ${i >= 5 ? "stroke-faint-2" : "stroke-foreground/50"}`}
            style={{ "--i": i, "--ill": "draw", "--draw": 60, strokeDasharray: 60 } as React.CSSProperties}
            strokeWidth="1.5"
          />
        ))}
        {nodes.map((node, i) => (
          <rect
            key={i}
            x={node.x - 6}
            y={node.y - 6}
            width="12"
            height="12"
            rx="3"
            className={`ill-part ${i >= 6 ? "fill-card stroke-faint-2" : i === 5 ? "fill-foreground" : "fill-card stroke-foreground/70"}`}
            style={{ "--i": i } as React.CSSProperties}
            strokeWidth="1.5"
          />
        ))}
      </svg>
      <span className="absolute left-5 top-4 font-mono text-[11px] text-faint">append-only</span>
      <span className="absolute bottom-3 right-5 font-mono text-[11px] text-muted-foreground">active branch · 1 fork kept</span>
    </div>
  );
}

export function TypedResult() {
  const lines: [string, string, string][] = [
    ["status", '"verified"', "text-success"],
    ["filesChanged", "2", "text-foreground"],
    ["tests", '["cargo test -p bridge-core"]', "text-foreground"],
    ["evidenceId", '"ev_7f3a"', "text-foreground"],
  ];
  return (
    <div className={`${frame} flex flex-col justify-center px-5 font-mono text-[11.5px] leading-6`} aria-hidden="true">
      <span className="text-faint">{"{"}</span>
      {lines.map(([key, value, tone], i) => (
        <span key={key} style={{ "--i": i, "--ill": "rise" } as React.CSSProperties} className="ill-part pl-4">
          <span className="text-muted-foreground">{key}</span>
          <span className="text-faint">: </span>
          <span className={tone}>{value}</span>
          <span className="text-faint">,</span>
        </span>
      ))}
      <span className="text-faint">{"}"}</span>
    </div>
  );
}

export function CrossHarness() {
  return (
    <div className={`${frame} flex items-center justify-center gap-3 px-5`} aria-hidden="true">
      <span style={{ "--i": 0 } as React.CSSProperties} className="ill-part flex flex-col items-center gap-2">
        <span className="grid size-11 place-items-center rounded-full border border-harness-claude/40 bg-harness-claude/10">
          <span className="size-3 rounded-full bg-harness-claude" />
        </span>
        <span className="font-mono text-[11px] text-muted-foreground">implemented</span>
      </span>
      <span className="relative flex h-px w-20 items-center">
        <span
          style={{ "--i": 1, "--ill": "grow-x" } as React.CSSProperties}
          className="ill-part h-px w-full origin-left bg-foreground/50"
        />
        <span style={{ "--i": 2 } as React.CSSProperties} className="ill-part absolute -top-2.5 right-0 text-[13px] leading-none text-foreground/60">
          ›
        </span>
      </span>
      <span style={{ "--i": 3 } as React.CSSProperties} className="ill-part flex flex-col items-center gap-2">
        <span className="grid size-11 place-items-center rounded-full border border-harness-codex/40 bg-harness-codex/10">
          <span className="size-3 rounded-full bg-harness-codex" />
        </span>
        <span className="font-mono text-[11px] text-muted-foreground">verified</span>
      </span>
      <span
        style={{ "--i": 4 } as React.CSSProperties}
        className="ill-part absolute bottom-3 right-5 rounded border border-border-card px-1.5 py-0.5 font-mono text-[10.5px] text-faint"
      >
        same-family rejected
      </span>
    </div>
  );
}

export function Checkpoints() {
  const marks = [12, 30, 48, 66, 84];
  return (
    <div className={`${frame} flex flex-col justify-center px-5`} aria-hidden="true">
      <div className="relative h-px w-full bg-border-card">
        {marks.map((left, i) => (
          <span
            key={left}
            style={{ left: `${left}%`, "--i": i } as React.CSSProperties}
            className={`ill-part absolute top-1/2 size-2.5 -translate-x-1/2 -translate-y-1/2 rounded-full ${
              i === 3 ? "bg-foreground ring-4 ring-foreground/15" : "bg-faint-2"
            }`}
          />
        ))}
      </div>
      <div className="mt-6 flex justify-between font-mono text-[11px] text-faint">
        <span>events kept</span>
        <span style={{ "--i": 4, "--ill": "rise" } as React.CSSProperties} className="ill-part text-muted-foreground">
          checkpoint · verified
        </span>
      </div>
      <span style={{ "--i": 5, "--ill": "rise" } as React.CSSProperties} className="ill-part mt-3 self-end rounded-md border border-border-card bg-card px-2 py-1 font-mono text-[11px] text-muted-foreground">
        resumed from checkpoint
      </span>
    </div>
  );
}
