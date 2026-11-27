import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { CircleAlert, CircleCheck, RotateCcw, SquareArrowOutUpRight, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { bridgeApi } from "../../api";
import { friendlyError } from "../../errors";
import { relativeTime } from "../workDashboard";
import type { ReviewJob, ReviewJobStatus } from "../../protocol/generated/protocol";
import { ReviewDraft } from "./ReviewDraft";
import { harnessLabel, REVIEW_STATUS, SEVERITIES, SEVERITY_TEXT } from "./reviewLabels";
import { Tip } from "@/components/ui/tooltip";
import { Button } from "@/components/ui/button";
import { SkeletonDot } from "@/components/ui/skeleton";

// The PR's review jobs, newest first, kept current by `reviews/job_updated`:
// a review started here, from another window, or while another chat was open
// shows where it is without a refresh. A draft opens its triage underneath.

const STATUS = REVIEW_STATUS;

const TERMINAL: ReadonlySet<ReviewJobStatus> = new Set(["posted", "failed", "cancelled", "interrupted"]);
const RETRYABLE: ReadonlySet<ReviewJobStatus> = new Set(["failed", "interrupted", "cancelled"]);

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

/** How long the status line keeps showing that the newest review failed. */
const FAILURE_SHOWN_MS = 24 * 60 * 60 * 1000;

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
  const act = async (call: () => Promise<{ job: ReviewJob }>, failed: string) => {
    setBusy(true); setError(undefined);
    try { onChanged((await call()).job); }
    catch (cause) { setError(friendlyError(cause, failed)); }
    finally { setBusy(false); }
  };
  const Icon = status.tone === "failed" ? CircleAlert : status.tone === "done" ? CircleCheck : null;
  return <div data-review-job={job.id} className="flex flex-wrap items-center gap-x-2 gap-y-1 px-3 py-2">
    <span className="flex items-center gap-1">
      {job.harnesses.map(harness => <span key={harness} className="rounded-sm border border-border bg-background px-1.5 py-px text-caption font-medium text-foreground">{harnessLabel(harness)}</span>)}
    </span>
    <span className={cn("flex items-center gap-1 text-meta",
      status.tone === "failed" && "text-destructive", status.tone === "done" && "text-success", status.tone === "waiting" && "font-medium text-foreground",
      (status.tone === "running" || status.tone === "quiet") && "text-muted-foreground")}>
      {status.tone === "running" ? <SkeletonDot /> : Icon && <Icon size={12} aria-hidden="true" />}
      {/* Only the label is announced; the clock beside it changes every second. */}
      <span role="status">{status.label}</span>
      {time && <span className="font-mono tabular-nums text-caption text-muted-foreground">{time}</span>}
    </span>
    {(job.status === "drafted" || job.status === "posting" || job.status === "posted") && <span className="text-caption text-muted-foreground">
      {job.findingCounts.total === 0 ? "No findings" : counts.map(severity => <span key={severity} className={cn("mr-1.5", SEVERITY_TEXT[severity])}>{job.findingCounts[severity]} {severity}</span>)}
    </span>}
    {job.trigger === "push" && <span className="text-caption text-muted-foreground">On push</span>}
    {job.incrementalFromSha && <Tip label={`Reviews the commits since ${job.incrementalFromSha}`}><span className="text-caption text-muted-foreground">Re-review since <span className="font-mono">{job.incrementalFromSha.slice(0, 7)}</span></span></Tip>}
    <span className="ml-auto flex items-center gap-1">
      {job.status === "posted" && job.postedReviewUrl && <a href={job.postedReviewUrl} target="_blank" rel="noreferrer" className="inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-meta text-foreground hover:bg-accent">
        View on GitHub <SquareArrowOutUpRight size={11} aria-hidden="true" />
      </a>}
      {RETRYABLE.has(job.status) && <Button variant="quiet" size="2xs" disabled={busy} onClick={() => void act(() => bridgeApi.retryReview(job.id), "Couldn't retry the review.")}>
        <RotateCcw size={11} aria-hidden="true" /> Retry
      </Button>}
      {!TERMINAL.has(job.status) && job.status !== "posting" && <Button variant="quiet" size="2xs" disabled={busy} onClick={() => void act(() => bridgeApi.cancelReview(job.id), job.status === "drafted" ? "Couldn't discard the draft." : "Couldn't cancel the review.")} aria-label={job.status === "drafted" ? "Discard this draft" : "Cancel this review"}>
        <X size={11} aria-hidden="true" /> {job.status === "drafted" ? "Discard" : "Cancel"}
      </Button>}
    </span>
    {(error || job.error) && <p role="alert" className="basis-full text-caption text-destructive">{error ?? friendlyError(job.error, "The review failed.")}</p>}
  </div>;
}

