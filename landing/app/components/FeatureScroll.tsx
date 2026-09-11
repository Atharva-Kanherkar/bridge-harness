import { Agents, SessionStorage, SwitchHarness, Usage } from "./features/panels";
import SectionHeader from "./SectionHeader";

/*
 * Four features, each one the mechanism running beside copy that sticks while it scrolls
 * past. The text column is `position: sticky` inside its own row, so the next feature pushes
 * the last one out the way a page naturally would; the panels animate on a scroll timeline,
 * so the visual plays as it arrives rather than sitting there as a picture.
 */
const features = [
  {
    id: "switch-harness",
    name: "Switch harness mid-chat",
    text: "Change model or provider inside one conversation, Codex to Claude Code to Cursor, without starting over. The provider session restarts; your history stays where it is. Nobody else lets you do this.",
    panel: <SwitchHarness />,
  },
  {
    id: "session-storage",
    name: "History that is never rewritten",
    text: "Every message, plan, tool call, and delegation lands in a local ledger that only ever grows. Filter it, fork it, rewind it. What an agent actually did survives the restart.",
    panel: <SessionStorage />,
  },
  {
    id: "agents",
    name: "Bring or build your own agents",
    text: "Install the coding agents you want, add plugins and skills, and define your own roles for the orchestrator to route to. The harness id space is open, so a new provider is an adapter, not a rewrite.",
    panel: <Agents />,
  },
  {
    id: "cost",
    name: "Spend less by delegating",
    text: "Narrow work goes to a cheap tier and only the hard parts reach an expensive one. Tokens, cost, and cache savings break out per harness and per model, so the routing pays for itself visibly.",
    panel: <Usage />,
  },
];

export default function FeatureScroll() {
  return (
    <section className="overflow-hidden border-t border-border py-24">
      <div className="mx-auto max-w-6xl px-6">
        <SectionHeader
          title={
            <>
              The work, <em className="not-italic text-muted-foreground">in one place.</em>
            </>
          }
          align="center"
        />

      </div>

      {/* The left column lines up with the page gutter; the capture runs off the right edge,
          so it is large enough to read instead of shrinking into half a column. */}
      <div className="mt-14 flex flex-col gap-14 pl-[max(1.5rem,calc((100vw-72rem)/2))] pr-6 lg:mt-20 lg:gap-0 lg:pr-0">
        {features.map((feature, i) => (
          <article key={feature.id} className="grid items-start gap-6 lg:grid-cols-[minmax(0,24rem)_minmax(0,1fr)] lg:gap-14">
            <div className="lg:sticky lg:top-28 lg:self-start lg:py-28">
              <span className="font-mono text-[11px] tabular-nums text-faint-2">{String(i + 1).padStart(2, "0")}</span>
              <h3 className="mt-3 font-display text-[1.75rem] font-semibold leading-tight tracking-[-0.03em] text-foreground sm:text-[2rem]">
                {feature.name}
              </h3>
              <p className="mt-4 max-w-md text-[14.5px] leading-7 text-muted-foreground">{feature.text}</p>
            </div>

            <div className="lg:py-12">{feature.panel}</div>
          </article>
        ))}
      </div>
    </section>
  );
}
