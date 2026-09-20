import type { Metadata } from "next";
import SiteFooter from "../components/SiteFooter";
import SiteHeader from "../components/SiteHeader";
import { releases } from "../content/changelog";
import { releaseNotesUrl, releasesUrl } from "../content/site";

export const metadata: Metadata = {
  title: "Changelog",
  description: "New features, fixes, and release notes for Bridge.",
};

export default function Changelog() {
  return (
    <div className="min-h-screen bg-background text-foreground">
      <SiteHeader />
      <main className="mx-auto max-w-3xl px-6 pb-24 pt-16">
        <h1 className="font-display text-5xl font-semibold tracking-[-0.03em] sm:text-6xl">Changelog</h1>
        <p className="mt-4 text-[15px] leading-7 text-muted-foreground">
          What shipped in each Bridge release. Every build is Developer ID signed, notarized, and stapled.
        </p>

        <div className="mt-16 flex flex-col gap-14">
          {releases.map((release) => (
            <article key={release.version}>
              <div className="flex items-baseline gap-3">
                <h2 className="font-display text-[1.75rem] font-semibold tracking-[-0.03em]">{release.version}</h2>
                <span className="text-[13px] text-muted-foreground">{release.date}</span>
              </div>
              <h3 className="mt-3 text-base font-medium">{release.headline}</h3>
              <ul className="mt-4 flex flex-col gap-2">
                {release.bullets.map((bullet) => (
                  <li key={bullet} className="flex gap-3 text-[14px] leading-6 text-muted-foreground">
                    <span className="mt-2.5 size-1 shrink-0 rounded-full bg-faint-2" aria-hidden="true" />
                    <span>{bullet}</span>
                  </li>
                ))}
              </ul>
              <a
                href={releaseNotesUrl(release.version)}
                className="mt-4 inline-block text-[13px] text-muted-foreground hover:text-foreground"
              >
                Release notes
              </a>
            </article>
          ))}
        </div>

        <a href={releasesUrl} className="mt-16 inline-block text-[13px] text-muted-foreground hover:text-foreground">
          All releases on GitHub
        </a>
      </main>
      <SiteFooter />
    </div>
  );
}
