import SectionHeader from "./SectionHeader";
import { Checkpoints, CrossHarness, PolicyGates, SessionForest, TypedResult, Worktrees } from "./Illustrations";

const primary = [
  {
    title: "The policy engine owns every gate",
    text: "Tier, write scope, isolation, concurrency, depth, retries, budgets, approvals. Decided before a worker starts. Routing can rank, never grant.",
    art: <PolicyGates />,
  },
  {
    title: "A worktree for every worker",
    text: "Overlapping write scopes get separate worktrees, so parallel agents never share a dirty tree.",
    art: <Worktrees />,
  },
  {
    title: "Session forest",
    text: "Append-only history in SQLite. Fork or rewind the conversation without pretending files were rewound.",
    art: <SessionForest />,
  },
  {
    title: "Typed worker results",
    text: "Structured, durable results with file evidence rather than a paraphrase. Later workers build on validated evidence.",
    art: <TypedResult />,
  },
  {
    title: "Cross-harness verification",
    text: "A completion gate can demand evidence from a different harness family. No worker closes its own task.",
    art: <CrossHarness />,
  },
  {
    title: "Checkpoints and resume",
    text: "Compaction adds a verified boundary instead of rewriting history, and a resumed session says how context came back.",
    art: <Checkpoints />,
  },
];

const more = [
  ["Usage and budgets", "Tokens, cost, and context health per harness and task. A worker out of budget stops instead of improvising."],
  ["One event model", "Messages, reasoning, plans, tool calls, approvals, and diffs render natively for every provider."],
  ["Daemon and CLI", "One daemon owns a data directory. bridge exec --json attaches for a single call or a streamed turn."],
  ["Generated protocol", "Schemas and the TypeScript client come from one source. Drift fails the build, not a session."],
  ["Authenticated browser bridge", "One approved tab in your own browser. No profile copied, no cookies exported, lease ends with the tab."],
  ["Managed runtimes", "Pinned npm closures for each harness, verified by receipt. Vendor credentials are never touched."],
  ["Mission Control", "Every active chat live in one grid, each tile a real conversation with its own composer and approvals."],
  ["Work board and terminal", "Diffs, policy checks, and approvals in one pane. A separate terminal keeps shell work out of the transcript."],
];

export default function Capabilities() {
  return (
    <section id="capabilities" className="border-t border-border">
      <div className="mx-auto max-w-6xl px-6 py-24">
        <SectionHeader
          eyebrow="Capabilities"
          title={
            <>
              Supervised, <em className="not-italic text-muted-foreground">end to end.</em>
            </>
          }
          text="Agent speed, with explicit boundaries around files, processes, approvals, delegation, and session state."
        />

        <div className="mt-14 grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {primary.map((item) => (
            <article
              key={item.title}
              className="reveal group flex flex-col rounded-xl border border-border-card bg-card p-4 transition-colors duration-300 hover:border-faint-2"
            >
              {item.art}
              <h3 className="mt-5 px-1 font-display text-[1.125rem] font-semibold leading-snug tracking-[-0.02em] text-foreground">{item.title}</h3>
              <p className="mt-2 px-1 pb-1 text-[13.5px] leading-6 text-muted-foreground">{item.text}</p>
            </article>
          ))}
        </div>

        <div className="reveal mt-16 grid gap-x-12 gap-y-0 border-t border-border md:grid-cols-2">
          {more.map(([title, text], i) => (
            <div key={title} className="flex gap-5 border-b border-border py-5">
              <span className="pt-0.5 font-mono text-[11px] tabular-nums text-faint">{String(i + 7).padStart(2, "0")}</span>
              <div>
                <h3 className="text-[15px] font-medium text-foreground">{title}</h3>
                <p className="mt-1 text-[13.5px] leading-6 text-muted-foreground">{text}</p>
              </div>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
