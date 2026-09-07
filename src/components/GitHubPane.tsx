import { Dialog, DialogPopup } from "@/components/ui/dialog";
import { useCallback, useEffect, useId, useRef, useState } from "react";
import {
  ArrowLeft,
  CircleCheck,
  CircleDashed,
  CircleDot,
  CircleSlash,
  CircleX,
  ExternalLink,
  FileDiff,
  FolderGit2,
  GitBranch,
  GitMerge,
  GitPullRequest,
  GitPullRequestDraft,
  Info,
  ListTodo,
  LoaderCircle,
  MessageSquare,
  RefreshCw,
  Sparkles,
  SquareArrowOutUpRight,
  Tag,
} from "lucide-react";
import { bridgeApi } from "../api";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Markdown } from "./Markdown";
import { relativeTime } from "./workDashboard";
import { checksNeedPolling, rollupState, type PullRequestListItem, type RollupState } from "../githubSurface";
import type {
  GithubAction,
  GithubCheckoutResult,
  GithubChecksResult,
  GithubIssueResult,
  GithubIssuesResult,
  GithubLabel,
  GithubMergeConfigResult,
  GithubPullRequestResult,
  GithubRepository,
  GithubRepositoryResult,
  GithubStatusResult,
  MergeStrategy,
  ReviewDecision,
} from "../protocol/generated/protocol";

// The GitHub surface: a dock pane, not a popover. Lives beside the
// conversation with real height, survives pane switches, and — unlike the old
// sidebar overlay — is never trapped by a transformed ancestor. Remote GitHub
// content is deliberately rendered through React text nodes only; every
// mutation runs behind a native confirmation naming the exact operation.

export type GitHubPaneProps = {
  workspaceId: string;
  workspaceBranch: string | null;
  /** The active orchestrator session, so a subagent review attaches to it
   * rather than minting an orphan session. */
  sessionId?: string;
  /** An outside ask (sidebar row, CI toast) to open one PR. Nonce distinguishes
   * "open it again" from a re-render. */
  intent?: { number: number; nonce: number };
  onJumpToFile: (path: string, line: number | undefined, headBranch: string) => void;
};

type Detail = { result: GithubPullRequestResult; checks: GithubChecksResult };
type SurfaceTab = "pulls" | "issues" | "repository";
type PullRequestTab = "conversation" | "changes" | "checks";

const ROLLUP: Record<RollupState, { icon: typeof CircleCheck; className: string; label: string }> = {
  failing: { icon: CircleX, className: "text-destructive", label: "Failing" },
  running: { icon: LoaderCircle, className: "animate-spin text-warning", label: "Running" },
  passing: { icon: CircleCheck, className: "text-success", label: "Passing" },
  none: { icon: CircleDashed, className: "text-muted-foreground", label: "No checks" },
};

const REVIEW: Record<ReviewDecision, { label: string; variant: "success" | "warning" | "info" | "outline" } | null> = {
  approved: { label: "Approved", variant: "success" },
  changesRequested: { label: "Changes requested", variant: "warning" },
  reviewRequired: { label: "Review required", variant: "info" },
  none: null,
};

const STRATEGY_LABEL: Record<MergeStrategy, string> = { merge: "Merge commit", squash: "Squash and merge", rebase: "Rebase and merge" };

function checkTone(conclusion: string | null | undefined, status: string) {
  if (status !== "completed") return { icon: LoaderCircle, className: "animate-spin text-warning" };
  switch (conclusion) {
    case "success": return { icon: CircleCheck, className: "text-success" };
    case "failure": case "timedOut": case "startupFailure": return { icon: CircleX, className: "text-destructive" };
    case "skipped": case "cancelled": case "neutral": return { icon: CircleSlash, className: "text-muted-foreground" };
    default: return { icon: CircleDashed, className: "text-muted-foreground" };
  }
}

/** Centered empty/availability state — the pane never renders blank. */
function PaneNotice({ icon: Icon, title, spin, children }: { icon: typeof GitPullRequest; title: string; spin?: boolean; children?: React.ReactNode }) {
  return <div className="flex h-full flex-col items-center justify-center gap-2 px-8 text-center">
    <span className="grid size-10 place-items-center rounded-xl border border-border bg-card text-muted-foreground"><Icon size={18} strokeWidth={1.6} className={cn(spin && "animate-spin")} aria-hidden="true" /></span>
    <p className="mt-1 text-[13px] font-medium text-foreground">{title}</p>
    <div className="max-w-[26rem] text-[12px] leading-relaxed text-muted-foreground">{children}</div>
  </div>;
}

/** GitHub-authored markdown at the pane's compact type scale. Rendering goes
 * through the app's Markdown component — React text nodes only, with fenced
 * HTML confined to a fully sandboxed iframe — so remote content stays inert. */
function GithubMarkdown({ text }: { text: string }) {
  return <div className="[&>.md]:text-[12.5px] [&>.md]:leading-relaxed"><Markdown text={text} /></div>;
}

/** A comment's relative age; omitted entirely when the timestamp is junk —
 * a raw ISO string (or "unknown") is noise, not information. */
function CommentTime({ iso }: { iso: string }) {
  if (Number.isNaN(Date.parse(iso))) return null;
  return <time dateTime={iso} title={new Date(iso).toLocaleString()} className="text-[11px] text-muted-foreground">{relativeTime(iso, new Date())}</time>;
}

const SKELETON_BAR = "animate-pulse rounded bg-muted-foreground/10";
const SKELETON_WIDTHS = ["w-3/5", "w-2/5", "w-1/2", "w-3/4", "w-2/5", "w-2/3"] as const;

/** Placeholder rows shaped like the list they precede, instead of a bare
 * spinner in an empty pane. */
function ListSkeleton({ label }: { label: string }) {
  return <div role="status" aria-label={label} className="min-h-0 flex-1 overflow-y-auto px-3 py-3 sm:px-4">
    <div className="divide-y divide-border overflow-hidden rounded-lg border border-border bg-card">
      {SKELETON_WIDTHS.map((width, index) => <div key={index} className="flex items-start gap-2.5 px-3 py-2.5">
        <span className={cn("mt-0.5 size-[15px] shrink-0 rounded-full", SKELETON_BAR)} />
        <span className="min-w-0 flex-1">
          <span className={cn("block h-3", SKELETON_BAR, width)} />
          <span className={cn("mt-1.5 block h-2.5 w-1/4", SKELETON_BAR)} />
        </span>
      </div>)}
    </div>
  </div>;
}

/** Placeholder shaped like a PR/issue detail: meta line, title, branch line,
 * then a few body bars. */
function DetailSkeleton({ label }: { label: string }) {
  return <div role="status" aria-label={label} className="min-h-0 flex-1 overflow-y-auto">
    <div className="border-b border-border px-4 pb-3 pt-3.5">
      <span className={cn("block h-3 w-28", SKELETON_BAR)} />
      <span className={cn("mt-2 block h-4 w-4/5", SKELETON_BAR)} />
      <span className={cn("mt-2 block h-3 w-2/5", SKELETON_BAR)} />
    </div>
    <div className="space-y-2.5 px-4 py-4">
      <span className={cn("block h-3 w-full", SKELETON_BAR)} />
      <span className={cn("block h-3 w-11/12", SKELETON_BAR)} />
      <span className={cn("block h-3 w-3/5", SKELETON_BAR)} />
    </div>
  </div>;
}

