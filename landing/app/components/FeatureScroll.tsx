import Image from "next/image";
import SectionHeader from "./SectionHeader";

/*
 * Five features, each one a real capture of the app beside copy that sticks while its
 * screenshot scrolls past. No JavaScript: the text column is `position: sticky` inside its
 * own row, so the next feature pushes the last one out the way a page naturally would.
 *
 * Captures come from the app's preview mode at 1600×1000, 2× scale, encoded with
 * `cwebp -q 86`. Retake them in the same change as any app redesign.
 */
const features = [
  {
    id: "switch-harness",
    name: "Switch harness mid-chat",
    text: "Change model or provider inside one conversation — Codex to Claude Code to Cursor — without starting over. The provider session restarts; your history stays where it is. Nobody else lets you do this.",
    image: "/screens/switch-harness.webp",
    alt: "The Bridge model picker open in a chat, listing Codex and Claude Code models together with the note that switching restarts the provider session while history stays.",
  },
  {
    id: "memory",
    name: "Memory that carries",
    text: "Bridge remembers how you work — your conventions, your constraints, the decisions you already made — and carries them into every new conversation, on any harness. Each pin shows how confident it is and how often it was recalled.",
    image: "/screens/memory.webp",
    alt: "The Bridge memory screen listing pinned preferences, facts, decisions, and constraints with confidence and recall counts.",
  },
  {
    id: "forest",
    name: "An append-only session forest",
    text: "Every message, plan, tool call, and delegation lands in a local ledger that is never rewritten. Filter it, fork it, rewind it — the record of what an agent actually did survives the restart.",
    image: "/screens/forest.webp",
    alt: "The Bridge transcript pane showing a filtered event stream of message, plan, tool, and delegation events beside a conversation.",
  },
  {
    id: "agents",
    name: "Bring or build your own agents",
    text: "Install the coding agents you want, add plugins and skills, and define your own roles for the orchestrator to route to. The harness id space is open, so a new provider is an adapter, not a rewrite.",
    image: "/screens/agents.webp",
    alt: "The Bridge marketplace listing Claude Code, Codex, Cursor, and OpenCode with install and uninstall actions.",
  },
  {
    id: "cost",
    name: "Spend less by delegating",
    text: "Narrow work goes to a cheap tier; only the hard parts reach an expensive one. Tokens, cost, and cache savings are broken out per harness and per model, so the routing pays for itself visibly.",
    image: "/screens/cost.webp",
    alt: "The Bridge usage screen showing cost per harness, a daily cost chart, token totals, cache savings, and a per-model breakdown.",
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

            <div className="lg:py-12">
              <div className="relative overflow-hidden rounded-xl border border-border-card bg-background shadow-[0_0_0_1px_#000,0_30px_90px_-30px_rgba(0,0,0,0.9)] lg:rounded-r-none lg:border-r-0">
                <Image src={feature.image} alt={feature.alt} width={1600} height={1000} unoptimized className="h-auto w-full" />
              </div>
            </div>
          </article>
        ))}
      </div>
    </section>
  );
}
