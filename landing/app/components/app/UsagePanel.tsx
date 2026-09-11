import { harnessDot, usage } from "../../content/appScenes";

/** The Usage screen, with the daily-cost area drawing itself in as the scene opens. */
export default function UsagePanel({ step }: { step: number }) {
  const revealed = Math.min(1, Math.max(0, step / 4));
  const bars = usage.chart;
  const peak = Math.max(...bars);

  return (
    <section className="flex min-h-0 min-w-0 flex-col overflow-hidden">
      <div className="flex h-11 shrink-0 items-center gap-2 border-b border-border px-4 sm:px-6">
        <h3 className="text-[13px] font-semibold leading-4 text-foreground">Usage</h3>
        <span className="ml-auto inline-flex h-7 items-center gap-1 rounded-md border border-border bg-card p-0.5 text-[12px]">
          <span className="rounded bg-muted px-2 py-0.5 text-foreground">Usage</span>
          <span className="px-2 py-0.5 text-muted-foreground">Insights</span>
        </span>
      </div>

      <div className="min-h-0 flex-1 overflow-hidden px-5 py-4">
        <p className="max-w-xl text-[12.5px] leading-5 text-muted-foreground">
          Tokens processed across harnesses and what they would cost at API rates. Not money spent: subscription plans bill separately.
        </p>

        <div className="mt-3 flex flex-wrap items-center gap-2 text-[12px]">
          <span className="inline-flex items-center gap-1 rounded-md border border-border bg-card p-0.5">
            <span className="rounded bg-muted px-2 py-0.5 text-foreground">Cost</span>
            <span className="px-2 py-0.5 text-muted-foreground">Tokens</span>
          </span>
          <span className="inline-flex items-center gap-1 rounded-md border border-border bg-card p-0.5">
            {["24h", "7d", "30d", "90d"].map(range => (
              <span key={range} className={`rounded px-2 py-0.5 ${range === "30d" ? "bg-muted text-foreground" : "text-muted-foreground"}`}>
                {range}
              </span>
            ))}
          </span>
          <span className="ml-auto text-[11.5px] text-muted-foreground">Aug 13 to Sep 11</span>
        </div>

        <div className="mt-3 grid gap-3 lg:grid-cols-[minmax(0,260px)_1fr]">
          <div className="rounded-lg border border-border-card bg-card p-4">
            <div className="font-heading text-[30px] leading-none tracking-[-0.02em] text-foreground tabular-nums">{usage.total}</div>
            <p className="mt-1.5 text-[11.5px] text-muted-foreground">{usage.caption}</p>
            <div className="mt-3 flex flex-col gap-2.5">
              {usage.harnesses.map((row, i) => (
                <div key={row.label} className="animate-entry-in motion-reduce:animate-none" style={{ animationDelay: `${i * 90}ms` }}>
                  <div className="flex items-center gap-2 text-[12.5px]">
                    <span className={`size-2 shrink-0 rounded-full ${harnessDot[row.harness]}`} aria-hidden="true" />
                    <span className="text-foreground">{row.label}</span>
                    <span className="ml-auto tabular-nums text-foreground">{row.cost}</span>
                  </div>
                  <p className="mt-0.5 pl-4 text-[11px] text-muted-foreground">
                    {row.share}% of cost · {row.tokens} tokens
                  </p>
                </div>
              ))}
            </div>
          </div>

          <div className="rounded-lg border border-border-card bg-card p-4">
            <div className="text-[12.5px] font-medium text-foreground">Daily cost</div>
            <div className="mt-3 flex h-28 items-end gap-1">
              {bars.map((value, i) => (
                <span
                  key={i}
                  className="flex-1 origin-bottom rounded-sm bg-linear-to-t from-foreground/15 to-foreground/45 transition-[height] duration-700 ease-out"
                  style={{ height: `${revealed * (value / peak) * 100}%`, transitionDelay: `${i * 22}ms` }}
                  aria-hidden="true"
                />
              ))}
            </div>
            <div className="mt-2 flex justify-between font-mono text-[10.5px] text-faint">
              <span>Aug 13</span>
              <span>Aug 27</span>
              <span>Sep 11</span>
            </div>
          </div>
        </div>

        <div className="mt-3 grid grid-cols-3 gap-2 lg:grid-cols-6">
          {usage.stats.map((stat, i) => (
            <div
              key={stat.label}
              className="animate-entry-in rounded-lg border border-border-card bg-card px-3 py-2.5 motion-reduce:animate-none"
              style={{ animationDelay: `${150 + i * 60}ms` }}
            >
              <div className="truncate text-[10.5px] text-muted-foreground">{stat.label}</div>
              <div className="mt-0.5 text-[15px] tabular-nums text-foreground">{stat.value}</div>
            </div>
          ))}
        </div>

        <div className="mt-3 overflow-hidden rounded-lg border border-border-card bg-card">
          <div className="flex items-center justify-between px-4 py-2.5">
            <span className="text-[12.5px] font-medium text-foreground">Breakdown</span>
            <span className="inline-flex items-center gap-1 rounded-md border border-border p-0.5 text-[11.5px]">
              <span className="rounded bg-muted px-2 py-0.5 text-foreground">Model</span>
              <span className="px-2 py-0.5 text-muted-foreground">Day</span>
            </span>
          </div>
          <table className="w-full border-collapse text-[12px]">
            <thead>
              <tr className="text-left text-[10.5px] uppercase tracking-wider text-faint">
                <th className="px-4 py-1.5 font-normal">Model</th>
                <th className="px-4 py-1.5 text-right font-normal">Cost</th>
                <th className="px-4 py-1.5 text-right font-normal">Share</th>
                <th className="px-4 py-1.5 text-right font-normal">Tokens</th>
              </tr>
            </thead>
            <tbody>
              {usage.rows.map(row => (
                <tr key={row.model}>
                  <td className="border-t border-border px-4 py-2">
                    <span className="flex items-center gap-2">
                      <span className={`size-2 shrink-0 rounded-full ${harnessDot[row.harness]}`} aria-hidden="true" />
                      <span className="truncate font-mono text-[11.5px] text-foreground">{row.model}</span>
                    </span>
                  </td>
                  <td className="border-t border-border px-4 py-2 text-right tabular-nums text-foreground">{row.cost}</td>
                  <td className="border-t border-border px-4 py-2 text-right tabular-nums text-muted-foreground">{row.share}%</td>
                  <td className="border-t border-border px-4 py-2 text-right tabular-nums text-muted-foreground">{row.tokens}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>
    </section>
  );
}