export function GitHubPane({ workspaceId, workspaceBranch, sessionId, intent, onJumpToFile }: GitHubPaneProps) {
  const [status, setStatus] = useState<GithubStatusResult>();
  const [prs, setPrs] = useState<PullRequestListItem[]>();
  const [issues, setIssues] = useState<GithubIssuesResult["issues"]>();
  const [repositoryOverview, setRepositoryOverview] = useState<GithubRepositoryResult>();
  const [surfaceTab, setSurfaceTab] = useState<SurfaceTab>("pulls");
  const [surfaceError, setSurfaceError] = useState<string>();
  const [tabErrors, setTabErrors] = useState<Partial<Record<SurfaceTab, string>>>({});
  const [refreshing, setRefreshing] = useState(false);

  const [selected, setSelected] = useState<number>();
  const [detail, setDetail] = useState<Detail>();
  const [detailError, setDetailError] = useState<string>();
  const [selectedIssue, setSelectedIssue] = useState<number>();
  const [issueDetail, setIssueDetail] = useState<GithubIssueResult>();
  const [issueError, setIssueError] = useState<string>();

  const alive = useRef(true);
  const refreshingChecks = useRef(false);
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);

  const loadSurface = useCallback(async (fresh = false) => {
    setRefreshing(true);
    try {
      const next = await bridgeApi.githubStatus(workspaceId, fresh);
      if (!alive.current) return;
      setStatus(next);
      setSurfaceError(undefined);
      if (next.availability.status === "available") {
        const settle = async <T,>(tab: SurfaceTab, read: Promise<T>, apply: (value: T) => void) => {
          try {
            const value = await read;
            if (!alive.current) return;
            apply(value);
            setTabErrors(current => {
              const nextErrors = { ...current };
              delete nextErrors[tab];
              return nextErrors;
            });
          } catch (error) {
            if (!alive.current) return;
            setTabErrors(current => ({
              ...current,
              [tab]: error instanceof Error ? error.message : String(error),
            }));
          }
        };
        await Promise.all([
          settle("pulls", bridgeApi.githubPullRequests(workspaceId), value => setPrs(value.pullRequests)),
          settle("issues", bridgeApi.githubIssues(workspaceId), value => setIssues(value.issues)),
          settle("repository", bridgeApi.githubRepository(workspaceId), setRepositoryOverview),
        ]);
      }
    } catch (error) {
      if (alive.current) setSurfaceError(error instanceof Error ? error.message : String(error));
    } finally {
      if (alive.current) setRefreshing(false);
    }
  }, [workspaceId]);

  const openDetail = useCallback(async (number: number, retain = false) => {
    setSurfaceTab("pulls");
    setSelected(number);
    if (!retain) setDetail(undefined);
    setDetailError(undefined);
    try {
      const [result, checks] = await Promise.all([
        bridgeApi.githubPullRequest(workspaceId, number),
        bridgeApi.githubChecks(workspaceId, number),
      ]);
      if (alive.current) setDetail({ result, checks });
    } catch (error) {
      if (alive.current) setDetailError(error instanceof Error ? error.message : String(error));
    }
  }, [workspaceId]);

  const openIssue = useCallback(async (number: number, retain = false) => {
    setSurfaceTab("issues");
    setSelectedIssue(number);
    if (!retain) setIssueDetail(undefined);
    setIssueError(undefined);
    try {
      const result = await bridgeApi.githubIssue(workspaceId, number);
      if (alive.current) setIssueDetail(result);
    } catch (error) {
      if (alive.current) setIssueError(error instanceof Error ? error.message : String(error));
    }
  }, [workspaceId]);

  const refreshChecks = useCallback(async (number: number) => {
    if (refreshingChecks.current) return;
    refreshingChecks.current = true;
    try {
      const [checks, list] = await Promise.all([
        bridgeApi.githubChecks(workspaceId, number),
        bridgeApi.githubPullRequests(workspaceId),
      ]);
      if (!alive.current) return;
      setPrs(list.pullRequests);
      setDetail(current => current && current.result.pullRequest.summary.number === number ? { ...current, checks } : current);
      setDetailError(undefined);
    } catch (error) {
      if (alive.current) setDetailError(error instanceof Error ? error.message : String(error));
    } finally { refreshingChecks.current = false; }
  }, [workspaceId]);

  // The timer path reads one PR's checks and nothing else. Refetching the
  // whole list every 8 seconds hammered `gh pr list` on large repositories
  // and kept daemon connections busy; list refreshes belong to the
  // checks-changed/ci-finished events, where the server saw a real change.
  const pollChecks = useCallback(async (number: number) => {
    if (refreshingChecks.current) return;
    refreshingChecks.current = true;
    try {
      const checks = await bridgeApi.githubChecks(workspaceId, number);
      if (!alive.current) return;
      setDetail(current => current && current.result.pullRequest.summary.number === number ? { ...current, checks } : current);
    } catch {
      // Keep the last good checks; the next tick or event retries.
    } finally { refreshingChecks.current = false; }
  }, [workspaceId]);

  useEffect(() => {
    setStatus(undefined); setPrs(undefined); setIssues(undefined); setRepositoryOverview(undefined); setSurfaceError(undefined); setTabErrors({});
    setSelected(undefined); setDetail(undefined); setDetailError(undefined);
    setSelectedIssue(undefined); setIssueDetail(undefined); setIssueError(undefined);
    void loadSurface();
  }, [loadSurface]);

  useEffect(() => {
    if (selected === undefined || !detail || !checksNeedPolling(detail.checks.checks)) return;
    const timer = window.setInterval(() => { void pollChecks(selected); }, 8_000);
    return () => window.clearInterval(timer);
  }, [selected, detail, pollChecks]);

  // Live refresh: a check transition or a terminal rollup re-reads whatever is
  // on screen. Handlers read the latest selection through a ref so one stable
  // subscription outlives every selection change.
  const selectedRef = useRef<number>();
  selectedRef.current = selected;
  useEffect(() => {
    let active = true;
    const offs: Array<() => void> = [];
    const refetch = (payload: { workspaceId: string; number: number }) => {
      if (!active || payload.workspaceId !== workspaceId) return;
      if (selectedRef.current === payload.number) void refreshChecks(payload.number);
      else void bridgeApi.githubPullRequests(workspaceId).then(value => { if (active) setPrs(value.pullRequests); }).catch(() => undefined);
    };
    void bridgeApi.onGithubChecksChanged(refetch).then(off => { if (active) offs.push(off); else off(); });
    void bridgeApi.onGithubCiFinished(refetch).then(off => { if (active) offs.push(off); else off(); });
    return () => { active = false; offs.forEach(off => off()); };
  }, [workspaceId, refreshChecks]);

  // Deep links (sidebar row, CI toast) land here.
  const seenIntent = useRef(0);
  useEffect(() => {
    if (!intent || intent.nonce === seenIntent.current) return;
    seenIntent.current = intent.nonce;
    void openDetail(intent.number);
  }, [intent, openDetail]);

  const repoLabel = status?.repository ? `${status.repository.owner}/${status.repository.name}` : undefined;

  let body: React.ReactNode;
  if (surfaceError) {
    body = <PaneNotice icon={CircleX} title="GitHub is unreachable">{surfaceError}</PaneNotice>;
  } else if (!status) {
    body = <PaneNotice icon={LoaderCircle} spin title="Checking GitHub…">Resolving the CLI and repository.</PaneNotice>;
  } else if (status.availability.status === "notInstalled") {
    body = <PaneNotice icon={CircleSlash} title="GitHub CLI is not installed">Bridge drives GitHub through <code className="font-mono text-foreground/90">gh</code> — install it and sign in, and this pane fills in by itself.</PaneNotice>;
  } else if (status.availability.status === "notAuthenticated") {
    body = <PaneNotice icon={CircleDot} title="Sign in to GitHub">Run <code className="rounded-md border border-border bg-card px-1.5 py-0.5 font-mono text-[12px] text-foreground/90">{status.availability.remediation}</code> in a terminal, then refresh.</PaneNotice>;
  } else if (surfaceTab === "pulls" && selected !== undefined) {
    body = <PullRequestDetail
      workspaceId={workspaceId}
      workspaceBranch={workspaceBranch}
      sessionId={sessionId}
      repository={status.repository}
      number={selected}
      detail={detail}
      error={detailError}
      availableLabels={repositoryOverview?.labels ?? []}
      onBack={() => { setSelected(undefined); setDetail(undefined); setDetailError(undefined); }}
      onActed={() => { void loadSurface(); void openDetail(selected, true); }}
      onJumpToFile={onJumpToFile}
    />;
  } else if (surfaceTab === "pulls" && tabErrors.pulls && !prs) {
    body = <PaneNotice icon={CircleX} title="Pull requests did not load">{tabErrors.pulls}</PaneNotice>;
  } else if (surfaceTab === "pulls" && !prs) {
    body = <ListSkeleton label="Loading pull requests" />;
  } else if (surfaceTab === "pulls" && prs?.length === 0) {
    body = <PaneNotice icon={GitPullRequest} title="No open pull requests">{repoLabel ? `${repoLabel} has nothing waiting on you.` : "This repository has nothing waiting on you."}</PaneNotice>;
  } else if (surfaceTab === "pulls" && prs) {
    body = <div className="min-h-0 flex-1 overflow-y-auto px-3 py-3 sm:px-4">
      <div className="divide-y divide-border overflow-hidden rounded-lg border border-border bg-card">
        {prs.map(pr => <PullRequestRow key={pr.number} pr={pr} onOpen={() => void openDetail(pr.number)} />)}
      </div>
    </div>;
  } else if (surfaceTab === "issues" && selectedIssue !== undefined) {
    body = <IssueDetail
      workspaceId={workspaceId}
      repository={status.repository}
      number={selectedIssue}
      detail={issueDetail}
      error={issueError}
      availableLabels={repositoryOverview?.labels ?? []}
      onBack={() => { setSelectedIssue(undefined); setIssueDetail(undefined); setIssueError(undefined); }}
      onActed={() => { void loadSurface(); void openIssue(selectedIssue, true); }}
    />;
  } else if (surfaceTab === "issues" && tabErrors.issues && !issues) {
    body = <PaneNotice icon={CircleX} title="Issues did not load">{tabErrors.issues}</PaneNotice>;
  } else if (surfaceTab === "issues" && !issues) {
    body = <ListSkeleton label="Loading issues" />;
  } else if (surfaceTab === "issues" && issues?.length === 0) {
    body = <PaneNotice icon={ListTodo} title="No open issues">This repository has no open issues.</PaneNotice>;
  } else if (surfaceTab === "issues" && issues) {
    body = <IssueList issues={issues} onOpen={number => void openIssue(number)} />;
  } else if (tabErrors.repository && !repositoryOverview) {
    body = <PaneNotice icon={CircleX} title="Repository did not load">{tabErrors.repository}</PaneNotice>;
  } else if (!repositoryOverview) {
    body = <DetailSkeleton label="Loading repository" />;
  } else {
    body = <RepositoryOverview overview={repositoryOverview} />;
  }

  return <section className="relative flex h-full w-full flex-col" aria-label="GitHub repository">
    <header className="flex min-h-11 shrink-0 flex-wrap items-center gap-2 border-b border-border bg-muted/25 px-3 py-1.5">
      <FolderGit2 size={14} className="shrink-0 text-muted-foreground" aria-hidden="true" />
      {repoLabel && <span className="min-w-0 truncate font-mono text-[11px] text-muted-foreground">{repoLabel}</span>}
      <nav className="u-segmented ml-auto flex shrink-0 p-0.5" aria-label="GitHub sections">
        {([
          ["pulls", "Pull requests", GitPullRequest],
          ["issues", "Issues", ListTodo],
          ["repository", "Repository", Info],
        ] as const).map(([id, label, Icon]) => <button
          key={id}
          type="button"
          aria-label={label}
          aria-pressed={surfaceTab === id}
          data-active={surfaceTab === id}
          title={label}
          onClick={() => { setSurfaceTab(id); setSelected(undefined); setSelectedIssue(undefined); }}
          className="u-segmented-item grid size-7 place-items-center rounded-md text-muted-foreground"
        ><Icon size={12} aria-hidden="true" /></button>)}
      </nav>
      <button
        type="button"
        onClick={() => { void loadSurface(true); if (selected !== undefined) void openDetail(selected, true); if (selectedIssue !== undefined) void openIssue(selectedIssue, true); }}
        aria-label="Refresh GitHub"
        title="Refresh"
        className="grid size-7 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
      >
        <RefreshCw size={12.5} className={cn(refreshing && "animate-spin")} aria-hidden="true" />
      </button>
    </header>
    {tabErrors[surfaceTab] && ((surfaceTab === "pulls" && prs) || (surfaceTab === "issues" && issues) || (surfaceTab === "repository" && repositoryOverview)) && <p role="alert" className="border-b border-border bg-warning/10 px-4 py-2 text-[11px] text-warning">Refresh failed: {tabErrors[surfaceTab]}</p>}
    {body}
  </section>;
}

