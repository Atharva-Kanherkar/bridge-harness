import Reveal from "./Reveal";

const harnesses = [
  {
    name: "Codex",
    dot: "bg-harness-codex",
    text: "Connects through the Codex app-server JSON-RPC protocol.",
  },
  {
    name: "Claude Code",
    dot: "bg-harness-claude",
    text: "Runs through the Claude Agent SDK in a Node sidecar, not a headless CLI prompt.",
  },
  {
    name: "OpenCode",
    dot: "bg-harness-opencode",
    text: "Talks to the OpenCode headless server API.",
  },
];

export default function HarnessSection() {
  return (
    <section className="border-t border-border">
      <div className="mx-auto max-w-6xl px-6 py-20">
        <Reveal>
          <h2 className="max-w-2xl font-display text-3xl font-semibold tracking-tight sm:text-4xl">Bring your own harness</h2>
          <p className="mt-4 max-w-2xl text-[15px] leading-7 text-muted-foreground">
            An adapter translates the native process and event protocol of each provider into one event model, so messages,
            reasoning, plans, tool calls, approvals, and file changes render the same way whoever produced them.
          </p>
        </Reveal>
        <Reveal className="mt-10 grid gap-4 sm:grid-cols-3">
          {harnesses.map((harness) => (
            <div key={harness.name} className="rounded-lg border border-border-card bg-card p-5">
              <div className="flex items-center gap-2">
                <span className={`inline-block size-2 shrink-0 rounded-full ${harness.dot}`} />
                <h3 className="text-[15px] font-medium">{harness.name}</h3>
              </div>
              <p className="mt-2 text-[13.5px] leading-6 text-muted-foreground">{harness.text}</p>
            </div>
          ))}
        </Reveal>
        <Reveal>
          <p className="mt-6 max-w-2xl text-[13.5px] leading-6 text-faint">
            Each adapter reports its availability and capabilities before a session starts. A missing CLI shows that adapter as
            unavailable instead of blocking startup.
          </p>
        </Reveal>
      </div>
    </section>
  );
}
