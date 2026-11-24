import { useCallback, useEffect, useMemo, useState } from "react";
import { CircleAlert, CircleCheck, LoaderCircle, RotateCcw, SquareArrowOutUpRight, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { bridgeApi } from "../../api";
import { relativeTime } from "../workDashboard";
import type { ReviewJob, ReviewJobStatus, ReviewSeverity } from "../../protocol/generated/protocol";
import { ReviewDraft } from "./ReviewDraft";
import { harnessLabel, SEVERITY_TEXT } from "./reviewLabels";

// The PR's review jobs, newest first, kept current by `reviews/job_updated`:
// a review started here, from another window, or while another chat was open
// shows where it is without a refresh. A draft opens its triage underneath.

const STATUS: Record<ReviewJobStatus, { label: string; tone: "running" | "waiting" | "done" | "failed" | "quiet" }> = {
  queued: { label: "Queued", tone: "running" },
  preparing: { label: "Preparing the checkout", tone: "running" },
  reviewing: { label: "Reviewing", tone: "running" },
  drafted: { label: "Draft ready", tone: "waiting" },
  posting: { label: "Posting", tone: "running" },
  posted: { label: "Posted", tone: "done" },
  failed: { label: "Failed", tone: "failed" },
  cancelled: { label: "Cancelled", tone: "quiet" },
  interrupted: { label: "Interrupted", tone: "failed" },
};

const TERMINAL: ReadonlySet<ReviewJobStatus> = new Set(["posted", "failed", "cancelled", "interrupted"]);
const RETRYABLE: ReadonlySet<ReviewJobStatus> = new Set(["failed", "interrupted", "cancelled"]);
const SEVERITIES: ReadonlyArray<ReviewSeverity> = ["critical", "major", "minor", "nit"];

/** Is the job still doing something on its own (not waiting for a person)? */
const running = (job: ReviewJob) => !TERMINAL.has(job.status) && job.status !== "drafted";

/** `1m 05s`, `42s`, `1h 03m`. */
export function formatElapsed(milliseconds: number): string {
  const seconds = Math.max(0, Math.round(milliseconds / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ${String(seconds % 60).padStart(2, "0")}s`;
  return `${Math.floor(minutes / 60)}h ${String(minutes % 60).padStart(2, "0")}m`;
}

/** Finished jobs past this many fold under "Show N more". */
const FINISHED_SHOWN = 10;

/** How long a running job has been at it, or when a finished one ended. A
 *  finished job's duration would include however long the draft waited for
 *  the person, so it says when instead. A posting job has no clock: its
 *  `startedAt` is when the review began, not the post. */
function elapsed(job: ReviewJob, now: number): string | undefined {
  if (job.status === "posting") return undefined;
  if (running(job)) {
    const from = Date.parse(job.startedAt ?? job.createdAt);
    return Number.isNaN(from) ? undefined : formatElapsed(now - from);
  }
  return job.finishedAt && !Number.isNaN(Date.parse(job.finishedAt)) ? relativeTime(job.finishedAt, new Date(now)) : undefined;
}

function upsert(jobs: ReviewJob[], job: ReviewJob): ReviewJob[] {
  const rest = jobs.filter(item => item.id !== job.id);
  return [job, ...rest].sort((a, b) => b.createdAt.localeCompare(a.createdAt));
}

function JobRow({ job, now, onChanged }: { job: ReviewJob; now: number; onChanged: (job: ReviewJob) => void }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const status = STATUS[job.status];
  const time = elapsed(job, now);
  const counts = SEVERITIES.filter(severity => job.findingCounts[severity] > 0);
  const act = async (call: () => Promise<{ job: ReviewJob }>) => {
    setBusy(true); setError(undefined);
    try { onChanged((await call()).job); }
    catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); }
    finally { setBusy(false); }
  };
  const Icon = status.tone === "running" ? LoaderCircle : status.tone === "failed" ? CircleAlert : status.tone === "done" ? CircleCheck : null;
  return <div data-review-job={job.id} className="flex flex-wrap items-center gap-x-2 gap-y-1 px-3 py-2">
    <span className="flex items-center gap-1">
      {job.harnesses.map(harness => <span key={harness} className="rounded-sm border border-border bg-background px-1.5 py-px text-caption font-medium text-foreground">{harnessLabel(harness)}</span>)}
    </span>
    <span className={cn("flex items-center gap-1 text-meta",
      status.tone === "failed" && "text-destructive", status.tone === "done" && "text-success", status.tone === "waiting" && "font-medium text-foreground",
      (status.tone === "running" || status.tone === "quiet") && "text-muted-foreground")}>
      {Icon && <Icon size={12} aria-hidden="true" className={cn(status.tone === "running" && "animate-spin motion-reduce:animate-none")} />}
      {/* Only the label is announced; the clock beside it changes every second. */}
      <span role="status">{status.label}</span>
      {time && <span className="font-mono tabular-nums text-caption text-muted-foreground">{time}</span>}
    </span>
    {(job.status === "drafted" || job.status === "posting" || job.status === "posted") && <span className="text-caption text-muted-foreground">
      {job.findingCounts.total === 0 ? "No findings" : counts.map(severity => <span key={severity} className={cn("mr-1.5", SEVERITY_TEXT[severity])}>{job.findingCounts[severity]} {severity}</span>)}
    </span>}
    {job.trigger === "push" && <span className="text-caption text-muted-foreground">On push</span>}
    {job.incrementalFromSha && <span className="text-caption text-muted-foreground" title={`Reviews the commits since ${job.incrementalFromSha}`}>Re-review since <span className="font-mono">{job.incrementalFromSha.slice(0, 7)}</span></span>}
    <span className="ml-auto flex items-center gap-1">
      {job.status === "posted" && job.postedReviewUrl && <a href={job.postedReviewUrl} target="_blank" rel="noreferrer" className="inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-meta text-foreground hover:bg-accent">
        View on GitHub <SquareArrowOutUpRight size={11} aria-hidden="true" />
      </a>}
      {RETRYABLE.has(job.status) && <button type="button" disabled={busy} onClick={() => void act(() => bridgeApi.retryReview(job.id))} className="inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-meta text-foreground hover:bg-accent disabled:opacity-50">
        <RotateCcw size={11} aria-hidden="true" /> Retry
      </button>}
      {!TERMINAL.has(job.status) && job.status !== "posting" && <button type="button" disabled={busy} onClick={() => void act(() => bridgeApi.cancelReview(job.id))} aria-label={job.status === "drafted" ? "Discard this draft" : "Cancel this review"} className="inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-meta text-muted-foreground hover:bg-accent hover:text-foreground disabled:opacity-50">
        <X size={11} aria-hidden="true" /> {job.status === "drafted" ? "Discard" : "Cancel"}
      </button>}
    </span>
    {(error || job.error) && <p role="alert" className="basis-full text-caption text-destructive">{error ?? job.error}</p>}
  </div>;
}

export function ReviewJobs({ workspaceId, number, repositoryUrl, started }: {
  workspaceId: string;
  number: number;
  repositoryUrl?: string;
  /** A job the pane just started (or got back from an idempotent start). */
  started?: ReviewJob;
}) {
  const [jobs, setJobs] = useState<ReviewJob[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string>();
  const [now, setNow] = useState(() => Date.now());
  const [showAll, setShowAll] = useState(false);
  // Every change re-reads the clock, so no time on screen is older than it.
  const update = useCallback((job: ReviewJob) => { setNow(Date.now()); setJobs(current => upsert(current, job)); }, []);

  // Subscribe, wait until the subscription is live, then list. Every change
  // after that arrives as an event, so a job's latest event is never older
  // than the list's copy of it: the list fills in only jobs no event has
  // touched.
  useEffect(() => {
    let active = true;
    setJobs([]); setLoaded(false); setError(undefined); setShowAll(false);
    let unlisten: (() => void) | undefined;
    const live = new Set<string>();
    void (async () => {
      try {
        const stop = await bridgeApi.onReviewJobUpdated(({ job }) => {
          if (!active || job.workspaceId !== workspaceId || job.number !== number) return;
          live.add(job.id);
          update(job);
        });
        if (!active) { stop(); return; }
        unlisten = stop;
        const result = await bridgeApi.listReviews(workspaceId, number);
        if (active) { setNow(Date.now()); setJobs(current => result.jobs.filter(job => !live.has(job.id)).reduce(upsert, current)); }
      } catch (cause) {
        if (active) setError(cause instanceof Error ? cause.message : String(cause));
      } finally {
        if (active) setLoaded(true);
      }
    })();
    return () => { active = false; unlisten?.(); };
  }, [workspaceId, number, update]);

  useEffect(() => {
    if (started && started.workspaceId === workspaceId && started.number === number) update(started);
  }, [started, workspaceId, number, update]);

  // Every second while a job runs (its timer), else every 30 seconds so a
  // finished job's "when" keeps ageing.
  const ticking = jobs.some(running);
  const hasJobs = jobs.length > 0;
  useEffect(() => {
    if (!hasJobs) return;
    const timer = window.setInterval(() => setNow(Date.now()), ticking ? 1000 : 30_000);
    return () => window.clearInterval(timer);
  }, [ticking, hasJobs]);

  // A job that still needs the person, or is still working, always shows;
  // finished ones past the first few fold away.
  const shown = useMemo(() => {
    if (showAll) return jobs;
    let finished = 0;
    return jobs.filter(job => !TERMINAL.has(job.status) || ++finished <= FINISHED_SHOWN);
  }, [jobs, showAll]);
  const hidden = jobs.length - shown.length;
  if (!loaded && !jobs.length) return null;
  if (!jobs.length && !error) return null;
  return <section aria-label="Reviews" className="mb-5">
    <h3 className="mb-2.5 flex items-center gap-2 text-caption font-medium text-muted-foreground">
      Reviews <span className="rounded-full bg-muted px-1.5 font-mono text-micro tabular-nums tracking-normal text-muted-foreground">{jobs.length}</span>
    </h3>
    {error && <p role="alert" className="mb-2 text-meta text-destructive">{error}</p>}
    <div className="divide-y divide-border overflow-hidden rounded-lg border border-border bg-card">
      {shown.map(job => <div key={job.id}>
        <JobRow job={job} now={now} onChanged={update} />
        {job.status === "drafted" && <ReviewDraft job={job} repositoryUrl={repositoryUrl} onPosting={update} />}
      </div>)}
    </div>
    {hidden > 0 && <button type="button" onClick={() => setShowAll(true)} className="mt-1.5 rounded-md px-2 py-0.5 text-meta text-muted-foreground hover:bg-accent hover:text-foreground">
      Show {hidden} more
    </button>}
  </section>;
}
