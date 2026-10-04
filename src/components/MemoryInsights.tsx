import { useEffect, useState } from "react";
import { LoaderCircle, Sparkles } from "lucide-react";
import { cn } from "@/lib/utils";
import { bridgeApi } from "../api";
import { harnessLabel } from "../utils";
import type { MemoryInsightsResult } from "../types";
import { CARD, relative, rise, Themes, TONE } from "./UsageInsights";

// The Memory Insights tab: the same harness-written report as usage Insights,
// over the memory ledger. Figures are Bridge's own (recall counts, budget);
// the words are the model's, and the footer says which is which. Opening the
// tab only reads the stored report. Running the model sends memory text to the
// user's harness, so it is always an explicit click.

const LOADING_STEPS = ["Reading your memories", "Counting recalls", "Asking your harness to write it up"];

export function MemoryInsights({ onError }: { onError: (message: string) => void }) {
  const [result, setResult] = useState<MemoryInsightsResult | null>(null);
  const [loading, setLoading] = useState(true);
  const [step, setStep] = useState(0);

  const load = (refresh: boolean) => {
    setLoading(true);
    setStep(0);
    const ticker = window.setInterval(() => setStep(current => current + 1), refresh ? 4_000 : 1_000);
    bridgeApi.memoryInsights(refresh)
      .then(setResult)
      .catch(error => {
        const message = error instanceof Error ? error.message : String(error);
        setResult({ status: "failed", detail: message });
        onError(message);
      })
      .finally(() => { window.clearInterval(ticker); setLoading(false); });
  };

  useEffect(() => { load(false); }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const report = result?.report ?? null;
  const analyseButton = <button type="button" onClick={() => load(true)} disabled={loading} className="inline-flex h-8 shrink-0 items-center gap-2 rounded-lg border border-border px-3 text-caption text-foreground transition-colors hover:bg-accent disabled:opacity-40">
    {loading ? <LoaderCircle size={13} className="animate-spin" aria-hidden="true" /> : <Sparkles size={13} aria-hidden="true" />}{report ? "Analyse again" : "Analyse my memory"}
  </button>;

  if (loading && !report) {
    return <div role="status" aria-live="polite" className="flex items-center gap-2 py-6 text-caption text-muted-foreground">
      <LoaderCircle size={13} className="animate-spin" aria-hidden="true" />{LOADING_STEPS[Math.min(step, LOADING_STEPS.length - 1)]}…
    </div>;
  }

  if (!report) {
    const status = result?.status ?? "empty";
    return <div className={cn(CARD, "flex flex-col items-start gap-3 py-8")}>
      <h2 className="font-display text-lg font-semibold text-foreground">{status === "unavailable" ? "Insights need a harness" : status === "failed" ? "The analysis did not finish" : "Nothing analysed yet"}</h2>
      <p className="max-w-xl text-caption text-muted-foreground">{result?.detail ?? "Bridge reads your active memories and how often each reached a prompt, then asks your harness to write up what it sees. The memory text goes only to the harness you already use. Nothing is stored except the report."}</p>
      {status !== "unavailable" && analyseButton}
    </div>;
  }

  const { stats } = report;
  const usedPct = stats.activeRecords > 0 ? Math.round((stats.recalledRecords / stats.activeRecords) * 100) : null;
  return <div className="space-y-4">
    <section className={cn(CARD, rise(0).className)} style={rise(0).style} aria-label="Summary">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0 max-w-2xl">
          <h2 className="font-display text-xl font-semibold tracking-tight text-foreground">{report.headline}</h2>
          <p className="mt-2 text-ui leading-relaxed text-muted-foreground">{report.summary}</p>
        </div>
        {analyseButton}
      </div>
      <dl className="mt-4 grid grid-cols-3 gap-3">
        {[
          ["Active", String(stats.activeRecords)],
          ["Used in 14 days", usedPct === null ? "n/a" : `${usedPct}%`],
          ["Packets with memory", `${stats.packetsWithMemories} of ${stats.packets}`],
        ].map(([label, value]) => <div key={label}>
          <dt className="text-[11px] text-muted-foreground">{label}</dt>
          <dd className="font-display text-xl font-semibold tabular-nums text-foreground">{value}</dd>
        </div>)}
      </dl>
      <p className="mt-3 text-[11px] text-muted-foreground">
        {result?.harness && result?.model ? <>Written by {harnessLabel(result.harness)} · <span className="font-mono">{result.model}</span> · </> : null}
        {result?.generatedAt ? `${relative(result.generatedAt)} · ` : ""}{report.memoriesAnalysed} memories read. The figures are Bridge's own; the words are the model's.
      </p>
    </section>

    {report.highlights.length > 0 && <section className="grid gap-3 sm:grid-cols-3" aria-label="Highlights">
      {report.highlights.map((item, index) => <article key={item.title} className={cn(CARD, "py-3", rise(index + 1).className)} style={rise(index + 1).style}>
        <div className="mb-1 flex items-center gap-1.5 text-[11px] text-muted-foreground"><span className={cn("size-1.5 rounded-full", TONE[item.tone].dot)} aria-hidden="true" />{TONE[item.tone].label}</div>
        <h3 className="text-ui font-medium text-foreground">{item.title}</h3>
        <p className="mt-1 text-caption leading-relaxed text-muted-foreground">{item.detail}</p>
      </article>)}
    </section>}

    {report.themes.length > 0 && <section className={cn(CARD, rise(4).className)} style={rise(4).style} aria-label="What your memories cover">
      <h3 className="mb-3 text-ui font-medium text-foreground">What your memories cover</h3>
      <Themes report={{ themes: report.themes }} />
    </section>}

    <section className={cn(CARD, rise(5).className)} style={rise(5).style} aria-label="Recommendations">
      <h3 className="mb-3 text-ui font-medium text-foreground">Try next</h3>
      <ol className="space-y-2.5">
        {report.recommendations.map((item, index) => <li key={item} className="flex gap-3 text-ui text-foreground"><span className="shrink-0 font-mono text-caption tabular-nums text-muted-foreground">{String(index + 1).padStart(2, "0")}</span><span className="leading-relaxed">{item}</span></li>)}
        {report.recommendations.length === 0 && <li className="text-caption text-muted-foreground">No recommendations this time.</li>}
      </ol>
    </section>
  </div>;
}
