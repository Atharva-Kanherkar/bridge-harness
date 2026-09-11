import CornerTicks from "./CornerTicks";
import SectionHeader from "./SectionHeader";

/*
 * The whole surface, organised the way Bridge itself is: the workspace tree, the agent tree,
 * and the conversation tree are separate hierarchies, and conflating them is the recurring
 * design bug. A fourth column holds the places you work. One board, no prose between the
 * reader and the list.
 */
type Column = { name: string; note: string; rows: { title: string; text: string }[] };

const columns: Column[] = [
  {
    name: "Workspace",
    note: "the filesystem tree",
    rows: [
      { title: "A worktree per task", text: "A branch and a checkout of its own, cut when the task opens." },
      { title: "Isolation on overlap", text: "Workers whose write scopes collide never share a dirty tree." },
      { title: "Diff and per-file risk", text: "Every change reviewed where it happened, ranked by blast radius." },
      { title: "Adopt or discard", text: "A worker's result merges into your workspace, or it does not." },
      { title: "Reclaimed on archive", text: "Finished checkouts are swept, with the bytes accounted for." },
    ],
  },
  {
    name: "Agents",
    note: "the delegation tree",
    rows: [
      { title: "An orchestrator that routes", text: "It plans, slices the work, and picks who runs each piece." },
      { title: "Policy owns every gate", text: "Tier, scope, isolation, depth, retries, approvals. Decided first." },
      { title: "Typed results", text: "Workers hand back structured evidence, not a paraphrase." },
      { title: "Cross-harness verification", text: "A gate can demand a second family. No worker closes its own task." },
      { title: "Budgets as gates", text: "A worker out of turn budget stops rather than improvising." },
    ],
  },
  {
    name: "History",
    note: "the conversation tree",
    rows: [
      { title: "Append-only session forest", text: "Immutable entries in SQLite, each with a parent and a kind." },
      { title: "Fork without losing files", text: "Rewind the conversation without pretending the disk rewound." },
      { title: "Checkpoints and resume", text: "Compaction adds a verified boundary instead of rewriting." },
      { title: "One event model", text: "Every provider's stream renders the same: plans, tools, diffs." },
      { title: "Usage and context health", text: "Tokens, cost, and window pressure per harness and per task." },
    ],
  },
  {
    name: "Surfaces",
    note: "where you work",
    rows: [
      { title: "Mission Control", text: "Every live chat at once, each tile a real conversation." },
      { title: "Agent Fleet", text: "Agent CLIs split into one terminal grid per checkout." },
      { title: "Work board", text: "Diffs, policy checks, and pending approvals in one pane." },
      { title: "Browser bridge", text: "One tab you approve, in your own browser. No profile copied." },
      { title: "Daemon and CLI", text: "bridge exec --json attaches for a single call or a streamed turn." },
    ],
  },
];

export default function CapabilityBoard() {
  return (
    <section className="border-t border-border">
      <div className="mx-auto max-w-6xl px-6 py-24">
        <SectionHeader
          eyebrow="Capabilities"
          title={
            <>
              Three trees, kept <em className="not-italic text-muted-foreground">apart.</em>
            </>
          }
          text="Bridge keeps the workspace, the agents, and the conversation as separate hierarchies, because blurring them is how agent tools lose your work."
          align="center"
        />

        <div className="reveal relative mt-14">
          <CornerTicks />
          <div className="grid gap-px overflow-hidden rounded-xl border border-border-card bg-border sm:grid-cols-2 lg:grid-cols-4">
            {columns.map(column => (
              <div key={column.name} className="bg-background">
                <header className="flex items-baseline gap-2 border-b border-border px-5 py-4">
                  <h3 className="text-[13px] font-semibold text-foreground">{column.name}</h3>
                  <span className="font-mono text-[10.5px] text-faint">{column.note}</span>
                </header>

                <ul>
                  {column.rows.map((row, i) => (
                    <li
                      key={row.title}
                      className="group relative border-b border-border px-5 py-4 last:border-b-0"
                    >
                      <span
                        aria-hidden="true"
                        className="absolute inset-0 bg-linear-to-r from-teal-400/10 via-blue-500/10 to-purple-500/10 opacity-0 transition-opacity duration-300 group-hover:opacity-100"
                      />
                      <div className="relative flex gap-3">
                        <span className="pt-0.5 font-mono text-[10.5px] tabular-nums text-faint-2">{String(i + 1).padStart(2, "0")}</span>
                        <div className="min-w-0">
                          <h4 className="text-[13.5px] font-medium leading-5 text-foreground">{row.title}</h4>
                          <p className="mt-1 text-[12.5px] leading-5 text-muted-foreground">{row.text}</p>
                        </div>
                      </div>
                    </li>
                  ))}
                </ul>
              </div>
            ))}
          </div>
        </div>
      </div>
    </section>
  );
}
