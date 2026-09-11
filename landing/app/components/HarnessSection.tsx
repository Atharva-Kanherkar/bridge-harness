import HarnessMark from "./app/HarnessMark";
import SectionHeader from "./SectionHeader";

const harnesses = [
  { id: "codex", name: "Codex", ring: "border-border-card bg-muted", via: "app-server JSON-RPC", text: "Connects through the Codex app-server protocol." },
  { id: "claude", name: "Claude Code", ring: "border-harness-claude/40 bg-harness-claude/10", via: "Agent SDK sidecar", text: "Runs through the Claude Agent SDK in a Node sidecar, not a headless CLI prompt." },
  { id: "opencode", name: "OpenCode", ring: "border-harness-opencode/40 bg-harness-opencode/10", via: "headless server API", text: "Talks to the OpenCode headless server." },
  { id: "cursor", name: "Cursor", ring: "border-border-card bg-muted", via: "agent CLI", text: "Drives the Cursor agent CLI as one more worker in the fleet." },
];

export default function HarnessSection() {
  return (
    <section className="border-t border-border">
      <div className="mx-auto max-w-6xl px-6 py-24">
        <div className="grid gap-12 lg:grid-cols-[1fr_1.4fr] lg:items-start">
          <SectionHeader
            eyebrow="Harnesses"
            title={
              <>
                Bring your <em className="not-italic text-muted-foreground">own</em> harness.
              </>
            }
            text="One adapter per provider turns its native process and event protocol into a single event model, so messages, reasoning, tool calls, approvals, and diffs look the same whoever produced them."
          />
          <div className="grid gap-3">
            {harnesses.map((harness, i) => (
              <div
                key={harness.name}
                style={{ "--i": i } as React.CSSProperties}
                className="reveal group flex items-center gap-5 rounded-xl border border-border-card bg-card p-5 transition-colors hover:border-faint-2"
              >
                <span className={`grid size-12 shrink-0 place-items-center rounded-full border ${harness.ring}`}>
                  <HarnessMark harness={harness.id} size={20} />
                </span>
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-baseline gap-x-3">
                    <h3 className="font-display text-[1.25rem] font-semibold leading-none tracking-[-0.02em] text-foreground">{harness.name}</h3>
                    <span className="font-mono text-[11px] text-faint">{harness.via}</span>
                  </div>
                  <p className="mt-1.5 text-[13.5px] leading-6 text-muted-foreground">{harness.text}</p>
                </div>
              </div>
            ))}
            <p className="mt-2 text-[13px] leading-6 text-faint">
              Each adapter reports availability before a session starts. A missing CLI shows as unavailable instead of blocking startup.
            </p>
          </div>
        </div>
      </div>
    </section>
  );
}
