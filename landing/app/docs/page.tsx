import type { Metadata } from "next";
import Link from "next/link";
import SiteFooter from "../components/SiteFooter";
import SiteHeader from "../components/SiteHeader";
import { docGroups } from "../content/docs";

export const metadata: Metadata = {
  title: "Docs",
  description: "Bridge design references: the session forest, delegation policy, worktrees, runtimes, and the protocol.",
};

export default function Docs() {
  return (
    <div className="min-h-screen bg-background text-foreground">
      <SiteHeader />
      <main className="mx-auto max-w-3xl px-6 pb-24 pt-16">
        <h1 className="font-display text-5xl font-semibold tracking-[-0.03em] sm:text-6xl">Docs</h1>
        <p className="mt-4 text-[15px] leading-7 text-muted-foreground">
          The design references that ship with the repository, rendered here rather than linked away.
        </p>

        <div className="mt-16 flex flex-col gap-12">
          {docGroups.map((group) => (
            <section key={group.title}>
              <h2 className="text-[11px] uppercase tracking-wider text-faint">{group.title}</h2>
              <div className="mt-4 flex flex-col">
                {group.entries.map((entry) => (
                  <Link
                    key={entry.slug}
                    href={`/docs/${entry.slug}`}
                    className="group border-t border-border py-4 first:border-t-0 first:pt-0"
                  >
                    <h3 className="text-[15px] font-medium group-hover:text-foreground/80">{entry.title}</h3>
                    <p className="mt-1 text-[13.5px] leading-6 text-muted-foreground">{entry.summary}</p>
                  </Link>
                ))}
              </div>
            </section>
          ))}
        </div>
      </main>
      <SiteFooter />
    </div>
  );
}
