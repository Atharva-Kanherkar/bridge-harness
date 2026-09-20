import type { Metadata } from "next";
import CompareTable from "../components/CompareTable";
import DownloadButtons from "../components/DownloadButtons";
import SectionHeader from "../components/SectionHeader";
import SiteFooter from "../components/SiteFooter";
import SiteHeader from "../components/SiteHeader";
import { comparedOn, edges, scope, tables } from "../content/compare";
import { issuesUrl, repoUrl } from "../content/site";

export const metadata: Metadata = {
  title: "Compare",
  description:
    "Bridge next to Conductor, Orca, Superset, and T3 Code. MIT licensed, with a policy engine, cross-family verification, and history you own.",
};

export default function Compare() {
  return (
    <div className="min-h-screen bg-background text-foreground">
      <SiteHeader />

      <main>
        <section className="border-b border-border">
          <div className="mx-auto max-w-6xl px-6 pb-16 pt-16">
            <span className="eyebrow">Comparison</span>
            <h1 className="mt-4 max-w-3xl font-display text-[2.5rem] font-semibold leading-[1.05] tracking-[-0.03em] sm:text-6xl">
              Everyone runs agents in parallel. We govern them.
            </h1>
            <p className="mt-5 max-w-2xl text-[15.5px] leading-7 text-muted-foreground">
              Bridge next to Conductor, Orca, Superset, and T3 Code. Only shipped features are in these tables.
              {" "}
              {scope}
            </p>
            <div className="mt-8">
              <DownloadButtons />
            </div>
          </div>
        </section>

        {tables.map((table) => (
          <section key={table.id} id={table.id} className="border-b border-border">
            <div className="mx-auto max-w-6xl px-6 py-20">
              <SectionHeader title={table.title} text={table.text} />
              <CompareTable table={table} />
            </div>
          </section>
        ))}

        <section className="border-b border-border">
          <div className="mx-auto max-w-6xl px-6 py-20">
            <SectionHeader
              eyebrow="The short version"
              title="Six reasons to switch"
              text="Each one is a feature you can open the app and use today."
            />
            <div className="reveal mt-10 grid gap-px overflow-hidden rounded-xl border border-border-card bg-border sm:grid-cols-2 lg:grid-cols-3">
              {edges.map((edge, index) => (
                <div key={edge.title} className="bg-background p-6 transition-colors duration-300 hover:bg-card">
                  <span className="font-mono text-[11px] text-faint">{String(index + 1).padStart(2, "0")}</span>
                  <h3 className="mt-3 text-[15px] font-semibold text-foreground">{edge.title}</h3>
                  <p className="mt-2 text-[13.5px] leading-6 text-muted-foreground">{edge.body}</p>
                </div>
              ))}
            </div>
          </div>
        </section>

        <section className="border-b border-border">
          <div className="mx-auto flex max-w-6xl flex-col items-center gap-6 px-6 py-20 text-center">
            <h2 className="max-w-2xl font-display text-[2rem] font-semibold leading-[1.1] tracking-[-0.03em]">
              Read the code, then decide.
            </h2>
            <p className="max-w-xl text-[15px] leading-7 text-muted-foreground">
              MIT licensed and free. No seat, no account, no cloud round trip.
            </p>
            <DownloadButtons />
            <a href={repoUrl} className="text-[13px] text-muted-foreground hover:text-foreground">
              Bridge on GitHub
            </a>
          </div>
        </section>

        <section>
          <div className="mx-auto max-w-6xl px-6 py-12">
            <p className="text-[12.5px] leading-6 text-muted-foreground">
              Checked against each product&rsquo;s public site and documentation in {comparedOn}. Prices are the published list
              prices for a single seat. This category moves fast. If a row is out of date,{" "}
              <a href={issuesUrl} className="underline underline-offset-4 hover:text-muted-foreground">
                open an issue
              </a>{" "}
              and we will correct it. Product names and marks belong to their owners.
            </p>
          </div>
        </section>
      </main>

      <SiteFooter />
    </div>
  );
}
