import GradientButton from "./GradientButton";
import { downloadPath, nodeRequirement, platformLabel, repoUrl } from "../content/site";

export default function CallToAction() {
  return (
    <section className="relative overflow-hidden border-t border-border">
      <div aria-hidden="true" className="pointer-events-none absolute inset-0 bg-grid mask-fade-b opacity-60" />
      <div
        aria-hidden="true"
        className="pointer-events-none absolute left-1/2 top-0 h-[420px] w-[720px] -translate-x-1/2 rounded-full bg-foreground/[0.07] blur-3xl"
      />
      <div className="relative mx-auto max-w-6xl px-6 py-32 text-center">
        <div className="reveal">
          <span className="eyebrow">Get Bridge</span>
          <h2 className="mt-5 font-display text-4xl font-semibold leading-[1.05] tracking-[-0.035em] text-foreground sm:text-6xl">
            Delegate the coding.
            <br />
            <em className="not-italic text-muted-foreground">Keep the judgment.</em>
          </h2>
          <p className="mx-auto mt-6 max-w-xl text-[15px] leading-7 text-muted-foreground">
            Early-stage software for {platformLabel}. {nodeRequirement}
          </p>
          <div className="mt-9 flex flex-col items-center justify-center gap-3 sm:flex-row">
            <GradientButton href={downloadPath} label="Download for Mac" />
            <a
              href={repoUrl}
              className="rounded-full border border-border-card px-6 py-3 text-sm font-medium transition-colors hover:border-faint-2 hover:bg-muted"
            >
              View on GitHub
            </a>
          </div>
        </div>
      </div>
    </section>
  );
}
