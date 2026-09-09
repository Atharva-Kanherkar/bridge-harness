import CallToAction from "./components/CallToAction";
import FeatureGrid from "./components/FeatureGrid";
import FeatureTabs from "./components/FeatureTabs";
import HarnessSection from "./components/HarnessSection";
import HowItWorks from "./components/HowItWorks";
import SiteFooter from "./components/SiteFooter";
import SiteHeader from "./components/SiteHeader";
import { latestReleaseUrl, latestVersion, platformLabel, repoUrl } from "./content/site";

export default function Home() {
  return (
    <div className="min-h-screen bg-background text-foreground">
      <SiteHeader />

      <main className="mx-auto flex max-w-6xl flex-col items-center px-6 pb-24 pt-16 text-center">
        <span className="text-[13px] text-muted-foreground">
          {platformLabel} · v{latestVersion}
        </span>
        <h1 className="mt-5 max-w-4xl font-display text-5xl font-semibold leading-[1.05] tracking-tight sm:text-7xl">
          Delegate the coding.
          <br />
          Keep the judgment.
        </h1>
        <p className="mt-6 max-w-2xl text-lg leading-8 text-muted-foreground">
          Bridge runs Codex, Claude Code, and OpenCode as one team. An orchestrator plans and routes, a policy engine owns every
          safety gate, and each worker lands in its own worktree with verifiable evidence.
        </p>
        <div className="mt-8 flex flex-col gap-3 sm:flex-row">
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

        <FeatureTabs />
      </main>

      <HarnessSection />
      <HowItWorks />
      <FeatureGrid />
      <CallToAction />
      <SiteFooter />
    </div>
  );
}
