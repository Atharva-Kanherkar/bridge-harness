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
    id: "mission-control",
    name: "Mission Control",
    text: "Every active chat at once, each tile the real conversation with its own transcript, approvals, and composer. Watch four agents work, steer any of them without leaving the grid.",
    image: "/screens/mission-control.webp",
    alt: "Bridge Mission Control with four live agent conversations side by side, each with its own composer.",
  },
  {
    id: "agent-fleet",
    name: "Agent Fleet",
    text: "Open Codex, Claude Code, OpenCode, or Grok as terminals in the same screen. New agents split into the grid beside the others, and the layout and scrollback survive a restart.",
    image: "/screens/agent-fleet.webp",
    alt: "Bridge Agent Fleet with a shell split into a grid alongside Claude Code, Codex, and OpenCode terminals.",
  },
  {
    id: "diffs",
    name: "Review every diff",
    text: "Changes land in the pane beside the conversation that produced them: the hunks inline, per-file risk, and what the worker ran to prove it. Adopt the result into your workspace or discard it.",
    image: "/screens/diffs.webp",
    alt: "A Bridge session showing an inline diff in the transcript beside the changes dock listing four changed files with risk labels.",
  },
  {
    id: "github",
    name: "GitHub, built in",
    text: "Pull requests, review state, and check runs for the repository you are in, without a browser tab. Filter to what is ready, failing, or waiting on you.",
    image: "/screens/github.webp",
    alt: "The Bridge GitHub pane listing open pull requests with review-required, conflicts, and approved states.",
  },
  {
    id: "verify",
    name: "Nothing merges without proof",
    text: "A completion gate collects deterministic checks and scrutiny from a different harness family, records the evidence against a revision, and still waits for you to adopt.",
    image: "/screens/verify.webp",
    alt: "A Bridge verification record showing two checks passed, Claude scrutiny running, and Codex user testing pending, above Adopt and Discard actions.",
  },
];

export default function FeatureScroll() {
  return (
    <section className="overflow-hidden border-t border-border py-24">
      <div className="mx-auto max-w-6xl px-6">
        <SectionHeader
          eyebrow="Inside Bridge"
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
                <span
                  aria-hidden="true"
                  className="pointer-events-none absolute inset-0 z-10 bg-linear-to-br from-teal-400/10 via-transparent to-purple-500/10"
                />
                <Image src={feature.image} alt={feature.alt} width={1600} height={1000} unoptimized className="h-auto w-full" />
              </div>
            </div>
          </article>
        ))}
      </div>
    </section>
  );
}
