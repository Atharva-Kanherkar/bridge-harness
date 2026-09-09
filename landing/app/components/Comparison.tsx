
type Verdict = "yes" | "partial" | "no";

const rows: { capability: string; bridge: Verdict; others: Verdict }[] = [
  { capability: "Parallel agents, each in its own Git worktree", bridge: "yes", others: "partial" },
  { capability: "Delegation gated by capability tier, write scope, and budget", bridge: "yes", others: "no" },
  { capability: "Typed, durable worker results carried between sessions", bridge: "yes", others: "no" },
  { capability: "Completion gate that demands another harness family", bridge: "yes", others: "no" },
  { capability: "Append-only session forest with fork and rewind", bridge: "yes", others: "partial" },
  { capability: "Compaction that checkpoints instead of rewriting history", bridge: "yes", others: "no" },
  { capability: "Daemon plus a one-shot CLI for CI", bridge: "yes", others: "partial" },
  { capability: "Generated RPC contract with drift enforced by tests", bridge: "yes", others: "no" },
];

const mark: Record<Verdict, { glyph: string; className: string; label: string }> = {
  yes: { glyph: "✓", className: "text-success", label: "Yes" },
  partial: { glyph: "◐", className: "text-warning", label: "Sometimes" },
  no: { glyph: "—", className: "text-faint", label: "No" },
};

function Cell({ verdict }: { verdict: Verdict }) {
  const value = mark[verdict];
  return (
    <td className="border-t border-border px-4 py-3 text-center">
      <span className={value.className} aria-hidden="true">
        {value.glyph}
      </span>
      <span className="sr-only">{value.label}</span>
    </td>
  );
}

export default function Comparison() {
  return (
    <section className="border-t border-border">
      <div className="mx-auto max-w-6xl px-6 py-20">
        <div>
          <h2 className="max-w-2xl font-display text-3xl font-semibold tracking-tight sm:text-4xl">
            Built for supervision, not retrofitted
          </h2>
          <p className="mt-4 max-w-2xl text-[15px] leading-7 text-muted-foreground">
            Terminal wrappers stop at a prompt and an editor was built for one person typing. Bridge is the layer that decides
            what an agent is allowed to do before it does it.
          </p>
        </div>
        <div className="mt-12 overflow-x-auto">
          <table className="w-full min-w-[560px] border-collapse text-[13.5px]">
            <thead>
              <tr className="text-left text-[11px] uppercase tracking-wider text-faint">
                <th className="px-4 py-3 font-normal">Capability</th>
                <th className="w-32 px-4 py-3 text-center font-normal">Bridge</th>
                <th className="w-48 px-4 py-3 text-center font-normal">Wrappers and agent IDEs</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((row) => (
                <tr key={row.capability}>
                  <td className="border-t border-border px-4 py-3 text-muted-foreground">{row.capability}</td>
                  <Cell verdict={row.bridge} />
                  <Cell verdict={row.others} />
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <div>
          <p className="mt-6 text-[12.5px] leading-6 text-faint">
            The right column generalizes across a category rather than naming a product, and what any given tool ships changes
            quickly.
          </p>
        </div>
      </div>
    </section>
  );
}
