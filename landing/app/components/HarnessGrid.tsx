import HarnessMark from "./app/HarnessMark";
import SectionHeader from "./SectionHeader";

/*
 * The harnesses Bridge drives, as a logo grid. Marks are the vendors' own, drawn from
 * `app/HarnessMark` which mirrors the app's `harnessMarks.tsx`. Grok ships as a wordmark
 * because Bridge has no authentic mark for it yet — a wordmark is the honest cell, not a
 * guessed glyph.
 */
const harnesses = [
  { id: "codex", name: "Codex", via: "app-server JSON-RPC" },
  { id: "claude", name: "Claude Code", via: "Agent SDK sidecar" },
  { id: "cursor", name: "Cursor", via: "agent CLI" },
  { id: "opencode", name: "OpenCode", via: "headless server" },
  { id: "grok", name: "Grok", via: "ACP over stdio", wordmark: true },
];

/** The corner ticks a technical drawing wears. */
function Tick({ className }: { className: string }) {
  return (
    <span aria-hidden="true" className={`absolute text-[13px] leading-none text-faint-2 ${className}`}>
      +
    </span>
  );
}

export default function HarnessGrid() {
  return (
    <section className="border-t border-border">
      <div className="mx-auto max-w-6xl px-6 py-24">
        <SectionHeader
          eyebrow="Harnesses"
          title={
            <>
              Bring your own <em className="not-italic text-muted-foreground">sub.</em>
            </>
          }
          text="Bridge drives the agent CLIs you already pay for. One adapter per provider turns its native process and event protocol into a single event model, so messages, reasoning, tool calls, approvals, and diffs look the same whoever produced them."
          align="center"
        />

        <div className="reveal relative mt-14">
          <Tick className="-left-1.5 -top-1.5" />
          <Tick className="-right-1.5 -top-1.5" />
          <Tick className="-bottom-1.5 -left-1.5" />
          <Tick className="-bottom-1.5 -right-1.5" />

          <div className="grid gap-px overflow-hidden rounded-xl border border-border-card bg-border sm:grid-cols-3 lg:grid-cols-5">
            {harnesses.map(harness => (
              <div
                key={harness.id}
                className="group flex min-h-36 flex-col items-center justify-center gap-2.5 bg-background px-4 py-8 transition-colors duration-300 hover:bg-card"
              >
                {harness.wordmark ? (
                  <span className="font-display text-[26px] font-semibold leading-none tracking-[-0.03em] text-foreground">
                    {harness.name}
                  </span>
                ) : (
                  <>
                    <HarnessMark harness={harness.id} size={30} className="transition-transform duration-300 group-hover:scale-110" />
                    <span className="text-[15px] font-medium text-foreground">{harness.name}</span>
                  </>
                )}
                <span className="font-mono text-[10.5px] text-faint">{harness.via}</span>
              </div>
            ))}
          </div>
        </div>

        <p className="reveal mx-auto mt-6 max-w-2xl text-center text-[13px] leading-6 text-faint">
          Each adapter reports its availability before a session starts. A missing CLI shows as unavailable instead of blocking
          startup, and the harness id space is open — a fifth provider needs an adapter, not a rewrite.
        </p>
      </div>
    </section>
  );
}
