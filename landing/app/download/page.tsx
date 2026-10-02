import type { Metadata } from "next";
import SiteFooter from "../components/SiteFooter";
import SiteHeader from "../components/SiteHeader";
import { changelogPath, latestReleaseUrl, linuxDownloadPath, macDownloadPath, releasesUrl, signingIdentity } from "../content/site";

export const metadata: Metadata = {
  title: "Download",
  description: "Download the latest stable Bridge release for macOS and Linux.",
};

const requirements = [
  ["macOS", "12 or later, Apple Silicon"],
  ["Linux", "x86_64; see release notes for supported distributions and limitations"],
  ["macOS signing", "Developer ID, notarized and stapled"],
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
        <h1 className="font-display text-5xl font-semibold tracking-[-0.03em] sm:text-6xl">Get Bridge</h1>
        <p className="mt-4 text-[15px] leading-7 text-muted-foreground">
          Choose your platform. Downloads follow the latest stable release.
        </p>

        <div className="mt-8 grid gap-4 sm:grid-cols-2">
          {[
            { name: "macOS", detail: "Apple Silicon · DMG", href: macDownloadPath },
            { name: "Linux", detail: "x86_64 · AppImage", href: linuxDownloadPath },
          ].map((platform) => (
            <div key={platform.name} className="rounded-xl border border-border-card p-5">
              <h2 className="font-display text-xl font-semibold">{platform.name}</h2>
              <p className="mt-2 text-sm text-muted-foreground">{platform.detail}</p>
              <a href={platform.href} className="mt-5 block rounded-md bg-foreground px-5 py-3 text-center text-sm font-medium text-background hover:bg-foreground/90">
                Download for {platform.name}
              </a>
            </div>
          ))}
        </div>
        <p className="mt-4 text-[13px] leading-6 text-muted-foreground">
          If a platform package is not published yet, its button opens the release page.
          Linux packages, including Debian and Arch formats when available, and checksums are listed with each release.
        </p>
        <div className="mt-4 flex flex-wrap gap-6 text-[13px]">
          <a href={latestReleaseUrl} className="underline underline-offset-4 hover:no-underline">Latest stable release</a>
          <a href={releasesUrl} className="text-muted-foreground hover:text-foreground">All releases</a>
        </div>

        <h2 className="mt-16 font-display text-2xl font-semibold tracking-[-0.03em]">Requirements</h2>
        <dl className="mt-4 grid grid-cols-[auto_1fr] gap-x-6 gap-y-2 text-[14px]">
          {requirements.map(([term, value]) => (
            <div key={term} className="contents">
              <dt className="text-muted-foreground">{term}</dt>
              <dd>{value}</dd>
            </div>
          ))}
        </dl>

        <h2 className="mt-12 font-display text-2xl font-semibold tracking-[-0.03em]">Install on macOS</h2>
        <ol className="mt-4 flex flex-col gap-2">
          {steps.map((step, i) => (
            <li key={step} className="flex gap-3 text-[14px] leading-6 text-muted-foreground">
              <span className="font-mono text-[11px] text-faint">0{i + 1}</span>
              <span>{step}</span>
            </li>
          ))}
        </ol>

        <h2 className="mt-12 font-display text-2xl font-semibold tracking-[-0.03em]">Install on Linux</h2>
        <p className="mt-4 text-[14px] leading-6 text-muted-foreground">
          Make the downloaded AppImage executable in your file manager, then open it.
          For Debian or Arch packages, use your distribution&rsquo;s package manager.
          Check the release notes for required dependencies and platform limitations before installing.
        </p>

        <h2 className="mt-12 font-display text-2xl font-semibold tracking-[-0.03em]">Verify the download</h2>
        <p className="mt-4 text-[14px] leading-6 text-muted-foreground">
          Download the matching checksum file from the release page. On macOS, from the folder holding both files:
        </p>
        <pre className="mt-4 overflow-x-auto rounded-lg border border-border-card bg-code p-4 font-mono text-[12.5px] text-code-foreground">
          shasum -a 256 -c Bridge_*.dmg.sha256
        </pre>
        <p className="mt-4 text-[14px] leading-6 text-muted-foreground">
          On Linux, run <code className="font-mono text-[12.5px]">sha256sum -c</code> with the checksum filename supplied in the release.
        </p>
        <p className="mt-4 text-[13px] leading-6 text-muted-foreground">
          The macOS build is signed as {signingIdentity}. If you built from source yourself the binary is ad-hoc signed instead, and
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