function PullRequestRow({ pr, onOpen }: { pr: PullRequestListItem; onOpen: () => void }) {
  const rollup = ROLLUP[rollupState(pr)];
  const RollupIcon = rollup.icon;
  const review = REVIEW[pr.reviewDecision];
  const StateIcon = pr.isDraft ? GitPullRequestDraft : pr.state === "merged" ? GitMerge : GitPullRequest;
  return <button type="button" onClick={onOpen} className="group flex w-full items-start gap-2.5 px-3 py-2.5 text-left transition-colors hover:bg-accent/50">
    <StateIcon size={15} strokeWidth={1.8} aria-hidden="true" className={cn("mt-0.5 shrink-0", pr.isDraft ? "text-muted-foreground" : pr.state === "merged" ? "text-info" : "text-success")} />
    <span className="min-w-0 flex-1">
      <span className="block truncate text-[13px] font-medium leading-5 text-foreground">{pr.title}</span>
      <span className="mt-0.5 flex items-center gap-1.5 text-[11px] text-muted-foreground">
        <span className="shrink-0 tabular-nums">#{pr.number}</span>
        <span aria-hidden="true">·</span>
        <span className="truncate font-mono">{pr.headBranch}</span>
        <span aria-hidden="true">·</span>
        <span className="shrink-0">{pr.author?.login ?? "ghost"}</span>
      </span>
    </span>
    <span className="flex shrink-0 items-center gap-1.5 pt-0.5">
      {pr.isDraft && <Badge variant="outline" size="sm">Draft</Badge>}
      {review && <Badge variant={review.variant} size="sm">{review.label}</Badge>}
      {pr.checks.total > 0 && <span className={cn("inline-flex items-center gap-1 font-mono text-[11px] tabular-nums", rollup.className)} title={`${rollup.label} — ${pr.checks.passed}/${pr.checks.total} checks passed`}>
        <RollupIcon size={12} aria-hidden="true" />
        {pr.checks.passed}/{pr.checks.total}
      </span>}
    </span>
  </button>;
}

