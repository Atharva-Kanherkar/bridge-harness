import Reveal from "./Reveal";

const features = [
  {
    title: "The policy engine owns every gate",
    text: "Capability tier, write scope, worktree isolation, concurrency, depth, retry limits, per-turn budgets, and approvals are decided before a worker starts. Routing and learning rank the candidates the policy already allows. They can never grant a permission, widen a scope, or skip an approval.",
    wide: true,
  },
  {
    title: "Session forest",
    text: "Append-only local history in SQLite. Entries are immutable, each with a parent and an event kind. Fork or rewind the conversation without pretending that files, commits, or provider state were rewound.",
  },
  {
    title: "Every task in its own worktree",
    text: "A task owns a branch and a worktree. Workers whose write scopes overlap get their own, so parallel agents never share a dirty tree. Read-only workers are checked for unexpected tracked-file changes, and write-capable workers must stay inside an approved scope.",
    wide: true,
  },
  {
    title: "Typed worker results",
    text: "A worker hands back a structured, durable result with file evidence rather than a paraphrase. A later worker can build on validated evidence from the parent branch.",
  },
  {
    title: "Cross-harness verification",
    text: "A completion gate can require evidence from a different harness family, so a worker's own report never closes its own task.",
  },
  {
    title: "Checkpoints and resume",
    text: "Compaction preserves the original events and adds a verified boundary instead of rewriting history. A resumed session says whether context came back hot, provider-native, from a checkpoint, or fresh.",
  },
  {
    title: "Usage and budgets",
    text: "Tokens, cost, and context health per harness and per task. Budgets are gates too: a worker that runs out of turn budget stops instead of improvising.",
  },
  {
    title: "One event model",
    text: "Messages, reasoning, plans, tool calls, approvals, file changes, errors, and artifacts render natively. If a structured adapter is incomplete, Bridge surfaces that instead of falling back to an embedded terminal UI.",
  },
  {
    title: "Daemon and CLI",
    text: "One daemon owns a data directory and fans events out over a Unix socket with a token handshake. The bridge exec --json one-shot attaches to it for a single call or one streamed turn, which is what CI uses.",
  },
  {
    title: "Generated protocol",
    text: "The RPC contract has a single source of truth. JSON schemas and the TypeScript client are generated from it and drift is test-enforced, so renaming a wire field breaks the build rather than a user session.",
  },
  {
    title: "Authenticated browser bridge",
    text: "Supervise one user-approved tab in your own logged-in browser. Bridge does not copy profiles or export cookies, and the lease expires when the tab closes or navigation crosses the granted domain.",
  },
  {
    title: "Managed runtimes",
    text: "Install or remove the runtimes behind Claude, Codex, and OpenCode as pinned npm closures verified by receipt. Vendor credentials are never collected, proxied, or migrated.",
  },
  {
    title: "Work board and terminal",
    text: "Worktree stats, diffs, policy checks, and pending approvals sit in one pane. A separate workspace terminal keeps ad-hoc shell work out of the agent conversation.",
  },
];

export default function FeatureGrid() {
  return (
    <section className="border-t border-border">
      <div className="mx-auto max-w-6xl px-6 py-20">
        <Reveal>
          <h2 className="max-w-2xl font-display text-3xl font-semibold tracking-tight sm:text-4xl">Supervised, end to end</h2>
          <p className="mt-4 max-w-2xl text-[15px] leading-7 text-muted-foreground">
            The speed of coding agents, with explicit boundaries around files, processes, approvals, delegation, and session state.
          </p>
        </Reveal>
        <Reveal className="mt-12 grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {features.map((feature) => (
            <div
              key={feature.title}
              className={`rounded-lg border border-border-card bg-card p-5 ${feature.wide ? "lg:col-span-2" : ""}`}
            >
              <h3 className="text-[15px] font-medium">{feature.title}</h3>
              <p className="mt-2 text-[13.5px] leading-6 text-muted-foreground">{feature.text}</p>
            </div>
          ))}
        </Reveal>
      </div>
    </section>
  );
}
