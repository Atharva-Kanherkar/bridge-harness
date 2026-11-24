import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { CircleAlert, CircleCheck, LoaderCircle, RefreshCw, Search, SquareArrowOutUpRight, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { SCREEN_CONTENT, ScreenHeading } from "@/components/ui/screen";
import { bridgeApi } from "../../api";
import { relativeTime } from "../workDashboard";
import { formatTokens, formatUsd } from "../../usageReport";
import { GithubMarkdown } from "../github/GithubMarkdown";
import { ReviewDraft } from "../github/ReviewDraft";
import { formatElapsed } from "../github/ReviewJobs";
import { harnessLabel, REVIEW_STATUS, SEVERITIES, SEVERITY_TEXT, type ReviewStatusTone } from "../github/reviewLabels";
import type {
  ReviewHarnessStats, ReviewJob, ReviewJobDetail, ReviewOverview, ReviewOverviewFinding, ReviewOverviewStats, ReviewSeverity,
} from "../../protocol/generated/protocol";

// Every PR review Bridge ran, across repositories: what is waiting for the
// person, what is running, the numbers for a period, how each reviewer does,
// and every review and finding. A row opens the same triage as the GitHub
// pane. Kept current by `reviews/job_updated`.

type Period = 7 | 30 | "all";
type JobFilter = "all" | "waiting" | "running" | "posted" | "failed";
type Tab = "reviews" | "findings";

const PERIODS: { value: Period; label: string }[] = [{ value: 7, label: "7 days" }, { value: 30, label: "30 days" }, { value: "all", label: "All" }];
const JOB_FILTERS: { value: JobFilter; label: string }[] = [
  { value: "all", label: "All" }, { value: "waiting", label: "Needs you" }, { value: "running", label: "Running" }, { value: "posted", label: "Posted" }, { value: "failed", label: "Failed" },
];
const CARD = "u-surface rounded-2xl";
const EYEBROW = "text-caption font-medium text-muted-foreground";
/** Coalesce bursts of job updates (a review emits one per step) into one read. */
const REFETCH_DELAY_MS = 400;

const SEVERITY_FILL: Record<ReviewSeverity, string> = {
  critical: "bg-destructive",
  major: "bg-warning",
  minor: "bg-info",
  nit: "bg-muted-foreground/40",
};

const tone = (job: ReviewJob): ReviewStatusTone => REVIEW_STATUS[job.status].tone;
const matchesFilter = (job: ReviewJob, filter: JobFilter) =>
  filter === "all" || (filter === "waiting" ? job.status === "drafted" : filter === "posted" ? job.status === "posted" : filter === "failed" ? tone(job) === "failed" : tone(job) === "running");
const repoOf = (item: { repoOwner?: string | null; repoName?: string | null }) => item.repoOwner && item.repoName ? `${item.repoOwner}/${item.repoName}` : undefined;
const repositoryUrl = (job: ReviewJob) => { const repo = repoOf(job); return repo ? `https://github.com/${repo}` : undefined; };

/** `(kept + edited) / decided`, or undefined when nothing was decided: no
 *  decisions is not a 0% kept rate. */
export function keptRate(stats: Pick<ReviewOverviewStats, "kept" | "edited" | "dropped">): number | undefined {
  const decided = stats.kept + stats.edited + stats.dropped;
  return decided === 0 ? undefined : (stats.kept + stats.edited) / decided;
}

const percent = (value: number | undefined) => value === undefined ? "–" : `${Math.round(value * 100)}%`;
const seconds = (value: number | null | undefined) => value == null ? "–" : formatElapsed(value * 1000);

function Segmented<T extends string | number>({ label, value, options, onChange }: { label: string; value: T; options: { value: T; label: string }[]; onChange: (value: T) => void }) {
  return <div role="radiogroup" aria-label={label} className="u-segmented">
    {options.map(option => <button key={String(option.value)} type="button" role="radio" aria-checked={option.value === value} data-active={option.value === value} className="u-segmented-item" onClick={() => onChange(option.value)}>{option.label}</button>)}
  </div>;
}

function Tile({ label, value, sub, children }: { label: string; value: string; sub?: ReactNode; children?: ReactNode }) {
  return <div data-tile={label} className={cn(CARD, "flex min-w-0 flex-col gap-1 p-4")}>
    <p className={EYEBROW}>{label}</p>
    <p className="font-display text-title font-semibold tabular-nums tracking-tight text-foreground">{value}</p>
    {sub && <p className="text-caption text-muted-foreground">{sub}</p>}
    {children}
  </div>;
}

/** Findings by severity: one bar, 2px gaps, labelled below in text ink. */
function SeverityBar({ counts }: { counts: ReviewOverviewStats["findings"] }) {
  const present = SEVERITIES.filter(severity => counts[severity] > 0);
  if (counts.total === 0) return null;
  return <div className="mt-1.5">
    <div className="flex h-1.5 gap-0.5 overflow-hidden rounded-full" role="img" aria-label={present.map(severity => `${counts[severity]} ${severity}`).join(", ")}>
      {present.map(severity => <span key={severity} className={cn("h-full first:rounded-l-full last:rounded-r-full", SEVERITY_FILL[severity])} style={{ flexGrow: counts[severity] }} />)}
    </div>
    <p className="mt-1.5 flex flex-wrap gap-x-2 text-caption text-muted-foreground">
      {present.map(severity => <span key={severity}><span className={SEVERITY_TEXT[severity]}>●</span> {counts[severity]} {severity}</span>)}
    </p>
  </div>;
}

function StatusLabel({ job, now }: { job: ReviewJob; now: number }) {
  const status = REVIEW_STATUS[job.status];
  const Icon = status.tone === "running" ? LoaderCircle : status.tone === "failed" ? CircleAlert : status.tone === "done" ? CircleCheck : null;
  const since = status.tone === "running" && job.status !== "posting" ? Date.parse(job.startedAt ?? job.createdAt) : NaN;
  return <span className={cn("inline-flex items-center gap-1 whitespace-nowrap text-meta",
    status.tone === "failed" && "text-destructive", status.tone === "done" && "text-success", status.tone === "waiting" && "font-medium text-foreground",
    (status.tone === "running" || status.tone === "quiet") && "text-muted-foreground")}>
    {Icon && <Icon size={12} aria-hidden="true" className={cn(status.tone === "running" && "animate-spin motion-reduce:animate-none")} />}
    {status.label}
    {!Number.isNaN(since) && <span className="font-mono text-caption tabular-nums text-muted-foreground">{formatElapsed(now - since)}</span>}
  </span>;
}

function Reviewers({ harnesses }: { harnesses: string[] }) {
  return <span className="inline-flex flex-wrap gap-1">
    {harnesses.map(harness => <span key={harness} className="rounded-sm border border-border bg-background px-1.5 py-px text-caption font-medium text-foreground">{harnessLabel(harness)}</span>)}
  </span>;
}

function Counts({ job }: { job: ReviewJob }) {
  if (!["drafted", "posting", "posted"].includes(job.status)) return <span className="text-caption text-muted-foreground">–</span>;
  if (job.findingCounts.total === 0) return <span className="text-caption text-muted-foreground">None</span>;
  return <span className="text-caption">{SEVERITIES.filter(severity => job.findingCounts[severity] > 0).map(severity =>
    <span key={severity} className={cn("mr-1.5 whitespace-nowrap", SEVERITY_TEXT[severity])}>{job.findingCounts[severity]} {severity}</span>)}</span>;
}

function PrName({ job }: { job: Pick<ReviewJob, "number" | "prTitle" | "repoOwner" | "repoName"> }) {
  return <span className="min-w-0 flex-1">
    <span className="block truncate text-meta font-medium text-foreground" title={job.prTitle ?? undefined}>{job.prTitle ?? `Pull request #${job.number}`}</span>
    <span className="block truncate font-mono text-caption text-muted-foreground">{repoOf(job) ?? "Repository"} #{job.number}</span>
  </span>;
}

function JobList({ title, jobs, empty, now, onOpen }: { title: string; jobs: ReviewJob[]; empty: string; now: number; onOpen: (job: ReviewJob) => void }) {
  return <section aria-label={title} className={cn(CARD, "min-w-0 p-4")}>
    <h2 className="mb-2 flex items-baseline gap-2 text-ui font-semibold text-foreground">{title}<span className="text-caption font-normal tabular-nums text-muted-foreground">{jobs.length}</span></h2>
    {jobs.length === 0
      ? <p className="text-meta text-muted-foreground">{empty}</p>
      : <ul className="-mx-2 grid grid-cols-1 gap-0.5">{jobs.map(job => <li key={job.id}>
          <button type="button" data-open-job={job.id} onClick={() => onOpen(job)} className="flex w-full items-center gap-3 rounded-lg px-2 py-1.5 text-left transition-colors hover:bg-accent focus-visible:bg-accent focus-visible:outline-none">
            <PrName job={job} />
            <span className="ml-auto flex shrink-0 flex-col items-end gap-0.5">
              <StatusLabel job={job} now={now} />
              <span className="flex items-center gap-1.5"><Reviewers harnesses={job.harnesses} />{job.status === "drafted" && <Counts job={job} />}</span>
            </span>
          </button>
        </li>)}</ul>}
  </section>;
}

function ReviewersTable({ rows }: { rows: ReviewHarnessStats[] }) {
  return <section aria-label="Reviewers" className={cn(CARD, "min-w-0 overflow-x-auto p-4")}>
    <h2 className="mb-1 text-ui font-semibold text-foreground">Reviewers</h2>
    <p className="mb-3 text-caption text-muted-foreground">Kept rate is what you kept or edited of what you decided. A finding both reviewers flagged counts for each.</p>
    {rows.length === 0
      ? <p className="text-meta text-muted-foreground">No reviewer has run in this period.</p>
      : <table className="w-full min-w-[560px] text-left text-meta">
          <thead><tr className="border-b border-border text-caption text-muted-foreground">
            {["Reviewer", "Reviews", "Failed", "Findings", "Kept rate", "Median time", "Spend"].map(head => <th key={head} scope="col" className="py-1.5 pr-3 font-medium last:pr-0">{head}</th>)}
          </tr></thead>
          <tbody>{rows.map(row => {
            const rate = row.kept + row.dropped === 0 ? undefined : row.kept / (row.kept + row.dropped);
            return <tr key={row.harness} data-reviewer={row.harness} className="border-b border-border/60 last:border-0">
              <th scope="row" className="py-2 pr-3 font-medium text-foreground">{harnessLabel(row.harness)}</th>
              <td className="py-2 pr-3 tabular-nums">{row.passes}</td>
              <td className={cn("py-2 pr-3 tabular-nums", row.failed > 0 && "text-destructive")}>{row.failed}</td>
              <td className="py-2 pr-3 tabular-nums">{row.findings}</td>
              <td className="py-2 pr-3 tabular-nums">{percent(rate)}</td>
              <td className="py-2 pr-3 tabular-nums">{seconds(row.medianSeconds)}</td>
              <td className="py-2 tabular-nums" title={row.spendMicrousd === 0 ? "This harness reports no cost" : undefined}>{row.spendMicrousd === 0 ? "Not reported" : formatUsd(row.spendMicrousd)}</td>
            </tr>;
          })}</tbody>
        </table>}
  </section>;
}

function ReviewsTable({ jobs, now, onOpen }: { jobs: ReviewJob[]; now: number; onOpen: (job: ReviewJob) => void }) {
  const [filter, setFilter] = useState<JobFilter>("all");
  const shown = jobs.filter(job => matchesFilter(job, filter));
  return <div>
    <div className="mb-3"><Segmented<JobFilter> label="Show reviews" value={filter} options={JOB_FILTERS} onChange={setFilter} /></div>
    {shown.length === 0
      ? <p className="py-6 text-center text-meta text-muted-foreground">{jobs.length === 0 ? "No reviews in this period. Start one from a pull request's Review button in the GitHub pane." : "No reviews match this filter."}</p>
      : <div className="overflow-x-auto"><table className="w-full min-w-[720px] text-left text-meta">
          <thead><tr className="border-b border-border text-caption text-muted-foreground">
            {["Pull request", "Reviewers", "Status", "Findings", "Started"].map(head => <th key={head} scope="col" className="py-1.5 pr-3 font-medium">{head}</th>)}
          </tr></thead>
          <tbody>{shown.map(job => <tr key={job.id} data-job-row={job.id} onClick={() => onOpen(job)} className="cursor-pointer border-b border-border/60 transition-colors last:border-0 hover:bg-accent/60">
            <td className="max-w-[320px] py-2 pr-3">
              <button type="button" onClick={event => { event.stopPropagation(); onOpen(job); }} className="block w-full min-w-0 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
                <PrName job={job} />
              </button>
            </td>
            <td className="py-2 pr-3"><Reviewers harnesses={job.harnesses} />
              {(job.trigger === "push" || job.incrementalFromSha) && <span className="mt-0.5 block text-caption text-muted-foreground">{[job.trigger === "push" && "On push", job.incrementalFromSha && "Re-review"].filter(Boolean).join(" · ")}</span>}
            </td>
            <td className="py-2 pr-3"><StatusLabel job={job} now={now} /></td>
            <td className="py-2 pr-3"><Counts job={job} /></td>
            <td className="whitespace-nowrap py-2 text-caption text-muted-foreground">{relativeTime(job.createdAt, new Date(now))}</td>
          </tr>)}</tbody>
        </table></div>}
  </div>;
}

const DECISION_LABEL = { pending: "Not decided", kept: "Kept", edited: "Edited", dropped: "Dropped" } as const;

function FindingsTable({ findings, onOpenJob }: { findings: ReviewOverviewFinding[]; onOpenJob: (jobId: string) => void }) {
  const [query, setQuery] = useState("");
  const [severity, setSeverity] = useState<ReviewSeverity | "all">("all");
  const needle = query.trim().toLowerCase();
  const shown = findings.filter(item =>
    (severity === "all" || item.finding.severity === severity)
    && (!needle || [item.finding.title, item.finding.body, item.finding.path, repoOf(item) ?? "", item.prTitle ?? ""].some(text => text.toLowerCase().includes(needle))));
  return <div>
    <div className="mb-3 flex flex-wrap items-center gap-2">
      <label className="flex h-8 min-w-[220px] flex-1 items-center gap-2 rounded-lg border border-border bg-background px-2.5 text-meta focus-within:ring-2 focus-within:ring-ring">
        <Search size={13} aria-hidden="true" className="text-muted-foreground" />
        <input aria-label="Search findings" value={query} onChange={event => setQuery(event.target.value)} placeholder="Search titles, files and pull requests" className="min-w-0 flex-1 bg-transparent text-foreground outline-none placeholder:text-muted-foreground" />
      </label>
      <Segmented<ReviewSeverity | "all"> label="Severity" value={severity} options={[{ value: "all", label: "All" }, ...SEVERITIES.map(value => ({ value, label: value.charAt(0).toUpperCase() + value.slice(1) }))]} onChange={setSeverity} />
    </div>
    {shown.length === 0
      ? <p className="py-6 text-center text-meta text-muted-foreground">{findings.length === 0 ? "No findings in this period." : "No findings match."}</p>
      : <ul className="grid grid-cols-1 gap-1">{shown.map(item => <li key={item.finding.id} className="min-w-0">
          <button type="button" data-finding-row={item.finding.id} onClick={() => onOpenJob(item.finding.jobId)} className="grid w-full grid-cols-[72px_minmax(0,1fr)_auto] items-start gap-3 rounded-lg px-2 py-2 text-left transition-colors hover:bg-accent focus-visible:bg-accent focus-visible:outline-none">
            <span className={cn("text-caption font-semibold", SEVERITY_TEXT[item.finding.severity])}>{item.finding.severity.charAt(0).toUpperCase() + item.finding.severity.slice(1)}</span>
            <span className="min-w-0">
              <span className="block truncate text-meta font-medium text-foreground">{item.finding.title}</span>
              <span className="block truncate font-mono text-caption text-muted-foreground">{item.finding.path}:{item.finding.line} · {repoOf(item)} #{item.number}</span>
            </span>
            <span className="flex flex-col items-end gap-0.5 text-caption text-muted-foreground">
              <span className={cn(item.finding.decision !== "pending" && "font-medium text-foreground")}>{DECISION_LABEL[item.finding.decision]}</span>
              <span>{[item.harnesses.map(harnessLabel).join(" and "), item.finding.postedAt && "posted", item.finding.fixedInSha && "fixed later"].filter(Boolean).join(" · ")}</span>
            </span>
          </button>
        </li>)}</ul>}
  </div>;
}

function JobDetail({ jobId, fallback, now, onClose }: { jobId: string; fallback?: ReviewJob; now: number; onClose: () => void }) {
  const [detail, setDetail] = useState<ReviewJobDetail>();
  const [error, setError] = useState<string>();
  const closeRef = useRef<HTMLButtonElement>(null);
  const load = useCallback(() => {
    bridgeApi.getReview(jobId).then(setDetail, cause => setError(cause instanceof Error ? cause.message : String(cause)));
  }, [jobId]);
  useEffect(() => { setDetail(undefined); setError(undefined); load(); }, [load]);
  // The fresh job from the overview (live) wins over the detail's snapshot.
  const job = fallback ?? detail?.job;
  useEffect(() => { closeRef.current?.focus(); }, [jobId]);
  useEffect(() => {
    if (fallback && detail && fallback.status !== detail.job.status) load();
  }, [fallback?.status]); // eslint-disable-line react-hooks/exhaustive-deps
  return <div className="fixed inset-0 z-40 flex justify-end bg-background/40 backdrop-blur-[1px]" onMouseDown={event => { if (event.target === event.currentTarget) onClose(); }}>
    <aside role="dialog" aria-modal="true" aria-label="Review" data-review-detail={jobId} onKeyDown={event => { if (event.key === "Escape") { event.stopPropagation(); onClose(); } }}
      className="flex h-full w-full max-w-[640px] flex-col border-l border-border bg-background shadow-xl">
      <header className="flex items-start gap-3 border-b border-border px-5 py-4">
        {job ? <div className="min-w-0 flex-1">
          <p className="font-mono text-caption text-muted-foreground">{repoOf(job)} #{job.number}</p>
          <h2 className="mt-0.5 text-ui font-semibold text-foreground">{job.prTitle ?? `Pull request #${job.number}`}</h2>
          <div className="mt-1.5 flex flex-wrap items-center gap-2">
            <StatusLabel job={job} now={now} />
            <Reviewers harnesses={job.harnesses} />
            {repositoryUrl(job) && <a href={`${repositoryUrl(job)}/pull/${job.number}`} target="_blank" rel="noreferrer" className="inline-flex items-center gap-1 text-meta text-muted-foreground hover:text-foreground">Open on GitHub <SquareArrowOutUpRight size={11} aria-hidden="true" /></a>}
            {job.postedReviewUrl && <a href={job.postedReviewUrl} target="_blank" rel="noreferrer" className="inline-flex items-center gap-1 text-meta text-foreground hover:underline">Posted review <SquareArrowOutUpRight size={11} aria-hidden="true" /></a>}
          </div>
        </div> : <p className="flex-1 text-meta text-muted-foreground">Loading…</p>}
        <button ref={closeRef} type="button" onClick={onClose} aria-label="Close review" className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-muted-foreground hover:bg-accent hover:text-foreground"><X size={14} /></button>
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">
        {error && <p role="alert" className="mb-3 text-meta text-destructive">{error}</p>}
        {job?.error && <p role="alert" className="mb-3 text-meta text-destructive">{job.error}</p>}
        {detail?.summary && <section aria-label="Summary" className="mb-4">
          <h3 className={cn(EYEBROW, "mb-1.5")}>Summary</h3>
          <div className="text-meta text-foreground"><GithubMarkdown text={detail.summary} repositoryUrl={job ? repositoryUrl(job) : undefined} /></div>
        </section>}
        {detail && detail.passes.length > 0 && <section aria-label="Passes" className="mb-4">
          <h3 className={cn(EYEBROW, "mb-1.5")}>Reviewers</h3>
          <ul className="grid gap-1">{detail.passes.map(pass => {
            const took = pass.startedAt && pass.finishedAt ? Date.parse(pass.finishedAt) - Date.parse(pass.startedAt) : NaN;
            return <li key={pass.id} className="flex flex-wrap items-center gap-x-3 gap-y-0.5 text-meta">
              <span className="w-20 font-medium text-foreground">{harnessLabel(pass.harness)}</span>
              <span className={cn(pass.status === "failed" ? "text-destructive" : "text-muted-foreground")}>{pass.status === "done" ? "Done" : pass.status === "failed" ? "Failed" : pass.status === "running" ? "Running" : "Cancelled"}</span>
              {!Number.isNaN(took) && <span className="tabular-nums text-muted-foreground">{formatElapsed(took)}</span>}
              <span className="tabular-nums text-muted-foreground">{formatTokens(pass.tokensIn + pass.tokensOut)} tokens</span>
              <span className="tabular-nums text-muted-foreground">{pass.costMicrousd === 0 ? "cost not reported" : formatUsd(pass.costMicrousd)}</span>
              {pass.error && <span className="basis-full text-caption text-destructive">{pass.error}</span>}
            </li>;
          })}</ul>
        </section>}
        {job?.status === "drafted"
          ? <section aria-label="Triage"><ReviewDraft job={job} repositoryUrl={repositoryUrl(job)} onPosting={() => load()} /></section>
          : detail && detail.findings.length > 0 && <section aria-label="Findings">
              <h3 className={cn(EYEBROW, "mb-1.5")}>Findings</h3>
              <ul className="grid gap-1.5">{detail.findings.map(finding => <li key={finding.id} className="rounded-md border border-border px-3 py-2">
                <p className="flex items-baseline gap-2"><span className={cn("text-caption font-semibold", SEVERITY_TEXT[finding.severity])}>{finding.severity}</span><span className="text-meta font-medium text-foreground">{finding.title}</span></p>
                <p className="font-mono text-caption text-muted-foreground">{finding.path}:{finding.line} · {DECISION_LABEL[finding.decision]}{finding.fixedInSha ? " · fixed later" : ""}</p>
              </li>)}</ul>
            </section>}
      </div>
    </aside>
  </div>;
}

export function ReviewsScreen() {
  const [period, setPeriod] = useState<Period>(30);
  const [repository, setRepository] = useState<string>("");
  const [overview, setOverview] = useState<ReviewOverview>();
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string>();
  const [tab, setTab] = useState<Tab>("reviews");
  const [openJob, setOpenJob] = useState<string>();
  const [now, setNow] = useState(() => Date.now());
  const query = useRef({ period, repository });
  query.current = { period, repository };

  const read = useCallback(() => {
    const { period: days, repository: repo } = query.current;
    setLoading(true);
    bridgeApi.reviewOverview(days === "all" ? undefined : days, repo || undefined)
      .then(next => {
        // A later filter change may have landed first; keep only the current one.
        if (query.current.period === days && query.current.repository === repo) { setOverview(next); setError(undefined); setNow(Date.now()); }
      }, cause => setError(cause instanceof Error ? cause.message : String(cause)))
      .finally(() => setLoading(false));
  }, []);

  useEffect(() => { read(); }, [period, repository, read]);

  // Live: any job change rereads the overview, a burst of them once.
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    let unlisten: (() => void) | undefined;
    let active = true;
    void bridgeApi.onReviewJobUpdated(() => {
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => { if (active) read(); }, REFETCH_DELAY_MS);
    }).then(stop => { if (active) unlisten = stop; else stop(); });
    return () => { active = false; if (timer) clearTimeout(timer); unlisten?.(); };
  }, [read]);

  // Running jobs show a clock.
  const anyRunning = overview?.jobs.some(job => tone(job) === "running") ?? false;
  useEffect(() => {
    if (!anyRunning) return;
    const tick = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(tick);
  }, [anyRunning]);

  const jobs = overview?.jobs ?? [];
  const waiting = useMemo(() => jobs.filter(job => job.status === "drafted"), [jobs]);
  const running = useMemo(() => jobs.filter(job => tone(job) === "running"), [jobs]);
  const stats = overview?.stats;
  const rate = stats ? keptRate(stats) : undefined;
  const decided = stats ? stats.kept + stats.edited + stats.dropped : 0;
  const unpriced = overview?.harnesses.some(row => row.spendMicrousd === 0 && row.passes > 0) ?? false;
  const opened = openJob ? jobs.find(job => job.id === openJob) : undefined;

  return <div className="h-full min-h-0 overflow-y-auto">
    <div className={SCREEN_CONTENT}>
      <ScreenHeading
        title="Reviews"
        description="Every pull request review Bridge ran: what needs you, how each reviewer does, and what was found."
        action={<>
          <Segmented<Period> label="Period" value={period} options={PERIODS} onChange={setPeriod} />
          <select aria-label="Repository" value={repository} onChange={event => setRepository(event.target.value)} className="h-8 max-w-[220px] rounded-lg border border-border bg-background px-2 text-meta text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
            <option value="">All repositories</option>
            {(overview?.repositories ?? []).map(repo => <option key={repo} value={repo}>{repo}</option>)}
          </select>
          <button type="button" onClick={read} disabled={loading} aria-label="Refresh reviews" aria-busy={loading} className="inline-flex h-8 w-8 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-40"><RefreshCw size={14} className={cn(loading && "animate-spin motion-reduce:animate-none")} /></button>
        </>}
      />
      {error && <p role="alert" className="mb-4 text-meta text-destructive">{error}</p>}
      {stats && <>
        <div className="mb-4 grid grid-cols-2 gap-3 md:grid-cols-3 xl:grid-cols-6">
          <Tile label="Reviews" value={String(stats.reviews)} sub={[stats.posted && `${stats.posted} posted`, stats.waiting && `${stats.waiting} waiting`, stats.failed && `${stats.failed} failed`].filter(Boolean).join(" · ") || "None yet"} />
          <Tile label="Findings" value={String(stats.findings.total)} sub={stats.agreed > 0 ? `${stats.agreed} both reviewers flagged` : undefined}><SeverityBar counts={stats.findings} /></Tile>
          <Tile label="Kept" value={percent(rate)} sub={decided === 0 ? "Nothing decided yet" : `${stats.kept + stats.edited} of ${decided} decided`} />
          <Tile label="Fixed later" value={String(stats.fixedLater)} sub="Found fixed by a later review" />
          <Tile label="Median time" value={seconds(stats.medianSeconds)} sub="Start to draft" />
          <Tile label="Spend" value={formatUsd(stats.spendMicrousd)} sub={`${formatTokens(stats.tokensIn + stats.tokensOut)} tokens${unpriced ? " · some reviewers report no cost" : ""}`} />
        </div>
        <div className="mb-4 grid gap-3 lg:grid-cols-2">
          <JobList title="Needs you" jobs={waiting} empty="No drafts are waiting. A finished review shows up here to keep, drop or edit before it posts." now={now} onOpen={job => setOpenJob(job.id)} />
          <JobList title="Running" jobs={running} empty="Nothing is running." now={now} onOpen={job => setOpenJob(job.id)} />
        </div>
        <div className="mb-4"><ReviewersTable rows={overview.harnesses} /></div>
        <section aria-label="History" className={cn(CARD, "p-4")}>
          <div className="mb-3"><Segmented<Tab> label="History" value={tab} options={[{ value: "reviews", label: `Reviews ${jobs.length}` }, { value: "findings", label: `Findings ${overview.findings.length}` }]} onChange={setTab} /></div>
          {tab === "reviews"
            ? <ReviewsTable jobs={jobs} now={now} onOpen={job => setOpenJob(job.id)} />
            : <FindingsTable findings={overview.findings} onOpenJob={setOpenJob} />}
        </section>
      </>}
      {!stats && loading && <p className="text-meta text-muted-foreground">Loading reviews…</p>}
    </div>
    {openJob && <JobDetail jobId={openJob} fallback={opened} now={now} onClose={() => setOpenJob(undefined)} />}
  </div>;
}