function LabelBadge({ label }: { label: GithubLabel }) {
  return <Badge variant="outline" size="sm" title={label.description || label.name}>{label.name}</Badge>;
}

function LabelControls({ available, current, disabled, onChange }: {
  available: GithubLabel[];
  current: GithubLabel[];
  disabled: boolean;
  onChange: (label: string, operation: "add" | "remove") => void;
}) {
  const selected = new Set(current.map(label => label.name));
  return <div className="mt-2 grid gap-1 rounded-lg border border-border bg-card p-1.5" aria-label="Repository labels">
    {available.length ? available.map(label => {
      const active = selected.has(label.name);
      return <button key={label.name} type="button" disabled={disabled} onClick={() => onChange(label.name, active ? "remove" : "add")} className="flex items-center gap-2 rounded-md px-2 py-1 text-left text-[11px] text-foreground transition-colors hover:bg-accent disabled:opacity-50">
        <Tag size={11} className={active ? "text-primary" : "text-muted-foreground"} aria-hidden="true" />
        <span className="min-w-0 flex-1 truncate">{label.name}</span>
        <span className="text-[11px] text-muted-foreground">{active ? "Remove" : "Add"}</span>
      </button>;
    }) : <p className="px-2 py-1 text-[11px] text-muted-foreground">No repository labels.</p>}
  </div>;
}

/** Every rendered patch line is a DOM node; unbounded patches froze the
 * webview on large PRs. Files above the eager threshold start collapsed and
 * single patches render at most this many lines. */
export const EAGER_PATCH_FILE_LIMIT = 6;
export const PATCH_LINE_LIMIT = 600;

function PatchView({ patch, fullDiffUrl }: { patch: string; fullDiffUrl: string }) {
  const lines = patch.split("\n");
  const shown = lines.length > PATCH_LINE_LIMIT ? lines.slice(0, PATCH_LINE_LIMIT) : lines;
  return <>
    <pre className="max-h-80 overflow-auto bg-background/50 py-2 font-mono text-[11px] leading-5" aria-label="File patch">{shown.map((line, index) => <span key={`${index}-${line}`} className={cn("block whitespace-pre px-3", line.startsWith("+") && !line.startsWith("+++") && "bg-success/10 text-success", line.startsWith("-") && !line.startsWith("---") && "bg-destructive/10 text-destructive", line.startsWith("@@") && "bg-info/10 text-info")}>
      {line || " "}
    </span>)}</pre>
    {shown.length < lines.length && <a href={fullDiffUrl} target="_blank" rel="noreferrer" className="flex items-center gap-1.5 border-t border-border px-3 py-2 text-[11px] text-muted-foreground transition-colors hover:text-foreground">
      <ExternalLink size={11} aria-hidden="true" />
      Patch truncated at {PATCH_LINE_LIMIT} lines — view the full diff on GitHub
    </a>}
  </>;
}

function FileChange({ file, defaultOpen, fullDiffUrl }: {
  file: GithubPullRequestResult["files"][number];
  defaultOpen: boolean;
  fullDiffUrl: string;
}) {
  const [open, setOpen] = useState(defaultOpen);
  return <article className="overflow-hidden rounded-lg border border-border bg-card">
    <button type="button" onClick={() => setOpen(value => !value)} aria-expanded={open} className={cn("flex w-full items-center gap-2 px-3 py-2 text-left transition-colors hover:bg-accent/50", open && "border-b border-border")}>
      <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-foreground">{file.path}</span>
      <Badge variant="outline" size="sm">{file.status}</Badge>
      <span className="font-mono text-[11px] text-success">+{file.additions}</span>
      <span className="font-mono text-[11px] text-destructive">−{file.deletions}</span>
    </button>
    {open && (file.patch ? <PatchView patch={file.patch} fullDiffUrl={fullDiffUrl} /> : <p className="px-3 py-4 text-center text-[12px] text-muted-foreground">Binary file or patch unavailable from GitHub.</p>)}
  </article>;
}

function CommentList({ comments }: { comments: Array<{ id: string; author?: { login: string } | null; body: string; createdAt: string }> }) {
  return <div className="space-y-2">{comments.map(comment => <article key={comment.id} className="rounded-lg border border-border bg-card px-3 py-2.5">
    <p className="flex items-baseline gap-2 text-[11px]">
      <span className="font-medium text-foreground">{comment.author?.login ?? "ghost"}</span>
      <CommentTime iso={comment.createdAt} />
    </p>
    <div className="mt-1"><GithubMarkdown text={comment.body} /></div>
  </article>)}</div>;
}

function ChecksList({ checks }: { checks: GithubChecksResult["checks"] }) {
  return <section aria-label="Checks">
    <h3 className="mb-2 text-[12px] font-medium text-muted-foreground">CHECKS</h3>
    {checks.length ? <div className="divide-y divide-border overflow-hidden rounded-lg border border-border bg-card">
      {checks.map(check => {
        const tone = checkTone(check.conclusion, check.status);
        const ToneIcon = tone.icon;
        return <div key={`${check.workflow}-${check.name}`} className="flex items-center gap-2.5 px-3 py-2">
          <ToneIcon size={13.5} className={cn("shrink-0", tone.className)} aria-hidden="true" />
          <span className="min-w-0 flex-1 truncate text-[12px] text-foreground">{check.name}</span>
          <span className="shrink-0 text-[11px] text-muted-foreground">{check.conclusion ?? check.status}</span>
          {check.logUrl && <a href={check.logUrl} target="_blank" rel="noreferrer" aria-label={`Open logs for ${check.name}`} title="Open logs" className="grid size-7 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"><ExternalLink size={12.5} aria-hidden="true" /></a>}
        </div>;
      })}
    </div> : <p className="text-[12px] text-muted-foreground">No checks reported.</p>}
  </section>;
}