/** A PR's review jobs, newest first, live. One subscription per PR detail:
 *  the status line under the actions and the Reviews section share it. */
export function usePullRequestReviews(workspaceId: string, number: number, started?: ReviewJob) {
  const [jobs, setJobs] = useState<ReviewJob[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string>();
  const [now, setNow] = useState(() => Date.now());
  // Every change re-reads the clock, so no time on screen is older than it.
  const update = useCallback((job: ReviewJob) => { setNow(Date.now()); setJobs(current => upsert(current, job)); }, []);

  // Subscribe, wait until the subscription is live, then list. Every change
  // after that arrives as an event, so a job's latest event is never older
  // than the list's copy of it: the list fills in only jobs no event has
  // touched.
  useEffect(() => {
    let active = true;
    setJobs([]); setLoaded(false); setError(undefined);
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
        if (active) setError(friendlyError(cause, "Couldn't load the reviews."));
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

  return { jobs, loaded, error, now, update };
}

export type PullRequestReviews = ReturnType<typeof usePullRequestReviews>;

export function ReviewJobs({ workspaceId, number, repositoryUrl, started }: {
  workspaceId: string;
  number: number;
  repositoryUrl?: string;
  /** A job the pane just started (or got back from an idempotent start). */
  started?: ReviewJob;
}) {
  const reviews = usePullRequestReviews(workspaceId, number, started);
  return <ReviewJobsView reviews={reviews} repositoryUrl={repositoryUrl} />;
}

/** The Reviews section: every job of the PR, a draft's triage under it. */
export function ReviewJobsView({ reviews, repositoryUrl, reveal, sectionRef }: {
  reviews: PullRequestReviews;
  repositoryUrl?: string;
  /** Changes when the status line asks to show this section. */
  reveal?: number;
  sectionRef?: React.RefObject<HTMLElement>;
}) {
  const { jobs, loaded, error, now, update } = reviews;
  const [showAll, setShowAll] = useState(false);
  const ownRef = useRef<HTMLElement>(null);
  const ref = sectionRef ?? ownRef;
  useEffect(() => {
    if (!reveal) return;
    // After the tab switch has mounted the section.
    const frame = window.requestAnimationFrame(() => {
      ref.current?.scrollIntoView({ block: "start", behavior: "smooth" });
      ref.current?.focus({ preventScroll: true });
    });
    return () => window.cancelAnimationFrame(frame);
  }, [reveal, ref]);

  // A job that still needs the person, or is still working, always shows;
  // finished ones past the first few fold away.
  const shown = useMemo(() => {
    if (showAll) return jobs;
    const kept: typeof jobs = [];
    let finished = 0;
    for (const job of jobs) {
      if (!TERMINAL.has(job.status)) kept.push(job);
      else if (++finished <= FINISHED_SHOWN) kept.push(job);
    }
    return kept;
  }, [jobs, showAll]);
  const hidden = jobs.length - shown.length;
  if (!loaded && !jobs.length) return null;
  if (!jobs.length && !error) return null;
  return <section ref={ref} tabIndex={-1} aria-label="Reviews" className="mb-5 scroll-mt-14 outline-none">
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
    {hidden > 0 && <Button variant="quiet" size="2xs" className="mt-1.5" onClick={() => setShowAll(true)}>
      Show {hidden} more
    </Button>}
  </section>;
}

/** What the PR's reviews are doing, right under its actions and on every
 *  tab: a start never goes unanswered, and a ready draft or a failure is
 *  one click from where it is handled. */
export function ReviewStatusLine({ reviews, onShow }: { reviews: PullRequestReviews; onShow: () => void }) {
  const { jobs, now, update } = reviews;
  const [busy, setBusy] = useState(false);
  const active = jobs.filter(job => running(job));
  const drafts = jobs.filter(job => job.status === "drafted");
  // The newest job decides whether a failure is still news.
  const latest = jobs[0];
  // A failure stays news for a day, until another review starts.
  const failedAt = latest ? Date.parse(latest.finishedAt ?? latest.createdAt) : NaN;
  const failed = latest && STATUS[latest.status].tone === "failed" && now - failedAt < FAILURE_SHOWN_MS ? latest : undefined;
  if (!active.length && !drafts.length && !failed) return null;
  const who = (job: ReviewJob) => job.harnesses.map(harnessLabel).join(" and ");
  const lines: { key: string; tone: "running" | "waiting" | "failed"; text: string; time?: string; action: string; onAction: () => void }[] = [];
  for (const job of active) {
    const status = job.status === "queued" ? "Queued: waits for a free review slot" : job.status === "preparing" ? `Preparing the checkout for ${who(job)}` : job.status === "posting" ? "Posting the review to GitHub" : `${who(job)} ${job.harnesses.length > 1 ? "are" : "is"} reviewing`;
    lines.push({ key: job.id, tone: "running", text: status, time: elapsed(job, now), action: "Show", onAction: onShow });
  }
  const findings = (total: number) => total === 0 ? "no findings" : `${total} finding${total === 1 ? "" : "s"} to triage`;
  if (drafts.length === 1) {
    const [job] = drafts;
    lines.push({ key: job.id, tone: "waiting", text: `Draft ready from ${who(job)}: ${findings(job.findingCounts.total)}`, action: "Triage", onAction: onShow });
  } else if (drafts.length > 1) {
    // One line for every waiting draft: the Reviews section lists them.
    const total = drafts.reduce((sum, job) => sum + job.findingCounts.total, 0);
    lines.push({ key: "drafts", tone: "waiting", text: `${drafts.length} drafts ready: ${findings(total)}`, action: "Triage", onAction: onShow });
  }
  if (failed && !active.length) {
    // The reason is the job's raw error made plain; a reason that only reads
    // like code is dropped rather than shown.
    const reason = failed.error ? friendlyError(failed.error, "") : "";
    lines.push({
      key: failed.id, tone: "failed", text: `Review with ${who(failed)} ${failed.status === "interrupted" ? "was interrupted" : "failed"}.${reason ? ` ${reason}` : ""}`, action: "Retry",
      onAction: () => {
        setBusy(true);
        bridgeApi.retryReview(failed.id).then(result => update(result.job), () => onShow()).finally(() => setBusy(false));
      },
    });
  }
  return <div data-review-status className="mt-2 grid gap-1" aria-live="polite">
    {lines.map(line => <div key={line.key} className={cn("flex items-center gap-2 rounded-md border px-2.5 py-1.5 text-meta",
      line.tone === "failed" ? "border-destructive/25 bg-destructive/10 text-destructive" : line.tone === "waiting" ? "border-border bg-card text-foreground" : "border-border bg-muted/40 text-muted-foreground")}>
      {line.tone === "running" ? <SkeletonDot />
        : line.tone === "failed" ? <CircleAlert size={12} aria-hidden="true" className="shrink-0" /> : <CircleCheck size={12} aria-hidden="true" className="shrink-0 text-success" />}
      <Tip label={line.text}><span className="min-w-0 flex-1 truncate">{line.text}</span></Tip>
      {line.time && <span className="shrink-0 font-mono text-caption tabular-nums">{line.time}</span>}
      <Button variant="quiet" size="2xs" className="shrink-0" disabled={line.tone === "failed" && busy} loading={line.tone === "failed" && busy} onClick={line.onAction}>{line.action}</Button>
    </div>)}
  </div>;
}

/** What starting `harnesses` again would hit: a job of this PR with that very
 *  set that is still live (a start returns it rather than queue another). */
export function activeJobFor(jobs: ReviewJob[], harnesses: string[]): ReviewJob | undefined {
  const key = [...harnesses].sort().join(",");
  return jobs.find(job => !TERMINAL.has(job.status) && [...job.harnesses].sort().join(",") === key);
}

/** Reviews of the PR still running on their own. */
export const runningCount = (jobs: ReviewJob[]) => jobs.filter(running).length;
