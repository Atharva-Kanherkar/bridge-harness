import type { Metadata } from "next";
import SiteFooter from "../components/SiteFooter";
import SiteHeader from "../components/SiteHeader";
import { changelogPath, dmgName, latestReleaseUrl, latestVersion, releasesUrl, signingIdentity } from "../content/site";

export const metadata: Metadata = {
  title: "Download",
  description: "Download Bridge for macOS 12 or later on Apple Silicon.",
};

const requirements = [
  ["macOS", "12 or later, Apple Silicon"],
  ["Signing", "Developer ID, notarized and stapled"],
  ["Node.js", "18 or newer on PATH, for Claude models"],
  ["Provider CLIs", "Codex, Claude Code, and OpenCode are optional"],
];

const steps = [
  "Open the disk image and drag Bridge into Applications.",
  "Launch it. A notarized build opens without Gatekeeper blocking it.",
  "Connect a local Git repository, then start a task.",
];

export default function Download() {
  return (
    <div className="min-h-screen bg-background text-foreground">
      <SiteHeader />
      <main className="mx-auto max-w-3xl px-6 pb-24 pt-16">
        <h1 className="font-display text-4xl font-semibold tracking-tight sm:text-5xl">Get Bridge</h1>
        <p className="mt-4 text-[15px] leading-7 text-muted-foreground">
          Version {latestVersion} for macOS 12 or later on Apple Silicon. Early-stage software under active development.
        </p>

        <div className="mt-8 flex flex-col gap-3 sm:flex-row sm:items-center">
          <a
            href={latestReleaseUrl}
            className="rounded-md bg-foreground px-5 py-3 text-center text-sm font-medium text-background hover:bg-foreground/90"
          >
            Download {dmgName}
          </a>
          <a href={releasesUrl} className="text-center text-[13px] text-muted-foreground hover:text-foreground">
            All releases
          </a>
        </div>

        <h2 className="mt-16 font-display text-xl font-semibold tracking-tight">Requirements</h2>
        <dl className="mt-4 grid grid-cols-[auto_1fr] gap-x-6 gap-y-2 text-[14px]">
          {requirements.map(([term, value]) => (
            <div key={term} className="contents">
              <dt className="text-muted-foreground">{term}</dt>
              <dd>{value}</dd>
            </div>
          ))}
        </dl>

        <h2 className="mt-12 font-display text-xl font-semibold tracking-tight">Install</h2>
        <ol className="mt-4 flex flex-col gap-2">
          {steps.map((step, i) => (
            <li key={step} className="flex gap-3 text-[14px] leading-6 text-muted-foreground">
              <span className="font-mono text-[11px] text-faint">0{i + 1}</span>
              <span>{step}</span>
            </li>
          ))}
        </ol>

        <h2 className="mt-12 font-display text-xl font-semibold tracking-tight">Verify the download</h2>
        <p className="mt-4 text-[14px] leading-6 text-muted-foreground">
          Every release ships a checksum beside the disk image. From the folder holding both files:
        </p>
        <pre className="mt-4 overflow-x-auto rounded-lg border border-border-card bg-code p-4 font-mono text-[12.5px] text-code-foreground">
          shasum -a 256 -c {dmgName}.sha256
        </pre>
        <p className="mt-4 text-[13px] leading-6 text-faint">
          The build is signed as {signingIdentity}. If you built from source yourself the binary is ad-hoc signed instead, and
          macOS will ask you to confirm before opening it.
        </p>

        <p className="mt-12 text-[13px] text-muted-foreground">
          See what changed in{" "}
          <a href={changelogPath} className="text-foreground underline underline-offset-4 hover:no-underline">
            the changelog
          </a>
          .
        </p>
      </main>
      <SiteFooter />
    </div>
  );
}