function IssueList({ issues, onOpen }: { issues: GithubIssuesResult["issues"]; onOpen: (number: number) => void }) {
  return <div className="min-h-0 flex-1 overflow-y-auto px-3 py-3 sm:px-4">
    <div className="divide-y divide-border overflow-hidden rounded-lg border border-border bg-card">
      {issues.map(issue => <button key={issue.number} type="button" onClick={() => onOpen(issue.number)} className="flex w-full items-start gap-2.5 px-3 py-2.5 text-left transition-colors hover:bg-accent/50">
        <CircleDot size={14} className="mt-0.5 shrink-0 text-success" aria-hidden="true" />
        <span className="min-w-0 flex-1">
          <span className="block truncate text-[13px] font-medium text-foreground">{issue.title}</span>
          <span className="mt-1 flex items-center gap-1.5 text-[11px] text-muted-foreground"><span>#{issue.number}</span><span aria-hidden="true">·</span><span>{issue.author?.login ?? "ghost"}</span>{!Number.isNaN(Date.parse(issue.updatedAt)) && <><span aria-hidden="true">·</span><span>updated {relativeTime(issue.updatedAt, new Date())}</span></>}</span>
          {!!issue.labels.length && <span className="mt-1.5 flex flex-wrap gap-1">{issue.labels.map(label => <LabelBadge key={label.name} label={label} />)}</span>}
        </span>
      </button>)}
    </div>
  </div>;
}

function RepositoryOverview({ overview }: { overview: GithubRepositoryResult }) {
  const facts = [
    ["Default branch", overview.defaultBranch, GitBranch],
    ["Language", overview.primaryLanguage ?? "Not reported", FileDiff],
    ["Open issues", String(overview.openIssues), ListTodo],
    ["Open pull requests", String(overview.openPullRequests), GitPullRequest],
  ] as const;
  return <div className="min-h-0 flex-1 overflow-y-auto px-4 py-4">
    <div className="u-glass-soft rounded-xl border border-border p-4">
      <div className="flex items-start gap-3"><span className="grid size-9 shrink-0 place-items-center rounded-lg border border-border bg-card"><FolderGit2 size={16} aria-hidden="true" /></span><div className="min-w-0"><h2 className="truncate font-display text-base font-semibold text-foreground">{overview.nameWithOwner}</h2><p className="mt-1 text-[12px] leading-relaxed text-muted-foreground">{overview.description || "No repository description."}</p></div><Badge variant="outline" size="sm">{overview.visibility.toLowerCase()}</Badge></div>
      <div className="mt-4 grid grid-cols-2 gap-2">{facts.map(([label, value, Icon]) => <div key={label} className="rounded-lg border border-border bg-card px-3 py-2.5"><p className="flex items-center gap-1.5 text-[11px] text-muted-foreground"><Icon size={11} aria-hidden="true" />{label}</p><p className="mt-1 truncate font-mono text-[12px] text-foreground">{value}</p></div>)}</div>
      <section className="mt-4"><h3 className="mb-2 text-[12px] font-medium text-muted-foreground">LABELS</h3><div className="flex flex-wrap gap-1.5">{overview.labels.length ? overview.labels.map(label => <LabelBadge key={label.name} label={label} />) : <p className="text-[12px] text-muted-foreground">No labels.</p>}</div></section>
    </div>
  </div>;
}

// ── Detail ───────────────────────────────────────────────────────────────────

type PendingAction = { statement: string; requiresBody: boolean; build: (body: string) => GithubAction };

/** The harnesses a subagent review can run under. The model comes from the
 * Reviewer profile in settings, so the user only picks the agent. Cursor
 * Bugbot is not a local worker: it posts `cursor review` on the PR. */
const REVIEW_HARNESSES: ReadonlyArray<{ id: string; label: string }> = [
  { id: "claude", label: "Claude" },
  { id: "codex", label: "Codex" },
  { id: "opencode", label: "OpenCode" },
  { id: "bugbot", label: "Cursor Bugbot" },
];

type PullRequestDetailProps = {
  workspaceId: string;
  workspaceBranch: string | null;
  sessionId?: string;
  repository?: GithubRepository | null;
  number: number;
  detail?: Detail;
  error?: string;
  availableLabels: GithubLabel[];
  onBack: () => void;
  onActed: () => void;
  onJumpToFile: (path: string, line: number | undefined, headBranch: string) => void;
};

