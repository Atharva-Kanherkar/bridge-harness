import CornerTicks from "./CornerTicks";
import HarnessMark from "./app/HarnessMark";
import SectionHeader from "./SectionHeader";

/*
 * The harnesses Bridge drives, as a logo grid. Marks are the vendors' own, drawn from
 * `app/HarnessMark`, which mirrors the app's `harnessMarks.tsx`.
 */
const harnesses = [
  { id: "codex", name: "Codex" },
  { id: "claude", name: "Claude Code" },
  { id: "cursor", name: "Cursor" },
  { id: "opencode", name: "OpenCode" },
  { id: "grok", name: "Grok" },
];

export default function HarnessGrid() {
  return (
    <section className="border-t border-border">
      <div className="mx-auto max-w-6xl px-6 py-24">
        <SectionHeader
          title={
            <>
              Bring your own <em className="not-italic text-muted-foreground">sub.</em>
            </>
          }
          text="Bridge drives the agent CLIs you already pay for."
          align="center"
        />

        <div className="reveal relative mt-14">
          <CornerTicks />

          <div className="grid gap-px overflow-hidden rounded-xl border border-border-card bg-border sm:grid-cols-3 lg:grid-cols-5">
            {harnesses.map(harness => (
              <div
                key={harness.id}
                className="group flex min-h-36 flex-col items-center justify-center gap-2.5 bg-background px-4 py-8 transition-colors duration-300 hover:bg-card"
              >
                <HarnessMark harness={harness.id} size={30} className="transition-transform duration-300 group-hover:scale-110" />
                <span className="text-[15px] font-medium text-foreground">{harness.name}</span>
              </div>
            ))}
          </div>
        </div>

        <p className="reveal mt-6 text-center text-[13px] text-faint">And many more harnesses coming soon.</p>
      </div>
    </section>
  );
}
