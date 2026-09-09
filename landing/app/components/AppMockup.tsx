function Dot({ tone }: { tone: "success" | "warn" | "faint" | "fg" }) {
  const c = { success: "bg-success", warn: "bg-warn", faint: "bg-faint", fg: "bg-foreground" }[tone];
  return <span className={`inline-block size-1.5 shrink-0 rounded-full ${c}`} />;
}

function Row({ label, sub, tone, right, depth = 0, active = false }: {
  label: string; sub?: string; tone: "success" | "warn" | "faint" | "fg"; right?: string; depth?: number; active?: boolean;
}) {
  return (
    <div className={`flex items-center gap-2 rounded-md px-2 py-1.5 ${active ? "bg-muted" : ""}`} style={{ paddingLeft: 8 + depth * 14 }}>
      <Dot tone={tone} />
      <div className="min-w-0 flex-1">
        <div className="truncate text-[12px] leading-4 text-foreground">{label}</div>
        {sub && <div className="truncate text-[11px] leading-4 text-muted-foreground">{sub}</div>}
      </div>
      {right && <span className="text-[10px] tabular-nums text-faint">{right}</span>}
    </div>
  );
}

function Msg({ who, children, meta }: { who: "you" | "bridge"; children: React.ReactNode; meta?: string }) {
  return (
    <div className={`flex flex-col gap-1 ${who === "you" ? "items-end" : "items-start"}`}>
      {meta && <span className="text-[10px] uppercase tracking-wider text-faint">{meta}</span>}
      <div className={`max-w-[85%] rounded-xl px-3 py-2 text-[12.5px] leading-5 ${who === "you" ? "bg-muted text-foreground" : "text-foreground/90"}`}>
        {children}
      </div>
    </div>
  );
}

function Card({ title, right, children }: { title: string; right?: React.ReactNode; children: React.ReactNode }) {
  return (
    <div className="w-full rounded-lg border border-border-card bg-card">
      <div className="flex items-center justify-between border-b border-border px-3 py-1.5 text-[11px] text-muted-foreground">
        <span>{title}</span>{right}
      </div>
      <div className="px-3 py-2 text-[12px] leading-5">{children}</div>
    </div>
  );
}