function PullRequestDetail({ workspaceId, workspaceBranch, sessionId, repository, number, detail, error, availableLabels, onBack, onActed, onJumpToFile }: PullRequestDetailProps) {
  const [pending, setPending] = useState<PendingAction>();
  const [reviewOpen, setReviewOpen] = useState(false);
  const [reviewBusy, setReviewBusy] = useState(false);
  const [reviewNotice, setReviewNotice] = useState<{ tone: "success" | "error"; text: string }>();
  const [pendingBody, setPendingBody] = useState("");
  const [mergeOpen, setMergeOpen] = useState(false);
  const [mergeConfig, setMergeConfig] = useState<GithubMergeConfigResult>();
  const [strategy, setStrategy] = useState<MergeStrategy>();
  const [checkoutOpen, setCheckoutOpen] = useState(false);
  const [checkout, setCheckout] = useState<GithubCheckoutResult>();
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string>();
  const [tab, setTab] = useState<PullRequestTab>("conversation");
  const detailId = useId();
  const [labelsOpen, setLabelsOpen] = useState(false);

  const repoLabel = repository ? `${repository.owner}/${repository.name}` : "this repository";
  const closeOverlays = () => { setPending(undefined); setPendingBody(""); setMergeOpen(false); setMergeConfig(undefined); setStrategy(undefined); setCheckoutOpen(false); setLabelsOpen(false); setReviewOpen(false); };

  // Hand the PR to a read-only subagent that posts its review as a comment via
  // `gh`. Non-blocking: the notice reports how the launch was routed and the
  // worker runs on in the agent tree while the user stays in the pane.
  const startReview = async (harness: string) => {
    setReviewBusy(true); setReviewNotice(undefined);
    try {
      const result = await bridgeApi.githubReview(workspaceId, number, harness, sessionId);
      setReviewOpen(false);
      setReviewNotice({ tone: result.status === "failed" ? "error" : "success", text: result.message });
      if (harness === "bugbot" && result.status !== "failed") onActed();
    } catch (value) {
      setReviewNotice({ tone: "error", text: value instanceof Error ? value.message : String(value) });
    } finally { setReviewBusy(false); }
  };

  // The only path that reaches `github/act`. A refused `gh` write surfaces its
  // message verbatim and leaves the PR untouched — no optimistic edit.
  const submit = async (action: GithubAction) => {
    setBusy(true); setActionError(undefined);
    try {
      const outcome = await bridgeApi.githubAct(workspaceId, action, true);
      closeOverlays();
      if (!outcome.executed) { setActionError(outcome.message); return; }
      onActed();
    } catch (value) {
      setActionError(value instanceof Error ? value.message : String(value));
    } finally { setBusy(false); }
  };

  const openMerge = async () => {
    setActionError(undefined); setMergeOpen(true); setMergeConfig(undefined); setStrategy(undefined);
    try {
      const config = await bridgeApi.githubMergeConfig(workspaceId);
      setMergeConfig(config); setStrategy(config.defaultStrategy);
    } catch (value) {
      setMergeOpen(false);
      setActionError(value instanceof Error ? value.message : String(value));
    }
  };

  const runCheckout = async () => {
    setBusy(true); setActionError(undefined);
    try {
      const result = await bridgeApi.githubCheckout(workspaceId, number);
      closeOverlays();
      setCheckout(result);
    } catch (value) {
      setActionError(value instanceof Error ? value.message : String(value));
    } finally { setBusy(false); }
  };

  const header = <div className="flex shrink-0 items-center gap-1 border-b border-border px-2 py-1.5">
    <button type="button" onClick={onBack} className="inline-flex min-h-7 items-center gap-1.5 rounded-md px-2 py-1 text-[12px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">
      <ArrowLeft size={12.5} aria-hidden="true" /> All pull requests
    </button>
  </div>;

  if (error && !detail) return <div className="flex min-h-0 flex-1 flex-col">{header}<PaneNotice icon={CircleX} title={`Pull request #${number} did not load`}>{error}</PaneNotice></div>;
  if (!detail) return <div className="flex min-h-0 flex-1 flex-col">{header}<DetailSkeleton label={`Opening pull request #${number}`} /></div>;

  const { pullRequest, reviewThreads } = detail.result;
  const summary = pullRequest.summary;
  const review = REVIEW[summary.reviewDecision];
  const rerunnable = detail.checks.checks.some(check =>
    check.conclusion === "failure" || check.conclusion === "timedOut" || check.conclusion === "startupFailure"
  );
  const checkedOutHere = workspaceBranch === summary.headBranch;
  const StateIcon = summary.isDraft ? GitPullRequestDraft : summary.state === "merged" ? GitMerge : GitPullRequest;

  const actionButton = "rounded-md border border-border bg-card px-2.5 py-1 text-[12px] font-medium text-foreground transition-colors hover:bg-accent disabled:opacity-50";

  return <div className="relative flex min-h-0 flex-1 flex-col">
    {header}
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="border-b border-border px-4 pb-3 pt-3.5">
        <div className="flex items-center gap-2 text-[11px] text-muted-foreground">
          <StateIcon size={13} aria-hidden="true" className={cn(summary.isDraft ? "text-muted-foreground" : summary.state === "merged" ? "text-info" : summary.state === "closed" ? "text-destructive" : "text-success")} />
          <span className="tabular-nums">#{summary.number}</span>
          <span className="capitalize">{summary.isDraft ? "draft" : summary.state}</span>
          <span className="truncate">by {summary.author?.login ?? "ghost"}</span>
          {review && <Badge variant={review.variant} size="sm">{review.label}</Badge>}
          <a href={summary.url} target="_blank" rel="noreferrer" aria-label={`Open #${summary.number} on GitHub`} title="Open on GitHub" className="ml-auto text-muted-foreground transition-colors hover:text-foreground"><SquareArrowOutUpRight size={12.5} aria-hidden="true" /></a>
        </div>
        <h2 className="mt-1.5 font-display text-[15px] font-semibold leading-snug tracking-[-0.01em] text-foreground">{summary.title}</h2>
        <p className="mt-1 font-mono text-[11px] text-muted-foreground">
          {summary.headBranch} <span aria-hidden="true">→</span> {pullRequest.baseBranch}
          {checkedOutHere && <span className="ml-1.5 text-success">· checked out here</span>}
        </p>
        <div className="mt-2 flex flex-wrap items-center gap-1.5">
          {pullRequest.labels.map(label => <LabelBadge key={label.name} label={label} />)}
          <button type="button" onClick={() => setLabelsOpen(value => !value)} className="inline-flex min-h-7 items-center gap-1 rounded-md px-2 py-1 text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">
            <Tag size={10.5} aria-hidden="true" /> Manage labels
          </button>
        </div>
        {labelsOpen && <LabelControls
          available={availableLabels}
          current={pullRequest.labels}
          disabled={busy}
          onChange={(label, operation) => setPending({
            statement: `${operation === "add" ? "add" : "remove"} label ${JSON.stringify(label)} ${operation === "add" ? "to" : "from"} PR #${number} on ${repoLabel}`,
            requiresBody: false,
            build: () => ({ kind: "label", target: "pullRequest", number, label, operation }),
          })}
        />}

        {summary.state === "open" && <>
          <div className="mt-3 flex flex-wrap items-center gap-1.5">
          <button type="button" onClick={() => void openMerge()} disabled={busy} className="rounded-md border border-success/25 bg-success/10 px-2.5 py-1 text-[12px] font-medium text-success transition-colors hover:bg-success/20 disabled:opacity-50">Merge</button>
          <button type="button" disabled={busy} className={actionButton} onClick={() => setPending({ statement: `submit approval on PR #${number} on ${repoLabel}`, requiresBody: false, build: () => ({ kind: "review", number, event: "approve", body: "" }) })}>Approve</button>
          <button type="button" disabled={busy} className={actionButton} onClick={() => setPending({ statement: `submit requested changes on PR #${number} on ${repoLabel}`, requiresBody: true, build: body => ({ kind: "review", number, event: "requestChanges", body }) })}>Request changes</button>
          <button type="button" disabled={busy || reviewBusy} aria-haspopup="menu" aria-expanded={reviewOpen} className={actionButton} onClick={() => { setReviewNotice(undefined); setReviewOpen(value => !value); }}>
            <span className="inline-flex items-center gap-1.5"><Sparkles size={12} aria-hidden="true" /> {reviewBusy ? "Starting review…" : "Review"}</span>
          </button>
          {rerunnable && <button type="button" disabled={busy} className={actionButton} onClick={() => setPending({ statement: `re-run failed checks on PR #${number} on ${repoLabel}`, requiresBody: false, build: () => ({ kind: "rerun", number }) })}>Re-run failed</button>}
          {!checkedOutHere && <button type="button" disabled={busy} className={actionButton} onClick={() => { setActionError(undefined); setCheckoutOpen(true); }}>
            <span className="inline-flex items-center gap-1.5"><FolderGit2 size={12} aria-hidden="true" /> Check out</span>
          </button>}
          </div>
          {reviewOpen && <div className="mt-2 grid gap-1 rounded-lg border border-border bg-card p-1.5" role="menu" aria-label="Review harness">
            <p className="px-2 pb-0.5 pt-1 text-[11px] font-semibold tracking-[0.1em] text-muted-foreground">RUN REVIEW WITH</p>
            {REVIEW_HARNESSES.map(choice => <button key={choice.id} type="button" role="menuitem" disabled={reviewBusy} onClick={() => void startReview(choice.id)} className="flex items-center gap-2 rounded-md px-2 py-1 text-left text-[12px] text-foreground transition-colors hover:bg-accent disabled:opacity-50">
              <Sparkles size={11} className="text-muted-foreground" aria-hidden="true" />
              <span className="min-w-0 flex-1 truncate">{choice.label}</span>
            </button>)}
          </div>}
          {reviewNotice && <p role="status" className={cn("mt-2 rounded-md border px-2.5 py-1.5 text-[12px]", reviewNotice.tone === "success" ? "border-success/25 bg-success/10 text-success" : "border-destructive/25 bg-destructive/10 text-destructive")}>{reviewNotice.text}</p>}
        </>}
      </div>

      {actionError && <p role="alert" className="border-b border-border bg-destructive/10 px-4 py-2 text-[12px] text-destructive">{actionError}</p>}
      {error && <p role="alert" className="border-b border-border bg-warning/10 px-4 py-2 text-[12px] text-warning">Live refresh failed: {error}</p>}
      {checkout && <p className="border-b border-border bg-success/10 px-4 py-2 text-[12px] text-success">
        {checkout.reused ? "Reusing the task worktree already on " : "Checked out into a task worktree on "}
        <span className="font-mono">{checkout.branch}</span>
        <span className="block truncate font-mono text-[11px] text-success/80" title={checkout.path}>{checkout.path}</span>
      </p>}

      <div className="sticky top-0 z-[1] flex border-b border-border bg-background/95 px-4" role="tablist" aria-label="Pull request detail">
        {(["conversation", "changes", "checks"] as const).map((value, index, options) => <button key={value} type="button" role="tab" id={`${detailId}-${value}`} aria-controls={`${detailId}-panel`} aria-selected={tab === value} tabIndex={tab === value ? 0 : -1} onClick={() => setTab(value)} onKeyDown={event => {
          let next = index;
          if (event.key === "ArrowRight") next = (index + 1) % options.length;
          else if (event.key === "ArrowLeft") next = (index - 1 + options.length) % options.length;
          else if (event.key === "Home") next = 0;
          else if (event.key === "End") next = options.length - 1;
          else return;
          event.preventDefault();
          setTab(options[next]);
          document.getElementById(`${detailId}-${options[next]}`)?.focus();
        }} className={cn("border-b-2 border-transparent px-2.5 py-2 text-[12px] capitalize text-muted-foreground", tab === value && "border-primary text-foreground")}>
          {value}{value === "changes" ? ` ${pullRequest.changedFiles}` : value === "checks" ? ` ${detail.checks.checks.length}` : ""}
        </button>)}
      </div>

      <div id={`${detailId}-panel`} role="tabpanel" aria-labelledby={`${detailId}-${tab}`} tabIndex={0} className="space-y-5 px-4 py-4">
        {tab === "checks" && <ChecksList checks={detail.checks.checks} />}

        {tab === "changes" && <section aria-label="Changed files">
          <div className="mb-3 flex items-center gap-2 text-[11px] text-muted-foreground">
            <FileDiff size={13} aria-hidden="true" />
            <span>{pullRequest.changedFiles} changed files</span>
            <span className="ml-auto font-mono text-success">+{pullRequest.additions}</span>
            <span className="font-mono text-destructive">−{pullRequest.deletions}</span>
          </div>
          {detail.result.files.length ? <div className="space-y-3">{detail.result.files.map(file => <FileChange key={file.path} file={file} defaultOpen={detail.result.files.length <= EAGER_PATCH_FILE_LIMIT} fullDiffUrl={`${summary.url}/files`} />)}</div> : <p className="text-[12px] text-muted-foreground">No changed files reported.</p>}
        </section>}

        {tab === "conversation" && <>
        <section aria-label="Description">
          <h3 className="mb-2 text-[12px] font-medium text-muted-foreground">DESCRIPTION</h3>
          {pullRequest.body ? <GithubMarkdown text={pullRequest.body} /> : <p className="text-[12px] italic text-muted-foreground">No description provided.</p>}
        </section>

        <section aria-label="Conversation comments">
          <h3 className="mb-2 text-[12px] font-medium text-muted-foreground">CONVERSATION</h3>
          {pullRequest.comments.length ? <CommentList comments={pullRequest.comments} /> : <p className="text-[12px] text-muted-foreground">No conversation comments.</p>}
        </section>

        <section aria-label="Review threads">
          <h3 className="mb-2 text-[12px] font-medium text-muted-foreground">REVIEW THREADS</h3>
          {reviewThreads.length ? <div className="space-y-2.5">
            {reviewThreads.map(thread => {
              const rootId = thread.comments[0]?.databaseId ?? null;
              const line = thread.line ?? thread.originalLine ?? undefined;
              return <article key={thread.id} className="overflow-hidden rounded-lg border border-border bg-card">
                <div className="flex items-center gap-2 border-b border-border/70 px-3 py-1.5">
                  <button
                    type="button"
                    onClick={() => onJumpToFile(thread.path, line === undefined ? undefined : Number(line), summary.headBranch)}
                    title={checkedOutHere ? "Open in the editor at this line" : `Open in the editor — ${summary.headBranch} isn’t checked out here`}
                    className="group inline-flex min-w-0 items-center gap-1.5 rounded-md px-1 py-0.5 font-mono text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                  >
                    <span className="truncate">{thread.path}{line !== undefined ? `:${line}` : ""}</span>
                    <SquareArrowOutUpRight size={11} className="shrink-0 opacity-0 transition-opacity group-hover:opacity-100" aria-hidden="true" />
                  </button>
                  <span className="ml-auto flex shrink-0 items-center gap-1.5">
                    {thread.isOutdated && <Badge variant="outline" size="sm">Outdated</Badge>}
                    {thread.isResolved && <Badge variant="success" size="sm">Resolved</Badge>}
                  </span>
                </div>
                <div className="space-y-2.5 px-3 py-2.5">
                  {thread.comments.map(comment => <div key={comment.id}>
                    <p className="flex items-baseline gap-2 text-[11px]">
                      <span className="font-medium text-foreground">{comment.author?.login ?? "ghost"}</span>
                      <CommentTime iso={comment.createdAt} />
                    </p>
                    <div className="mt-0.5"><GithubMarkdown text={comment.body} /></div>
                  </div>)}
                  {rootId !== null && <button
                    type="button"
                    disabled={busy}
                    onClick={() => setPending({ statement: `reply to a review comment on PR #${number} on ${repoLabel}`, requiresBody: true, build: body => ({ kind: "reply", number, commentId: rootId, body }) })}
                    className="inline-flex min-h-7 items-center gap-1 rounded-md px-2 py-1 text-[11px] font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-50"
                  >
                    <MessageSquare size={11} aria-hidden="true" /> Reply
                  </button>}
                </div>
              </article>;
            })}
          </div> : <p className="text-[12px] text-muted-foreground">No review threads.</p>}
        </section>
        </>}
      </div>
    </div>

    {pending && <ConfirmOverlay
      statement={pending.statement}
      requiresBody={pending.requiresBody}
      body={pendingBody}
      onBody={setPendingBody}
      busy={busy}
      onCancel={closeOverlays}
      onConfirm={() => void submit(pending.build(pendingBody))}
    />}

    {checkoutOpen && <ConfirmOverlay
      statement={`check out PR #${number} (${summary.headBranch}) into a task worktree`}
      note="Creates an isolated worktree beside this one — your current checkout is untouched. Repeating it reuses the same worktree."
      requiresBody={false}
      body=""
      onBody={() => undefined}
      busy={busy}
      confirmLabel="Check out"
      onCancel={closeOverlays}
      onConfirm={() => void runCheckout()}
    />}

    {mergeOpen && <Dialog open onOpenChange={next => { if (!next && !busy) closeOverlays(); }}>
      <DialogPopup showCloseButton={false} aria-label="Confirm merge" className="max-w-md p-5">
        <h3 className="mb-2 font-display text-sm font-semibold">Merge pull request</h3>
        {!mergeConfig ? <p className="flex items-center gap-1 text-xs text-muted-foreground"><LoaderCircle className="animate-spin" size={12} aria-hidden="true" /> Reading repository settings…</p> : <>
          <fieldset className="space-y-1.5">
            {(["merge", "squash", "rebase"] as const).filter(name => mergeConfig.strategies[name]).map(name => <label key={name} className="flex items-center gap-2 text-xs">
              <input type="radio" name="merge-strategy" value={name} checked={strategy === name} onChange={() => setStrategy(name)} />
              {STRATEGY_LABEL[name]}
            </label>)}
          </fieldset>
          <p className="mt-3 text-xs text-muted-foreground">This runs <code className="font-mono text-foreground/90">merge PR #{number} ({strategy}) on {repoLabel}</code>.</p>
          <div className="mt-4 flex justify-end gap-2">
            <button type="button" onClick={closeOverlays} disabled={busy} className="min-h-8 rounded-lg px-3 text-[13px] font-medium hover:bg-accent disabled:opacity-50">Cancel</button>
            <button type="button" onClick={() => strategy && void submit({ kind: "merge", number, strategy })} disabled={busy || !strategy} className="min-h-8 rounded-lg bg-success/20 px-3 text-[13px] font-medium text-success hover:bg-success/30 disabled:opacity-50">Merge</button>
          </div>
        </>}
      </DialogPopup>
    </Dialog>}
  </div>;
}

