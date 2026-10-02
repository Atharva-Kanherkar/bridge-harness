import { Browser, History, Memory, Parallel, SwitchHarness, Usage, Verify } from "./features/panels";
import SectionHeader from "./SectionHeader";

/*
 * Seven features, each one the mechanism running beside copy that sticks while it scrolls
 * past. The text column is `position: sticky` inside its own row, so the next feature pushes
 * the last one out the way a page naturally would; each panel loops a
 * short scene of the app at work while it is on screen.
 */
const features = [
  {
    id: "parallel",
    name: "Every agent on its own branch",
    text: "Run as many agents as you like on one repo. Every chat gets its own Git worktree and branch, so they never step on each other's files. Mission Control shows them side by side and flags the one waiting on you.",
    panel: <Parallel />,
  },
  {
    id: "switch-harness",
    name: "Switch harness mid-chat",
    text: "Change model or provider inside one conversation, Codex to Claude Code to Cursor, without starting over. The provider session restarts; your history stays where it is. Nobody else lets you do this.",
    panel: <SwitchHarness />,
  },
  {
    id: "browser",
    name: "A real browser, signed in as you",
    text: "When an agent needs a site you're logged into, it asks. Approve it and the agent drives a throwaway copy of your session: it can click through a deploy preview or a dashboard, and you can take over at any point. The copy is thrown away when the turn ends.",
    panel: <Browser />,
  },
  {
    id: "history",
    name: "Nothing gets lost",
    text: "Every message, plan, tool call, and delegation lands in a local log that only ever grows. When the context window fills, Bridge compacts the model's view and leaves the log alone, so a restart picks up from a verified checkpoint instead of a blank page.",
    panel: <History />,
  },
  {
    id: "verify",
    name: "No agent grades its own homework",
    text: "Tests run first. Then a reviewer from a different model family has to sign off, and a review from the family that wrote the code is thrown out. Claude's work gets checked by Codex, and the other way round.",
    panel: <Verify />,
  },
  {
    id: "memory",
    name: "Tell it once",
    text: "Preferences, decisions, and constraints get saved as you work and come back in any later chat, whichever agent is running and whichever repo it's in. Every reply shows which memories it used.",
    panel: <Memory />,
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
              Ship more, <em className="not-italic text-muted-foreground">babysit less.</em>
            </>
          }
          align="center"
        />

      </div>

      {/* The left column lines up with the page gutter; the capture runs off the right edge,
          so it is large enough to read instead of shrinking into half a column. */}
      <div className="mt-14 flex flex-col gap-14 pl-[max(1.5rem,calc((100vw-72rem)/2))] pr-6 lg:mt-20 lg:gap-0 lg:pr-0">
        {features.map(feature => (
          <article
            key={feature.id}
            id={feature.id}
            className="grid scroll-mt-24 items-start gap-6 lg:grid-cols-[minmax(0,24rem)_minmax(0,1fr)] lg:gap-14"
          >
            <div className="lg:sticky lg:top-28 lg:self-start lg:py-16">
              <h3 className="font-display text-[1.75rem] font-semibold leading-tight tracking-[-0.03em] text-foreground sm:text-[2rem]">
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
