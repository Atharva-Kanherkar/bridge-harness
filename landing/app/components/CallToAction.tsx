import { latestReleaseUrl, nodeRequirement, platformLabel, repoUrl } from "../content/site";

export default function CallToAction() {
  return (
    <section className="border-t border-border">
      <div className="mx-auto max-w-6xl px-6 py-24 text-center">
        <div>
          <h2 className="font-display text-3xl font-semibold tracking-tight sm:text-4xl">Get Bridge</h2>
          <p className="mx-auto mt-4 max-w-xl text-[15px] leading-7 text-muted-foreground">
            Early-stage software for {platformLabel}. {nodeRequirement} Codex, Claude Code, and OpenCode stay optional.
          </p>
          <div className="mt-8 flex flex-col items-center justify-center gap-3 sm:flex-row">
            <a
              href={latestReleaseUrl}
              className="rounded-md bg-foreground px-5 py-3 text-sm font-medium text-background hover:bg-foreground/90"
            >
              Download for Mac
            </a>
            <a href={repoUrl} className="rounded-md border border-border-card px-5 py-3 text-sm font-medium hover:bg-muted">
              View on GitHub
            </a>
          </div>
        </div>
      </div>
    </section>
  );
}