function IssueDetail({ workspaceId, repository, number, detail, error, availableLabels, onBack, onActed }: {
  workspaceId: string;
  repository?: GithubRepository | null;
  number: number;
  detail?: GithubIssueResult;
  error?: string;
  availableLabels: GithubLabel[];
  onBack: () => void;
  onActed: () => void;
}) {
  const [pending, setPending] = useState<PendingAction>();
  const [labelsOpen, setLabelsOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string>();
  const repoLabel = repository ? `${repository.owner}/${repository.name}` : "this repository";
  const header = <div className="flex shrink-0 items-center border-b border-border px-2 py-1.5"><button type="button" onClick={onBack} className="inline-flex min-h-7 items-center gap-1.5 rounded-md px-2 py-1 text-[12px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"><ArrowLeft size={12.5} aria-hidden="true" /> All issues</button></div>;

  const submit = async (action: GithubAction) => {
    setBusy(true); setActionError(undefined);
    try {
      const outcome = await bridgeApi.githubAct(workspaceId, action, true);
      if (!outcome.executed) { setActionError(outcome.message); return; }
      setPending(undefined); setLabelsOpen(false); onActed();
    } catch (value) {
      setActionError(value instanceof Error ? value.message : String(value));
    } finally { setBusy(false); }
  };

  if (error && !detail) return <div className="flex min-h-0 flex-1 flex-col">{header}<PaneNotice icon={CircleX} title={`Issue #${number} did not load`}>{error}</PaneNotice></div>;
  if (!detail) return <div className="flex min-h-0 flex-1 flex-col">{header}<DetailSkeleton label={`Opening issue #${number}`} /></div>;
  const { issue } = detail;

  return <div className="relative flex min-h-0 flex-1 flex-col">
    {header}
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="border-b border-border px-4 py-3.5">
        <div className="flex items-center gap-2 text-[11px] text-muted-foreground"><CircleDot size={13} className="text-success" aria-hidden="true" /><span>#{issue.summary.number}</span><span className="capitalize">{issue.summary.state}</span></div>
        <h2 className="mt-1.5 font-display text-[15px] font-semibold leading-snug text-foreground">{issue.summary.title}</h2>
        <p className="mt-1 text-[11px] text-muted-foreground">Opened by {issue.summary.author?.login ?? "ghost"}</p>
        <div className="mt-2 flex flex-wrap items-center gap-1.5">{issue.summary.labels.map(label => <LabelBadge key={label.name} label={label} />)}<button type="button" onClick={() => setLabelsOpen(value => !value)} className="inline-flex min-h-7 items-center gap-1 rounded-md px-2 py-1 text-[11px] text-muted-foreground hover:bg-accent hover:text-foreground"><Tag size={10.5} aria-hidden="true" /> Manage labels</button></div>
        {labelsOpen && <LabelControls available={availableLabels} current={issue.summary.labels} disabled={busy} onChange={(label, operation) => setPending({
          statement: `${operation === "add" ? "add" : "remove"} label ${JSON.stringify(label)} ${operation === "add" ? "to" : "from"} issue #${number} on ${repoLabel}`,
          requiresBody: false,
          build: () => ({ kind: "label", target: "issue", number, label, operation }),
        })} />}
      </div>
      {actionError && <p role="alert" className="border-b border-border bg-destructive/10 px-4 py-2 text-[12px] text-destructive">{actionError}</p>}
      <div className="space-y-5 px-4 py-4">
        <section aria-label="Issue description"><h3 className="mb-2 text-[12px] font-medium text-muted-foreground">DESCRIPTION</h3>{issue.body ? <GithubMarkdown text={issue.body} /> : <p className="text-[12px] italic text-muted-foreground">No description provided.</p>}</section>
        <section aria-label="Issue comments"><h3 className="mb-2 text-[12px] font-medium text-muted-foreground">COMMENTS</h3>{issue.comments.length ? <CommentList comments={issue.comments} /> : <p className="text-[12px] text-muted-foreground">No comments.</p>}</section>
      </div>
    </div>
    {pending && <ConfirmOverlay statement={pending.statement} requiresBody={false} body="" onBody={() => undefined} busy={busy} onCancel={() => setPending(undefined)} onConfirm={() => void submit(pending.build(""))} />}
  </div>;
}

type ConfirmOverlayProps = {
  statement: string;
  note?: string;
  requiresBody: boolean;
  body: string;
  onBody: (value: string) => void;
  busy: boolean;
  confirmLabel?: string;
  onCancel: () => void;
  onConfirm: () => void;
};

function ConfirmOverlay({ statement, note, requiresBody, body, onBody, busy, confirmLabel = "Confirm", onCancel, onConfirm }: ConfirmOverlayProps) {
  const ready = !requiresBody || body.trim().length > 0;
  return <Dialog open onOpenChange={next => { if (!next && !busy) onCancel(); }}>
    <DialogPopup showCloseButton={false} aria-label="Confirm action" className="max-w-md p-5">
      <p className="text-xs text-muted-foreground">This runs</p>
      <p className="mt-1 font-mono text-xs text-foreground/90">{statement}</p>
      {note && <p className="mt-2 text-[11px] leading-relaxed text-muted-foreground">{note}</p>}
      {requiresBody && <textarea value={body} onChange={event => onBody(event.target.value)} aria-label="Comment body" placeholder="Write a comment…" className="mt-4 min-h-28 w-full resize-y rounded-lg border border-input bg-card p-3 text-[13px] leading-relaxed" />}
      <div className="mt-4 flex justify-end gap-2">
        <button type="button" onClick={onCancel} disabled={busy} className="min-h-8 rounded-lg px-3 text-[13px] font-medium hover:bg-accent disabled:opacity-50">Cancel</button>
        <button type="button" onClick={onConfirm} disabled={busy || !ready} className="min-h-8 rounded-lg bg-primary px-3 text-[13px] font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50">{confirmLabel}</button>
      </div>
    </DialogPopup>
  </Dialog>;
}
