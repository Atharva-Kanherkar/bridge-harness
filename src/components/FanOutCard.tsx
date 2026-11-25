import { useState, type ReactNode } from "react";
import { ChevronRight, LoaderCircle } from "lucide-react";
import { HarnessMark } from "./harnessMarks";
import { dockPaneChord } from "../dockLayout";
import { formatShortcut } from "@/lib/platform";
import { harnessLabel } from "../utils";
import {
  WORKER_STATE_TEXT,
  fanOutCountParts,
  fanOutElapsed,
  formatWorkerClock,
  useTickingNow,
  type FanOutSummary,
} from "./workerFanOut";
import { Tip } from "@/components/ui/tooltip";
import { Button } from "@/components/ui/button";

// One card per delegation: who the orchestrator handed work to, how each of
// them is doing, and how long it has taken. The worker rows themselves are the
// card's body, unchanged, so grouping never costs the reader a row, an
// objective, a model or a way into the worker's chat.

/** The Agents pane chord, as the tooltip on the card's opener names it. */
function agentsPaneHint(): string {
  const digit = dockPaneChord("tasks");
  return digit ? `Agents pane  ${formatShortcut({ meta: true, alt: true }, digit)}` : "Agents pane";
}

export function FanOutCard({ summary, rows, readOnly = false, onOpenAgents, onStopWorkers }: {
  summary: FanOutSummary;
  /** The fan-out's worker rows, drawn as the card's body. */
  rows?: ReactNode;
  readOnly?: boolean;
  /** Opens the dock's Agents pane on the lineage. */
  onOpenAgents?: () => void;
  /** Stops the workers still being waited on, through the host's worker stop path. */
  onStopWorkers?: (childSessionIds: string[]) => Promise<void>;
}) {
  const live = summary.live && !readOnly;
  const now = useTickingNow(live);
  const [stopping, setStopping] = useState(false);
  const [error, setError] = useState<string>();
  const elapsed = fanOutElapsed(summary, now);
  const parts = fanOutCountParts(summary.counts);
  const stop = () => {
    if (!onStopWorkers || stopping) return;
    setStopping(true);
    setError(undefined);
    void onStopWorkers(summary.waiting.map(worker => worker.childSessionId))
      .catch((cause: unknown) => setError(cause instanceof Error ? cause.message : String(cause)))
      .finally(() => setStopping(false));
  };
  return <div data-fan-out role="group" aria-label={`${summary.total} ${summary.total === 1 ? "worker" : "workers"}`} className="my-1 min-w-0 overflow-hidden rounded-xl border border-border bg-card">
    <div className="flex min-w-0 items-center gap-2.5 px-3.5 py-2.5">
      <span className="flex shrink-0 -space-x-1.5" aria-hidden="true">
        {summary.marks.map(worker => <Tip label={harnessLabel(worker.harness)}><span key={worker.childSessionId} className="grid size-6 place-items-center rounded-full border border-border bg-background">
          <HarnessMark harness={worker.harness} size={12}/>
        </span></Tip>)}
        {summary.overflow > 0 && <span data-fan-out-overflow className="grid size-6 place-items-center rounded-full border border-border bg-background text-micro font-medium tabular-nums text-muted-foreground">+{summary.overflow}</span>}
      </span>
      <span className="shrink-0 text-ui font-medium text-foreground">{summary.total} {summary.total === 1 ? "worker" : "workers"}</span>
      <span className="min-w-0 truncate text-meta">
        {parts.map((part, index) => <span key={part.state}>
          {index > 0 && <span className="text-muted-foreground"> · </span>}
          <span data-fan-out-count={part.state} className={WORKER_STATE_TEXT[part.state]}>{part.text}</span>
        </span>)}
      </span>
      {elapsed !== undefined && <span data-fan-out-clock className="ml-auto shrink-0 text-caption tabular-nums text-muted-foreground">{formatWorkerClock(elapsed)}</span>}
      {onOpenAgents && <Button
        variant="quiet"
        size="icon-2xs"
        onClick={onOpenAgents}
        aria-label="Open the Agents pane"
        tooltip={agentsPaneHint()}
        className={elapsed === undefined ? "ml-auto" : undefined}
      ><ChevronRight size={14} aria-hidden="true"/></Button>}
    </div>
    {rows && <div className="border-t border-border px-3.5 py-1">{rows}</div>}
    {live && summary.waiting.length > 0 && <div data-fan-out-waiting className="flex min-w-0 items-center gap-3 border-t border-border px-3.5 py-2 text-meta text-muted-foreground">
      <span className="inline-flex min-w-0 items-center gap-1.5">
        <LoaderCircle size={12} className="shrink-0 animate-spin" aria-hidden="true"/>
        <span className="min-w-0 truncate">Waiting on {summary.waiting.map((worker, index) => <span key={worker.childSessionId}>
          {index > 0 && ", "}
          <span className="text-foreground">{worker.name}</span>
        </span>)}</span>
      </span>
      {onStopWorkers && <Button variant="quiet" size="2xs" className="shrink-0" tooltip={summary.waiting.length === 1 ? "Stop this worker" : `Stop these ${summary.waiting.length} workers`}
        disabled={stopping}
        onClick={stop}
      >{stopping ? "Stopping…" : "Stop"}</Button>}
    </div>}
    {error && <p role="alert" className="border-t border-border px-3.5 py-2 text-meta text-destructive">{error}</p>}
  </div>;
}