export default function AppMockup() {
  return (
    <div className="w-full overflow-hidden rounded-xl border border-border-card bg-background text-left text-foreground shadow-[0_0_0_1px_#000,0_40px_80px_-30px_rgba(0,0,0,0.9)]">
      {/* title bar */}
      <div className="flex h-10 items-center border-b border-border bg-sidebar px-3">
        <div className="flex gap-1.5">
          <span className="size-3 rounded-full bg-[#ff5f57]" /><span className="size-3 rounded-full bg-[#febc2e]" /><span className="size-3 rounded-full bg-[#28c840]" />
        </div>
        <span className="ml-4 text-[12px] text-muted-foreground">Bridge</span>
        <div className="ml-auto flex items-center gap-2 text-[11px] text-muted-foreground">
          <span className="rounded-md border border-border px-2 py-0.5">harness · main</span>
          <span className="rounded-md border border-border px-2 py-0.5">3 workers live</span>
        </div>
      </div>

      <div className="grid h-[600px] grid-cols-[230px_1fr_320px] max-lg:grid-cols-[230px_1fr] max-md:grid-cols-1">
        {/* sidebar */}
        <aside className="flex flex-col border-r border-border bg-sidebar p-2 max-md:hidden">
          <div className="px-2 pb-2 pt-1 text-[10px] uppercase tracking-wider text-faint">Repositories</div>
          <Row label="harness" sub="github.com/bridge/harness" tone="fg" />
          <div className="mt-2 px-2 pb-1 text-[10px] uppercase tracking-wider text-faint">Tasks</div>
          <Row label="Worktree lifecycle inventory" sub="feat/worktree-lifecycle" tone="success" right="live" active />
          <Row label="impl · standard" sub="worker-2f9a · isolated" tone="success" depth={1} />
          <Row label="verify · codex" sub="worker-71c0 · read-only" tone="warn" depth={1} />
          <Row label="Usage ledger rollups" sub="feat/usage-tracking" tone="success" right="live" />
          <Row label="research · opencode" sub="worker-b330" tone="success" depth={1} />
          <Row label="Refresh-token rotation" sub="merged · PR #577" tone="faint" />
          <Row label="Sidebar archive action" sub="archived" tone="faint" />
          <div className="mt-auto flex items-center gap-2 border-t border-border px-2 pt-2 text-[11px] text-muted-foreground">
            <Dot tone="success" /> codex · claude · opencode
          </div>
        </aside>

        {/* conversation */}
        <section className="flex min-h-0 min-w-0 flex-col">
          <div className="flex h-9 items-center gap-4 border-b border-border px-4 text-[12px]">
            <span className="border-b border-foreground pb-2 pt-2 text-foreground">Conversation</span>
            <span className="text-muted-foreground">Changes <span className="text-faint">7</span></span>
            <span className="text-muted-foreground">Terminal</span>
            <span className="text-muted-foreground">Browser</span>
            <span className="ml-auto text-[11px] text-faint">fable-5-1 · high</span>
          </div>
          <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-hidden px-5 py-4">
            <Msg who="you">Reclaimed branches still show as active in the sidebar. Trace it and fix it, but I want a second harness to verify.</Msg>
            <Msg who="bridge" meta="orchestrator">
              The coordinator marks a branch reclaimed before the forest entry lands, so the sidebar reads one stale tick. Delegating the fix to an isolated worker.
            </Msg>
            <Card title="Delegation · implementation" right={<span className="text-success">running</span>}>
              <div className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-0.5 font-mono text-[11px] text-muted-foreground">
                <span>tier</span><span className="text-foreground">standard · medium</span>
                <span>write</span><span className="text-foreground">isolated · src-tauri/bridge-core/**</span>
                <span>verify</span><span className="text-foreground">cargo test -p bridge-core worktree::</span>
              </div>
            </Card>
            <Card title="Worker result · worker-2f9a" right={<span className="text-success">tests passed</span>}>
              Reordered reclaim after the forest append and added a regression test. 2 files changed, cargo test green.
            </Card>
            <Card title="Completion gate" right={<span className="text-warn">verifying</span>}>
              Codex verifier launched in the implementation worktree. Same-family evidence rejected by policy.
            </Card>
          </div>
          <div className="border-t border-border p-3">
            <div className="flex items-center gap-2 rounded-full border border-border-card bg-card px-3 py-2 text-[12px] text-faint">
              <span className="flex-1">Message Bridge…</span>
              <span className="rounded-full bg-foreground px-2 py-0.5 text-[10px] text-background">⏎</span>
            </div>
          </div>
        </section>

        {/* work view */}
        <aside className="flex flex-col border-l border-border bg-sidebar max-lg:hidden">
          <div className="flex h-9 items-center border-b border-border px-3 text-[12px] text-foreground">Work</div>
          <div className="flex flex-col gap-3 p-3">
            <Card title="Worktree">
              <div className="font-mono text-[11px] text-muted-foreground">
                .worktrees/worker-2f9a<br />
                <span className="text-foreground">+38 −12</span> · 2 files · clean
              </div>
            </Card>
            <Card title="worktree_coordinator.rs">
              <pre className="font-mono text-[10.5px] leading-4 text-muted-foreground">
{`- self.mark_reclaimed(&branch)?;
  forest.append(entry)?;
+ self.mark_reclaimed(&branch)?;`}
              </pre>
            </Card>
            <Card title="Policy">
              <div className="flex flex-col gap-1 text-[11px] text-muted-foreground">
                <span><span className="text-success">✓</span> write scope authorized</span>
                <span><span className="text-success">✓</span> isolation enforced</span>
                <span><span className="text-warn">●</span> approval pending · merge</span>
              </div>
            </Card>
            <Card title="Usage · today">
              <div className="font-mono text-[11px] text-muted-foreground">
                claude 412k · codex 96k · opencode 31k<br />
                <span className="text-foreground">$4.18</span> across 3 harnesses
              </div>
            </Card>
          </div>
        </aside>
      </div>
    </div>
  );
}
