
import SectionHeader from "./SectionHeader";

const questions = [
  {
    q: "What is Bridge?",
    a: "A native macOS control room for supervised coding-agent work. It connects local Git repositories to structured Codex, Claude Code, and OpenCode sessions, isolates concurrent tasks in worktrees, and keeps durable local history so agent activity stays inspectable and recoverable.",
  },
  {
    q: "How is this different from running an agent in a terminal?",
    a: "Bridge does not embed a provider terminal UI. Each provider's native process and event protocol is translated into one event model and rendered as structured UI. Work is isolated per worktree, delegation passes through a policy engine, and history survives a restart.",
  },
  {
    q: "Which agents does it support?",
    a: "Codex, Claude Code, and OpenCode. Each adapter reports its availability and capabilities before a session starts, and a missing CLI shows that adapter as unavailable instead of blocking startup.",
  },
  {
    q: "Do I need API keys?",
    a: "You bring your own provider access. Bridge never collects, proxies, migrates, or deletes a vendor credential. Claude models additionally need Node 18 or newer on your PATH, because Claude runs through the Agent SDK in a Node sidecar.",
  },
  {
    q: "Is it macOS only?",
    a: "For now, yes. Bridge is packaged for macOS 12 or later on Apple Silicon and is under active development.",
  },
  {
    q: "Is it open source?",
    a: "Yes. Bridge is MIT licensed, so you can read it, fork it, and ship your own build. Third-party code inside it keeps its own terms.",
  },
  {
    q: "What is the session forest?",
    a: "Bridge's append-only local conversation store. Each entry has an immutable identity, a parent entry, a semantic event kind, and visibility rules for context projection. It is local history rather than a tamper-proof or replicated evidence ledger.",
  },
  {
    q: "What does the policy engine gate?",
    a: "Capability availability, write scope, worktree isolation, concurrency, delegation depth, retry limits, per-turn budgets, and approvals. Routing and learning may rank the candidates it already allows, and nothing else may widen a permission.",
  },
  {
    q: "Can CI use it?",
    a: "Yes. A daemon owns the data directory and speaks newline-delimited JSON-RPC over a Unix socket, and the bridge exec --json one-shot attaches to it for a single call or one turn streamed as JSONL events.",
  },
  {
    q: "Where does my data live?",
    a: "In a local data directory owned by exactly one daemon, with the session forest in SQLite. Database paths, snapshots, adapter availability, and runtime health are all visible in the in-app health view.",
  },
];

export default function Faq() {
  return (
    <section className="border-t border-border">
      <div className="mx-auto max-w-6xl px-6 py-24">
        <div className="grid gap-12 lg:grid-cols-[1fr_1.6fr]">
          <SectionHeader title="FAQ" />
          <div className="reveal">
          {questions.map((item) => (
            <details key={item.q} className="group border-b border-border">
              <summary className="flex cursor-pointer list-none items-center justify-between gap-4 py-4 text-[15px] font-medium marker:content-none transition-colors hover:text-foreground/80">
                {item.q}
                <span className="grid size-6 shrink-0 place-items-center rounded-full border border-border text-faint transition-transform duration-300 group-open:rotate-45" aria-hidden="true">
                  +
                </span>
              </summary>
              <p className="pb-5 pr-8 text-[14px] leading-6 text-muted-foreground">{item.a}</p>
            </details>
          ))}
          </div>
        </div>
      </div>
    </section>
  );
}
