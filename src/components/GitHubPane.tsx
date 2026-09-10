import { Dialog, DialogPopup } from "@/components/ui/dialog";
import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { motion } from "framer-motion";
import {
  ArrowLeft,
  Ban,
  Check,
  ChevronRight,
  CircleCheck,
  CircleDashed,
  CircleDot,
  CircleSlash,
  CircleX,
  Copy,
  ExternalLink,
  FileDiff,
  FolderGit2,
  GitBranch,
  GitCommitHorizontal,
  GitMerge,
  GitPullRequest,
  GitPullRequestDraft,
  Info,
  ListTodo,
  LoaderCircle,
  MessageSquare,
  RefreshCw,
  RotateCcw,
  Search,
  Send,
  Sparkles,
  SquareArrowOutUpRight,
  Tag,
  TriangleAlert,
  X,
} from "lucide-react";
import { bridgeApi } from "../api";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Markdown } from "./Markdown";
import { relativeTime } from "./workDashboard";
import { normalizeGithubMarkdown, splitGithubDetails } from "./githubMarkdown";
import {
  checksNeedPolling,
  filterIssues,
  filterPullRequests,
  groupChecksByWorkflow,
  rollupState,
  PULL_REQUEST_FACETS,
  type PullRequestFacet,
  type PullRequestListItem,
  type RollupState,
} from "../githubSurface";
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
type PullRequestTab = "conversation" | "changes" | "commits" | "checks";

const ROLLUP: Record<RollupState, { icon: typeof CircleCheck; className: string; live?: boolean; label: string }> = {
  failing: { icon: CircleX, className: "text-destructive", label: "Failing" },
  running: { icon: CircleDashed, className: "text-warning", live: true, label: "Running" },
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

/** A check's glyph and tone. Nothing rotates: `live` breathes on the app's
 * shared cadence instead, so twenty queued checks read as one calm state
 * rather than twenty spinners at twenty phases. */
function checkTone(conclusion: string | null | undefined, status: string) {
  if (status !== "completed") return { icon: CircleDashed, className: "text-warning", live: true };
  switch (conclusion) {
    case "success": return { icon: CircleCheck, className: "text-success", live: false };
    case "failure": case "timedOut": case "startupFailure": return { icon: CircleX, className: "text-destructive", live: false };
    case "skipped": case "cancelled": case "neutral": return { icon: CircleSlash, className: "text-muted-foreground", live: false };
    default: return { icon: CircleDashed, className: "text-muted-foreground", live: false };
  }
}

/** Copy one string to the clipboard and say so for a beat. Every identifier on
 * this surface — a PR URL, a branch name, a commit SHA — is something a reader
 * wants in a terminal or a message a second later, and the pane used to offer
 * no way to get it out. */
function CopyButton({ value, label, className, children }: {
  value: string;
  label: string;
  className?: string;
  children?: React.ReactNode;
}) {
  const [copied, setCopied] = useState(false);
  const timer = useRef<number>();
  useEffect(() => () => window.clearTimeout(timer.current), []);
  const copy = () => {
    const done = () => {
      setCopied(true);
      window.clearTimeout(timer.current);
      timer.current = window.setTimeout(() => setCopied(false), 1400);
    };
    const write = navigator.clipboard?.writeText(value);
    if (write) void write.then(done).catch(() => undefined);
    else done();
  };
  return <button
    type="button"
    onClick={copy}
    aria-label={copied ? `${label} copied` : label}
    title={copied ? "Copied" : label}
    className={cn(
      "inline-flex min-h-7 shrink-0 items-center gap-1.5 rounded-md px-1.5 text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground",
      className,
    )}
  >
    {copied
      ? <Check size={13} strokeWidth={1.7} className="text-success" aria-hidden="true" />
      : <Copy size={13} strokeWidth={1.7} aria-hidden="true" />}
    {children}
  </button>;
}

const HEADER_ICON =
  "inline-flex size-7 shrink-0 items-center justify-center rounded-md p-0 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground";

/** Open something on github.com. Always an anchor, never a button dressed as
 * one, so the webview's own "copy link" still works. */
function OpenOnGithub({ url, what, className }: { url: string; what: string; className?: string }) {
  return <a
    href={url}
    target="_blank"
    rel="noreferrer"
    aria-label={`Open ${what} on GitHub`}
    title="Open on GitHub"
    className={cn(HEADER_ICON, className)}
  ><SquareArrowOutUpRight size={13} strokeWidth={1.7} aria-hidden="true" /></a>;
}

/** Centered empty/availability state — the pane never renders blank. */
function PaneNotice({ icon: Icon, title, spin, children }: { icon: typeof GitPullRequest; title: string; spin?: boolean; children?: React.ReactNode }) {
  return <div className="animate-page-mount flex h-full flex-col items-center justify-center gap-2 px-8 text-center">
    <span className="u-glass-soft grid size-11 place-items-center rounded-xl border border-border text-muted-foreground"><Icon size={18} strokeWidth={1.6} className={cn(spin && "animate-spin")} aria-hidden="true" /></span>
    <p className="mt-1 font-display text-[13px] font-semibold text-foreground">{title}</p>
    <div className="max-w-[26rem] text-[12px] leading-relaxed text-muted-foreground">{children}</div>
  </div>;
}

/** GitHub-authored markdown at the pane's compact type scale.
 *
 * Two things happen before the app's renderer sees the text: GitHub-only
 * spellings are normalized (see `githubMarkdown.ts`), and `<details>` sections
 * become real disclosures. Rendering itself still goes through the app's
 * Markdown component — React text nodes only, with fenced HTML confined to a
 * fully sandboxed iframe — so remote content stays inert. */
function GithubMarkdown({ text, repositoryUrl }: { text: string; repositoryUrl?: string }) {
  const blocks = useMemo(() => splitGithubDetails(text), [text]);
  return <div className="gh-md">
    {blocks.map((block, index) => block.kind === "details"
      ? <details key={index} className="group my-1.5 overflow-hidden rounded-lg border border-border bg-card">
        <summary className="flex cursor-pointer list-none items-center gap-1.5 px-2.5 py-1.5 text-[12px] font-medium text-foreground transition-colors hover:bg-accent/50">
          <ChevronRight size={12} aria-hidden="true" className="shrink-0 text-muted-foreground transition-transform duration-200 group-open:rotate-90" />
          <span className="min-w-0 flex-1 truncate">{block.summary}</span>
        </summary>
        <div className="border-t border-border px-2.5 py-2">
          <Markdown text={normalizeGithubMarkdown(block.body, repositoryUrl)} />
        </div>
      </details>
      : <Markdown key={index} text={normalizeGithubMarkdown(block.text, repositoryUrl)} />)}
  </div>;
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
    <div className="divide-y divide-border overflow-hidden rounded-xl border border-border bg-card">
      {SKELETON_WIDTHS.map((width, index) => <div key={index} className="flex items-start gap-2.5 px-3 py-3">
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

/** The check rollup as a bar rather than a number pair. Segments are widths,
 * so a re-read animates from the old proportions to the new ones and a reader
 * sees CI move without reading digits. */
function RollupMeter({ checks, className }: { checks: PullRequestListItem["checks"]; className?: string }) {
  if (!checks.total) return null;
  const running = checks.queued + checks.inProgress;
  const segments = [
    { key: "passed", count: checks.passed, className: "bg-success/70" },
    { key: "failed", count: checks.failed, className: "bg-destructive/80" },
    { key: "running", count: running, className: "bg-warning/80 github-check-live" },
    { key: "other", count: checks.skipped + checks.cancelled, className: "bg-muted-foreground/40" },
  ].filter(segment => segment.count > 0);
  return <span className={cn("flex h-1 overflow-hidden rounded-full bg-muted", className)} aria-hidden="true">
    {segments.map(segment => <span
      key={segment.key}
      className={cn("h-full transition-[width] duration-500 ease-out", segment.className)}
      style={{ width: `${(segment.count / checks.total) * 100}%` }}
    />)}
  </span>;
}

/** A repository label, in the colour the repository picked. The dot carries the
 * colour and the text stays on the chrome's own tokens: an achromatic pane
 * should not sprout six saturated pills the moment a PR is triaged. */
function LabelBadge({ label }: { label: GithubLabel }) {
  const color = /^[\da-f]{3,8}$/i.test(label.color) ? `#${label.color}` : undefined;
  return <span
    title={label.description || label.name}
    className="inline-flex max-w-[12rem] items-center gap-1.5 rounded-full border border-border bg-card px-2 py-0.5 text-[11px] text-muted-foreground"
  >
    <span className="size-2 shrink-0 rounded-full bg-muted-foreground" style={color ? { backgroundColor: color } : undefined} aria-hidden="true" />
    <span className="min-w-0 truncate">{label.name}</span>
  </span>;
}

function LabelControls({ available, current, disabled, onChange }: {
  available: GithubLabel[];
  current: GithubLabel[];
  disabled: boolean;
  onChange: (label: string, operation: "add" | "remove") => void;
}) {
  const selected = new Set(current.map(label => label.name));
  return <div className="animate-page-mount mt-2 grid max-h-56 gap-1 overflow-y-auto rounded-lg border border-border bg-card p-1.5" aria-label="Repository labels">
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

/** The list search field. One control for both lists; the placeholder says
 * which one it is filtering. */
function ListSearch({ value, onChange, placeholder }: { value: string; onChange: (next: string) => void; placeholder: string }) {
  return <label className="u-glass-soft flex min-w-0 flex-1 items-center gap-1.5 rounded-md border border-border px-2 py-1">
    <Search size={11.5} className="shrink-0 text-muted-foreground" aria-hidden="true" />
    <input
      type="search"
      value={value}
      onChange={event => onChange(event.target.value)}
      placeholder={placeholder}
      aria-label={placeholder}
      className="min-w-0 flex-1 bg-transparent text-[12px] text-foreground outline-none placeholder:text-muted-foreground [&::-webkit-search-cancel-button]:hidden"
    />
    {value && <button type="button" onClick={() => onChange("")} aria-label="Clear the filter" className="grid size-4 shrink-0 place-items-center rounded text-muted-foreground transition-colors hover:text-foreground">
      <X size={10} aria-hidden="true" />
    </button>}
  </label>;
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
  const [query, setQuery] = useState("");
  const [facet, setFacet] = useState<PullRequestFacet>("all");

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
  const repositoryUrl = repositoryOverview?.url
    ?? (status?.repository ? `https://${status.repository.host}/${status.repository.owner}/${status.repository.name}` : undefined);
  const visiblePrs = useMemo(() => prs && filterPullRequests(prs, query, facet), [prs, query, facet]);
  const visibleIssues = useMemo(() => issues && filterIssues(issues, query), [issues, query]);
  const inDetail = (surfaceTab === "pulls" && selected !== undefined) || (surfaceTab === "issues" && selectedIssue !== undefined);
  const showsFilters = !inDetail && surfaceTab !== "repository" && status?.availability.status === "available";

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
      repositoryUrl={repositoryUrl}
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
  } else if (surfaceTab === "pulls" && visiblePrs?.length === 0) {
    body = <PaneNotice icon={Search} title="Nothing matches this filter">{prs?.length} open pull request{prs?.length === 1 ? "" : "s"} — none of them match. <button type="button" onClick={() => { setQuery(""); setFacet("all"); }} className="text-foreground underline decoration-dotted underline-offset-2">Clear the filter</button>.</PaneNotice>;
  } else if (surfaceTab === "pulls" && visiblePrs) {
    body = <div className="min-h-0 flex-1 overflow-y-auto px-3 py-3 sm:px-4">
      <div className="divide-y divide-border overflow-hidden rounded-xl border border-border bg-card">
        {visiblePrs.map((pr, index) => <PullRequestRow key={pr.number} pr={pr} index={index} onOpen={() => void openDetail(pr.number)} />)}
      </div>
    </div>;
  } else if (surfaceTab === "issues" && selectedIssue !== undefined) {
    body = <IssueDetail
      workspaceId={workspaceId}
      repository={status.repository}
      repositoryUrl={repositoryUrl}
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
  } else if (surfaceTab === "issues" && visibleIssues?.length === 0) {
    body = <PaneNotice icon={Search} title="Nothing matches this filter">No open issue matches “{query}”.</PaneNotice>;
  } else if (surfaceTab === "issues" && visibleIssues) {
    body = <IssueList issues={visibleIssues} onOpen={number => void openIssue(number)} />;
  } else if (tabErrors.repository && !repositoryOverview) {
    body = <PaneNotice icon={CircleX} title="Repository did not load">{tabErrors.repository}</PaneNotice>;
  } else if (!repositoryOverview) {
    body = <DetailSkeleton label="Loading repository" />;
  } else {
    body = <RepositoryOverview overview={repositoryOverview} />;
  }

  return <section className="relative flex h-full w-full flex-col" aria-label="GitHub repository">
    <header className="u-glass flex shrink-0 flex-col gap-1.5 border-b border-border px-2 py-1.5">
      <div className="flex h-8 items-center gap-1.5">
        <div className="flex min-w-0 flex-1 items-center gap-1.5">
          <span className="inline-flex size-7 shrink-0 items-center justify-center text-muted-foreground">
            <FolderGit2 size={13} strokeWidth={1.7} aria-hidden="true" />
          </span>
          {repoLabel && <CopyButton value={repoLabel} label={`Copy ${repoLabel}`} className="h-7 min-w-0 shrink px-1.5">
            <span className="min-w-0 truncate font-mono text-[11px]">{repoLabel}</span>
          </CopyButton>}
        </div>
        <div className="u-segmented flex shrink-0 items-center p-0.5" role="toolbar" aria-label="GitHub repository actions">
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
            onClick={() => { setSurfaceTab(id); setSelected(undefined); setSelectedIssue(undefined); setQuery(""); }}
            className={cn("u-segmented-item", HEADER_ICON)}
          ><Icon size={13} strokeWidth={1.7} aria-hidden="true" /></button>)}
          {repositoryUrl && <OpenOnGithub url={repositoryUrl} what={repoLabel ?? "the repository"} />}
          <button
            type="button"
            onClick={() => { void loadSurface(true); if (selected !== undefined) void openDetail(selected, true); if (selectedIssue !== undefined) void openIssue(selectedIssue, true); }}
            aria-label="Refresh GitHub"
            title="Refresh"
            className={HEADER_ICON}
          >
            <RefreshCw size={13} strokeWidth={1.7} className={cn(refreshing && "animate-spin")} aria-hidden="true" />
          </button>
        </div>
      </div>
      {showsFilters && <div className="flex items-center gap-1.5 pb-0.5">
        <ListSearch value={query} onChange={setQuery} placeholder={surfaceTab === "pulls" ? "Filter pull requests" : "Filter issues"} />
        {surfaceTab === "pulls" && <div className="u-segmented flex shrink-0 p-0.5" role="group" aria-label="Pull request filters">
          {PULL_REQUEST_FACETS.map(choice => <button
            key={choice.id}
            type="button"
            aria-pressed={facet === choice.id}
            data-active={facet === choice.id}
            onClick={() => setFacet(choice.id)}
            className="u-segmented-item rounded-md px-2 py-1 text-[11px] text-muted-foreground"
          >{choice.label}</button>)}
        </div>}
      </div>}
    </header>
    {tabErrors[surfaceTab] && ((surfaceTab === "pulls" && prs) || (surfaceTab === "issues" && issues) || (surfaceTab === "repository" && repositoryOverview)) && <p role="alert" className="border-b border-border bg-warning/10 px-4 py-2 text-[11px] text-warning">Refresh failed: {tabErrors[surfaceTab]}</p>}
    {body}
  </section>;
}

function PullRequestRow({ pr, index, onOpen }: { pr: PullRequestListItem; index: number; onOpen: () => void }) {
  const rollup = ROLLUP[rollupState(pr)];
  const RollupIcon = rollup.icon;
  const review = REVIEW[pr.reviewDecision];
  const StateIcon = pr.isDraft ? GitPullRequestDraft : pr.state === "merged" ? GitMerge : GitPullRequest;
  return <div
    className="github-row-enter group relative flex items-start transition-colors hover:bg-accent/40"
    style={{ "--github-row-delay": `${Math.min(index, 12) * 28}ms` } as React.CSSProperties}
  >
    <button type="button" onClick={onOpen} className="flex min-w-0 flex-1 items-start gap-2.5 px-3 py-3 text-left">
      <StateIcon size={15} strokeWidth={1.8} aria-hidden="true" className={cn("mt-0.5 shrink-0", pr.isDraft ? "text-muted-foreground" : pr.state === "merged" ? "text-info" : "text-success")} />
      <span className="min-w-0 flex-1">
        <span className="flex flex-wrap items-start gap-x-1.5 gap-y-1">
          <span className="min-w-0 flex-1 text-[13px] font-medium leading-5 text-foreground">{pr.title}</span>
          {pr.mergeability === "conflicting" && <Badge variant="warning" size="sm">Conflicts</Badge>}
          {pr.isDraft && <Badge variant="outline" size="sm">Draft</Badge>}
          {review && <Badge variant={review.variant} size="sm">{review.label}</Badge>}
          {pr.checks.total > 0 && <span className={cn("inline-flex shrink-0 items-center gap-1 font-mono text-[11px] tabular-nums", rollup.className)} title={`${rollup.label} — ${pr.checks.passed}/${pr.checks.total} checks passed`}>
            <RollupIcon size={12} aria-hidden="true" className={cn(rollup.live && "github-check-live")} />
            {pr.checks.passed}/{pr.checks.total}
          </span>}
        </span>
        <span className="mt-1 flex items-center gap-1.5 pr-14 text-[11px] text-muted-foreground">
          <span className="shrink-0 tabular-nums">#{pr.number}</span>
          <span aria-hidden="true">·</span>
          <span className="truncate font-mono">{pr.headBranch}</span>
          <span aria-hidden="true">·</span>
          <span className="shrink-0">{pr.author?.login ?? "ghost"}</span>
        </span>
        {pr.checks.total > 0 && <RollupMeter checks={pr.checks} className="mt-2 w-full max-w-40" />}
      </span>
    </button>
    {/* Absolute, so the affordances cost the title no width until a pointer
        (or the keyboard) is actually on the row. */}
    <span className="u-glass-soft absolute bottom-2.5 right-1.5 flex items-center rounded-md opacity-0 transition-opacity focus-within:opacity-100 group-hover:opacity-100">
      <CopyButton value={pr.url} label={`Copy the link to #${pr.number}`} />
      <OpenOnGithub url={pr.url} what={`#${pr.number}`} />
    </span>
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
    <div className={cn("flex items-center transition-colors hover:bg-accent/40", open && "border-b border-border")}>
      <button type="button" onClick={() => setOpen(value => !value)} aria-expanded={open} className="flex min-w-0 flex-1 items-center gap-2 px-3 py-2 text-left">
        <ChevronRight size={11} aria-hidden="true" className={cn("shrink-0 text-muted-foreground transition-transform duration-200", open && "rotate-90")} />
        <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-foreground">{file.path}</span>
        <Badge variant="outline" size="sm">{file.status}</Badge>
        <span className="font-mono text-[11px] text-success">+{file.additions}</span>
        <span className="font-mono text-[11px] text-destructive">−{file.deletions}</span>
      </button>
      <CopyButton value={file.path} label={`Copy the path ${file.path}`} className="mr-1.5" />
    </div>
    {open && (file.patch ? <PatchView patch={file.patch} fullDiffUrl={fullDiffUrl} /> : <p className="px-3 py-4 text-center text-[12px] text-muted-foreground">Binary file or patch unavailable from GitHub.</p>)}
  </article>;
}

function CommentList({ comments, repositoryUrl }: {
  comments: Array<{ id: string; author?: { login: string } | null; body: string; createdAt: string; url: string }>;
  repositoryUrl?: string;
}) {
  return <div className="space-y-2">{comments.map(comment => <article key={comment.id} className="group overflow-hidden rounded-lg border border-border bg-card">
    <div className="flex items-baseline gap-2 border-b border-border/60 px-3 py-1.5">
      <span className="text-[11px] font-medium text-foreground">{comment.author?.login ?? "ghost"}</span>
      <CommentTime iso={comment.createdAt} />
      <span className="ml-auto flex shrink-0 items-center opacity-0 transition-opacity focus-within:opacity-100 group-hover:opacity-100">
        <CopyButton value={comment.url} label="Copy the link to this comment" />
        <OpenOnGithub url={comment.url} what="this comment" />
      </span>
    </div>
    <div className="px-3 py-2"><GithubMarkdown text={comment.body} repositoryUrl={repositoryUrl} /></div>
  </article>)}</div>;
}

/** Checks, grouped by the workflow that produced them. Each group states its
 * own tally so a red workflow is findable without reading every row. */
function ChecksList({ checks, rollup }: { checks: GithubChecksResult["checks"]; rollup?: PullRequestListItem["checks"] }) {
  const groups = useMemo(() => groupChecksByWorkflow(checks), [checks]);
  return <section aria-label="Checks" className="space-y-3">
    {rollup && rollup.total > 0 && <div className="rounded-lg border border-border bg-card px-3 py-2.5">
      <p className="flex items-center gap-2 text-[11px] text-muted-foreground">
        <span className="font-medium text-foreground">{rollup.passed}/{rollup.total} passed</span>
        {rollup.failed > 0 && <span className="text-destructive">{rollup.failed} failed</span>}
        {rollup.queued + rollup.inProgress > 0 && <span className="text-warning">{rollup.queued + rollup.inProgress} running</span>}
        {rollup.skipped + rollup.cancelled > 0 && <span>{rollup.skipped + rollup.cancelled} skipped</span>}
      </p>
      <RollupMeter checks={rollup} className="mt-2" />
    </div>}
    {groups.length ? groups.map(group => <div key={group.workflow} className="overflow-hidden rounded-lg border border-border bg-card">
      <p className="border-b border-border/60 px-3 py-1.5 text-[11px] font-medium tracking-[0.06em] text-muted-foreground">{group.workflow.toUpperCase()}</p>
      <div className="divide-y divide-border/60">
        {group.checks.map(check => {
          const tone = checkTone(check.conclusion, check.status);
          const ToneIcon = tone.icon;
          return <div key={`${check.workflow}-${check.name}`} className="flex items-center gap-2.5 px-3 py-2">
            <ToneIcon size={13.5} className={cn("shrink-0", tone.className, tone.live && "github-check-live")} aria-hidden="true" />
            <span className="min-w-0 flex-1 truncate text-[12px] text-foreground">{check.name}</span>
            <span className="shrink-0 text-[11px] text-muted-foreground">{check.conclusion ?? check.status}</span>
            {check.logUrl && <OpenOnGithub url={check.logUrl} what={`logs for ${check.name}`} />}
          </div>;
        })}
      </div>
    </div>) : <p className="text-[12px] text-muted-foreground">No checks reported.</p>}
  </section>;
}

/** The commit list. The pane could show a PR's files and its checks but never
 * its commits, which is the one view that answers "what actually landed on
 * this branch". Commits ride the `pr view` read the detail already pays for. */
function CommitList({ commits, prUrl }: { commits: GithubPullRequestResult["pullRequest"]["commits"]; prUrl: string }) {
  return <section aria-label="Commits">
    <h3 className="mb-2 text-[12px] font-medium text-muted-foreground">COMMITS</h3>
    {commits.length ? <div className="divide-y divide-border overflow-hidden rounded-lg border border-border bg-card">
      {commits.map((commit, index) => <article
        key={commit.oid}
        className="github-row-enter group relative flex items-start gap-2.5 px-3 py-2.5 transition-colors hover:bg-accent/30"
        style={{ "--github-row-delay": `${Math.min(index, 12) * 24}ms` } as React.CSSProperties}
      >
        <GitCommitHorizontal size={14} className="mt-0.5 shrink-0 text-muted-foreground" aria-hidden="true" />
        <div className="min-w-0 flex-1">
          <p className="pr-14 text-[12.5px] font-medium leading-5 text-foreground">{commit.messageHeadline || "(no commit message)"}</p>
          {commit.messageBody.trim() && <p className="mt-0.5 whitespace-pre-line text-[11px] leading-relaxed text-muted-foreground">{commit.messageBody.trim()}</p>}
          <p className="mt-1 flex flex-wrap items-center gap-1.5 text-[11px] text-muted-foreground">
            <span className="font-mono text-foreground/80">{commit.abbreviatedOid}</span>
            {commit.authors.length > 0 && <>
              <span aria-hidden="true">·</span>
              <span className="truncate">{commit.authors.map(author => author.login).join(", ")}</span>
            </>}
            {!Number.isNaN(Date.parse(commit.committedAt)) && <>
              <span aria-hidden="true">·</span>
              <CommentTime iso={commit.committedAt} />
            </>}
          </p>
        </div>
        <span className="u-glass-soft absolute right-1.5 top-2 flex items-center rounded-md opacity-0 transition-opacity focus-within:opacity-100 group-hover:opacity-100">
          <CopyButton value={commit.oid} label={`Copy the SHA ${commit.abbreviatedOid}`} />
          <OpenOnGithub url={`${prUrl}/commits/${commit.oid}`} what={`commit ${commit.abbreviatedOid}`} />
        </span>
      </article>)}
    </div> : <p className="text-[12px] text-muted-foreground">No commits reported for this pull request.</p>}
  </section>;
}

function IssueList({ issues, onOpen }: { issues: GithubIssuesResult["issues"]; onOpen: (number: number) => void }) {
  return <div className="min-h-0 flex-1 overflow-y-auto px-3 py-3 sm:px-4">
    <div className="divide-y divide-border overflow-hidden rounded-xl border border-border bg-card">
      {issues.map((issue, index) => <div
        key={issue.number}
        className="github-row-enter group relative flex items-start transition-colors hover:bg-accent/40"
        style={{ "--github-row-delay": `${Math.min(index, 12) * 28}ms` } as React.CSSProperties}
      >
        <button type="button" onClick={() => onOpen(issue.number)} className="flex min-w-0 flex-1 items-start gap-2.5 px-3 py-3 text-left">
          <CircleDot size={14} className={cn("mt-0.5 shrink-0", issue.state === "open" ? "text-success" : "text-muted-foreground")} aria-hidden="true" />
          <span className="min-w-0 flex-1">
            <span className="block pr-14 text-[13px] font-medium leading-5 text-foreground">{issue.title}</span>
            <span className="mt-1 flex items-center gap-1.5 text-[11px] text-muted-foreground"><span>#{issue.number}</span><span aria-hidden="true">·</span><span>{issue.author?.login ?? "ghost"}</span>{!Number.isNaN(Date.parse(issue.updatedAt)) && <><span aria-hidden="true">·</span><span>updated {relativeTime(issue.updatedAt, new Date())}</span></>}</span>
            {!!issue.labels.length && <span className="mt-1.5 flex flex-wrap gap-1">{issue.labels.map(label => <LabelBadge key={label.name} label={label} />)}</span>}
          </span>
        </button>
        <span className="u-glass-soft absolute right-1.5 top-2.5 flex items-center rounded-md opacity-0 transition-opacity focus-within:opacity-100 group-hover:opacity-100">
          <CopyButton value={issue.url} label={`Copy the link to #${issue.number}`} />
          <OpenOnGithub url={issue.url} what={`#${issue.number}`} />
        </span>
      </div>)}
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
  return <div className="animate-page-mount min-h-0 flex-1 overflow-y-auto px-4 py-4">
    <div className="u-glass-soft rounded-xl border border-border p-4">
      <div className="flex items-start gap-3">
        <span className="grid size-9 shrink-0 place-items-center rounded-lg border border-border bg-card"><FolderGit2 size={16} aria-hidden="true" /></span>
        <div className="min-w-0 flex-1">
          <h2 className="truncate font-display text-base font-semibold text-foreground">{overview.nameWithOwner}</h2>
          <p className="mt-1 text-[12px] leading-relaxed text-muted-foreground">{overview.description || "No repository description."}</p>
        </div>
        <Badge variant="outline" size="sm">{overview.visibility.toLowerCase()}</Badge>
      </div>
      <div className="mt-3 flex flex-wrap items-center gap-1">
        <CopyButton value={overview.url} label="Copy the repository link" className="border border-border bg-card px-2">Copy link</CopyButton>
        <CopyButton value={`git clone ${overview.url}.git`} label="Copy the clone command" className="border border-border bg-card px-2">Copy clone command</CopyButton>
        <OpenOnGithub url={overview.url} what={overview.nameWithOwner} />
      </div>
      <div className="mt-4 grid grid-cols-2 gap-2">{facts.map(([label, value, Icon]) => <div key={label} className="rounded-lg border border-border bg-card px-3 py-2.5"><p className="flex items-center gap-1.5 text-[11px] text-muted-foreground"><Icon size={11} aria-hidden="true" />{label}</p><p className="mt-1 truncate font-mono text-[12px] text-foreground">{value}</p></div>)}</div>
      <section className="mt-4"><h3 className="mb-2 text-[12px] font-medium text-muted-foreground">LABELS</h3><div className="flex flex-wrap gap-1.5">{overview.labels.length ? overview.labels.map(label => <LabelBadge key={label.name} label={label} />) : <p className="text-[12px] text-muted-foreground">No labels.</p>}</div></section>
    </div>
  </div>;
}

// ── Detail ───────────────────────────────────────────────────────────────────

type PendingAction = { statement: string; requiresBody: boolean; body?: string; build: (body: string) => GithubAction };

/** The harnesses a subagent review can run under. The model comes from the
 * Reviewer profile in settings, so the user only picks the agent. Cursor
 * Bugbot is not a local worker: it posts `cursor review` on the PR. */
const REVIEW_HARNESSES: ReadonlyArray<{ id: string; label: string }> = [
  { id: "claude", label: "Claude" },
  { id: "codex", label: "Codex" },
  { id: "opencode", label: "OpenCode" },
  { id: "bugbot", label: "Cursor Bugbot" },
];

const ACTION_BUTTON = "inline-flex min-h-7 items-center gap-1.5 rounded-md border border-border bg-card px-2.5 py-1 text-[12px] font-medium text-foreground transition-colors hover:bg-accent active:scale-[0.98] disabled:opacity-50";

/** A comment composer. Posting still goes through the same confirmation gate
 * as every other write, so the button hands the draft up rather than calling
 * `github/act` itself. */
function CommentComposer({ disabled, onSubmit }: { disabled: boolean; onSubmit: (body: string) => void }) {
  const [body, setBody] = useState("");
  const ready = body.trim().length > 0;
  return <div className="rounded-lg border border-border bg-card p-2">
    <textarea
      value={body}
      onChange={event => setBody(event.target.value)}
      aria-label="New comment"
      placeholder="Write a comment…"
      className="min-h-20 w-full resize-y bg-transparent text-[12.5px] leading-relaxed text-foreground outline-none placeholder:text-muted-foreground"
    />
    <div className="mt-1 flex items-center justify-end gap-2">
      <span className="mr-auto text-[11px] text-muted-foreground">Markdown is supported.</span>
      <button
        type="button"
        disabled={disabled || !ready}
        onClick={() => { onSubmit(body); setBody(""); }}
        className={ACTION_BUTTON}
      ><Send size={11} aria-hidden="true" /> Comment</button>
    </div>
  </div>;
}

/** The detail tab strip, with an underline that slides between tabs rather
 * than cutting. */
function DetailTabs<T extends string>({ id, tabs, active, onSelect, label }: {
  id: string;
  tabs: ReadonlyArray<{ value: T; label: string }>;
  active: T;
  onSelect: (value: T) => void;
  label: string;
}) {
  return <div className="sticky top-0 z-[1] flex border-b border-border bg-background/95 px-4 backdrop-blur" role="tablist" aria-label={label}>
    {tabs.map((tab, index) => <button
      key={tab.value}
      type="button"
      role="tab"
      id={`${id}-${tab.value}`}
      aria-controls={`${id}-panel`}
      aria-selected={active === tab.value}
      tabIndex={active === tab.value ? 0 : -1}
      onClick={() => onSelect(tab.value)}
      onKeyDown={event => {
        let next = index;
        if (event.key === "ArrowRight") next = (index + 1) % tabs.length;
        else if (event.key === "ArrowLeft") next = (index - 1 + tabs.length) % tabs.length;
        else if (event.key === "Home") next = 0;
        else if (event.key === "End") next = tabs.length - 1;
        else return;
        event.preventDefault();
        onSelect(tabs[next].value);
        document.getElementById(`${id}-${tabs[next].value}`)?.focus();
      }}
      className={cn("relative px-2.5 py-2 text-[12px] capitalize transition-colors", active === tab.value ? "text-foreground" : "text-muted-foreground hover:text-foreground")}
    >
      {tab.label}
      {active === tab.value && <motion.span layoutId={`${id}-underline`} className="absolute inset-x-1.5 -bottom-px h-0.5 rounded-full bg-primary" aria-hidden="true" />}
    </button>)}
  </div>;
}

type PullRequestDetailProps = {
  workspaceId: string;
  workspaceBranch: string | null;
  sessionId?: string;
  repository?: GithubRepository | null;
  repositoryUrl?: string;
  number: number;
  detail?: Detail;
  error?: string;
  availableLabels: GithubLabel[];
  onBack: () => void;
  onActed: () => void;
  onJumpToFile: (path: string, line: number | undefined, headBranch: string) => void;
};

function PullRequestDetail({ workspaceId, workspaceBranch, sessionId, repository, repositoryUrl, number, detail, error, availableLabels, onBack, onActed, onJumpToFile }: PullRequestDetailProps) {
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
  const tabs = [
    { value: "conversation" as const, label: "conversation" },
    { value: "changes" as const, label: `changes ${pullRequest.changedFiles}` },
    { value: "commits" as const, label: `commits ${pullRequest.commits.length}` },
    { value: "checks" as const, label: `checks ${detail.checks.checks.length}` },
  ];

  return <div className="relative flex min-h-0 flex-1 flex-col">
    {header}
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="animate-page-mount border-b border-border px-4 pb-3 pt-3.5">
        <div className="flex items-center gap-2 text-[11px] text-muted-foreground">
          <StateIcon size={13} aria-hidden="true" className={cn(summary.isDraft ? "text-muted-foreground" : summary.state === "merged" ? "text-info" : summary.state === "closed" ? "text-destructive" : "text-success")} />
          <span className="tabular-nums">#{summary.number}</span>
          <span className="capitalize">{summary.isDraft ? "draft" : summary.state}</span>
          <span className="truncate">by {summary.author?.login ?? "ghost"}</span>
          {review && <Badge variant={review.variant} size="sm">{review.label}</Badge>}
          <span className="ml-auto flex shrink-0 items-center">
            <CopyButton value={summary.url} label={`Copy the link to #${summary.number}`} />
            <OpenOnGithub url={summary.url} what={`#${summary.number}`} />
          </span>
        </div>
        <h2 className="mt-1.5 font-display text-[15px] font-semibold leading-snug tracking-[-0.01em] text-foreground">{summary.title}</h2>
        <p className="mt-1 flex flex-wrap items-center gap-1 font-mono text-[11px] text-muted-foreground">
          {summary.headBranch} <span aria-hidden="true">→</span> {pullRequest.baseBranch}
          <CopyButton value={summary.headBranch} label={`Copy the branch name ${summary.headBranch}`} />
          {checkedOutHere && <span className="font-sans text-success">· checked out here</span>}
        </p>
        {summary.mergeability === "conflicting" && <p className="mt-2 flex items-start gap-1.5 rounded-md border border-warning/25 bg-warning/10 px-2.5 py-1.5 text-[11px] text-warning">
          <TriangleAlert size={12} className="mt-0.5 shrink-0" aria-hidden="true" />
          <span>This branch has conflicts with {pullRequest.baseBranch}. GitHub reports its merge state as <span className="font-mono">{summary.mergeStateStatus.toLowerCase()}</span>.</span>
        </p>}
        {summary.checks.total > 0 && <div className="mt-2.5">
          <RollupMeter checks={summary.checks} />
        </div>}
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
          <button type="button" onClick={() => void openMerge()} disabled={busy} className="inline-flex min-h-7 items-center gap-1.5 rounded-md border border-success/25 bg-success/10 px-2.5 py-1 text-[12px] font-medium text-success transition-colors hover:bg-success/20 active:scale-[0.98] disabled:opacity-50"><GitMerge size={12} aria-hidden="true" /> Merge</button>
          <button type="button" disabled={busy} className={ACTION_BUTTON} onClick={() => setPending({ statement: `submit approval on PR #${number} on ${repoLabel}`, requiresBody: false, build: () => ({ kind: "review", number, event: "approve", body: "" }) })}>Approve</button>
          <button type="button" disabled={busy} className={ACTION_BUTTON} onClick={() => setPending({ statement: `submit requested changes on PR #${number} on ${repoLabel}`, requiresBody: true, build: body => ({ kind: "review", number, event: "requestChanges", body }) })}>Request changes</button>
          <button type="button" disabled={busy || reviewBusy} aria-haspopup="menu" aria-expanded={reviewOpen} className={ACTION_BUTTON} onClick={() => { setReviewNotice(undefined); setReviewOpen(value => !value); }}>
            <Sparkles size={12} aria-hidden="true" /> {reviewBusy ? "Starting review…" : "Review"}
          </button>
          {summary.isDraft && <button type="button" disabled={busy} className={ACTION_BUTTON} onClick={() => setPending({ statement: `mark PR #${number} ready for review on ${repoLabel}`, requiresBody: false, build: () => ({ kind: "ready", number }) })}>Ready for review</button>}
          {rerunnable && <button type="button" disabled={busy} className={ACTION_BUTTON} onClick={() => setPending({ statement: `re-run failed checks on PR #${number} on ${repoLabel}`, requiresBody: false, build: () => ({ kind: "rerun", number }) })}><RotateCcw size={11.5} aria-hidden="true" /> Re-run failed</button>}
          {!checkedOutHere && <button type="button" disabled={busy} className={ACTION_BUTTON} onClick={() => { setActionError(undefined); setCheckoutOpen(true); }}>
            <FolderGit2 size={12} aria-hidden="true" /> Check out
          </button>}
          <button type="button" disabled={busy} className={ACTION_BUTTON} onClick={() => setPending({ statement: `close PR #${number} on ${repoLabel}`, requiresBody: false, build: () => ({ kind: "setState", target: "pullRequest", number, operation: "close" }) })}><Ban size={11.5} aria-hidden="true" /> Close</button>
          </div>
          {reviewOpen && <div className="animate-page-mount mt-2 grid gap-1 rounded-lg border border-border bg-card p-1.5" role="menu" aria-label="Review harness">
            <p className="px-2 pb-0.5 pt-1 text-[11px] font-semibold tracking-[0.1em] text-muted-foreground">RUN REVIEW WITH</p>
            {REVIEW_HARNESSES.map(choice => <button key={choice.id} type="button" role="menuitem" disabled={reviewBusy} onClick={() => void startReview(choice.id)} className="flex items-center gap-2 rounded-md px-2 py-1 text-left text-[12px] text-foreground transition-colors hover:bg-accent disabled:opacity-50">
              <Sparkles size={11} className="text-muted-foreground" aria-hidden="true" />
              <span className="min-w-0 flex-1 truncate">{choice.label}</span>
            </button>)}
          </div>}
          {reviewNotice && <p role="status" className={cn("animate-page-mount mt-2 rounded-md border px-2.5 py-1.5 text-[12px]", reviewNotice.tone === "success" ? "border-success/25 bg-success/10 text-success" : "border-destructive/25 bg-destructive/10 text-destructive")}>{reviewNotice.text}</p>}
        </>}
        {summary.state === "closed" && <div className="mt-3 flex flex-wrap items-center gap-1.5">
          <button type="button" disabled={busy} className={ACTION_BUTTON} onClick={() => setPending({ statement: `reopen PR #${number} on ${repoLabel}`, requiresBody: false, build: () => ({ kind: "setState", target: "pullRequest", number, operation: "reopen" }) })}><RotateCcw size={11.5} aria-hidden="true" /> Reopen</button>
        </div>}
      </div>

      {actionError && <p role="alert" className="border-b border-border bg-destructive/10 px-4 py-2 text-[12px] text-destructive">{actionError}</p>}
      {error && <p role="alert" className="border-b border-border bg-warning/10 px-4 py-2 text-[12px] text-warning">Live refresh failed: {error}</p>}
      {checkout && <p className="border-b border-border bg-success/10 px-4 py-2 text-[12px] text-success">
        {checkout.reused ? "Reusing the task worktree already on " : "Checked out into a task worktree on "}
        <span className="font-mono">{checkout.branch}</span>
        <span className="block truncate font-mono text-[11px] text-success/80" title={checkout.path}>{checkout.path}</span>
      </p>}

      <DetailTabs id={detailId} tabs={tabs} active={tab} onSelect={setTab} label="Pull request detail" />

      <div id={`${detailId}-panel`} role="tabpanel" aria-labelledby={`${detailId}-${tab}`} tabIndex={0} className="space-y-5 px-4 py-4">
        {tab === "checks" && <ChecksList checks={detail.checks.checks} rollup={summary.checks} />}

        {tab === "commits" && <CommitList commits={pullRequest.commits} prUrl={summary.url} />}

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
          {pullRequest.body ? <GithubMarkdown text={pullRequest.body} repositoryUrl={repositoryUrl} /> : <p className="text-[12px] italic text-muted-foreground">No description provided.</p>}
        </section>

        <section aria-label="Conversation comments">
          <h3 className="mb-2 text-[12px] font-medium text-muted-foreground">CONVERSATION</h3>
          {pullRequest.comments.length ? <CommentList comments={pullRequest.comments} repositoryUrl={repositoryUrl} /> : <p className="text-[12px] text-muted-foreground">No conversation comments.</p>}
          <div className="mt-2.5">
            <CommentComposer disabled={busy} onSubmit={body => setPending({
              statement: `post a comment on PR #${number} on ${repoLabel}`,
              requiresBody: true,
              body,
              build: value => ({ kind: "comment", target: "pullRequest", number, body: value }),
            })} />
          </div>
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
                    <div className="mt-0.5"><GithubMarkdown text={comment.body} repositoryUrl={repositoryUrl} /></div>
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
      body={pending.body ?? pendingBody}
      onBody={setPendingBody}
      readOnlyBody={pending.body !== undefined}
      busy={busy}
      onCancel={closeOverlays}
      onConfirm={() => void submit(pending.build(pending.body ?? pendingBody))}
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

function IssueDetail({ workspaceId, repository, repositoryUrl, number, detail, error, availableLabels, onBack, onActed }: {
  workspaceId: string;
  repository?: GithubRepository | null;
  repositoryUrl?: string;
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
  const open = issue.summary.state === "open";

  return <div className="relative flex min-h-0 flex-1 flex-col">
    {header}
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="animate-page-mount border-b border-border px-4 py-3.5">
        <div className="flex items-center gap-2 text-[11px] text-muted-foreground">
          <CircleDot size={13} className={open ? "text-success" : "text-muted-foreground"} aria-hidden="true" />
          <span>#{issue.summary.number}</span>
          <span className="capitalize">{issue.summary.state}</span>
          <span className="ml-auto flex shrink-0 items-center">
            <CopyButton value={issue.summary.url} label={`Copy the link to #${issue.summary.number}`} />
            <OpenOnGithub url={issue.summary.url} what={`#${issue.summary.number}`} />
          </span>
        </div>
        <h2 className="mt-1.5 font-display text-[15px] font-semibold leading-snug text-foreground">{issue.summary.title}</h2>
        <p className="mt-1 text-[11px] text-muted-foreground">Opened by {issue.summary.author?.login ?? "ghost"}</p>
        <div className="mt-2 flex flex-wrap items-center gap-1.5">{issue.summary.labels.map(label => <LabelBadge key={label.name} label={label} />)}<button type="button" onClick={() => setLabelsOpen(value => !value)} className="inline-flex min-h-7 items-center gap-1 rounded-md px-2 py-1 text-[11px] text-muted-foreground hover:bg-accent hover:text-foreground"><Tag size={10.5} aria-hidden="true" /> Manage labels</button></div>
        {labelsOpen && <LabelControls available={availableLabels} current={issue.summary.labels} disabled={busy} onChange={(label, operation) => setPending({
          statement: `${operation === "add" ? "add" : "remove"} label ${JSON.stringify(label)} ${operation === "add" ? "to" : "from"} issue #${number} on ${repoLabel}`,
          requiresBody: false,
          build: () => ({ kind: "label", target: "issue", number, label, operation }),
        })} />}
        <div className="mt-3 flex flex-wrap items-center gap-1.5">
          <button type="button" disabled={busy} className={ACTION_BUTTON} onClick={() => setPending({
            statement: `${open ? "close" : "reopen"} issue #${number} on ${repoLabel}`,
            requiresBody: false,
            build: () => ({ kind: "setState", target: "issue", number, operation: open ? "close" : "reopen" }),
          })}>
            {open ? <><Ban size={11.5} aria-hidden="true" /> Close</> : <><RotateCcw size={11.5} aria-hidden="true" /> Reopen</>}
          </button>
        </div>
      </div>
      {actionError && <p role="alert" className="border-b border-border bg-destructive/10 px-4 py-2 text-[12px] text-destructive">{actionError}</p>}
      <div className="space-y-5 px-4 py-4">
        <section aria-label="Issue description"><h3 className="mb-2 text-[12px] font-medium text-muted-foreground">DESCRIPTION</h3>{issue.body ? <GithubMarkdown text={issue.body} repositoryUrl={repositoryUrl} /> : <p className="text-[12px] italic text-muted-foreground">No description provided.</p>}</section>
        <section aria-label="Issue comments">
          <h3 className="mb-2 text-[12px] font-medium text-muted-foreground">COMMENTS</h3>
          {issue.comments.length ? <CommentList comments={issue.comments} repositoryUrl={repositoryUrl} /> : <p className="text-[12px] text-muted-foreground">No comments.</p>}
          <div className="mt-2.5">
            <CommentComposer disabled={busy} onSubmit={body => setPending({
              statement: `post a comment on issue #${number} on ${repoLabel}`,
              requiresBody: true,
              body,
              build: value => ({ kind: "comment", target: "issue", number, body: value }),
            })} />
          </div>
        </section>
      </div>
    </div>
    {pending && <ConfirmOverlay
      statement={pending.statement}
      requiresBody={pending.requiresBody}
      body={pending.body ?? ""}
      onBody={() => undefined}
      readOnlyBody={pending.body !== undefined}
      busy={busy}
      onCancel={() => setPending(undefined)}
      onConfirm={() => void submit(pending.build(pending.body ?? ""))}
    />}
  </div>;
}

type ConfirmOverlayProps = {
  statement: string;
  note?: string;
  requiresBody: boolean;
  body: string;
  onBody: (value: string) => void;
  /** The body was already composed elsewhere — show it, don't ask for it again. */
  readOnlyBody?: boolean;
  busy: boolean;
  confirmLabel?: string;
  onCancel: () => void;
  onConfirm: () => void;
};

function ConfirmOverlay({ statement, note, requiresBody, body, onBody, readOnlyBody, busy, confirmLabel = "Confirm", onCancel, onConfirm }: ConfirmOverlayProps) {
  const ready = !requiresBody || body.trim().length > 0;
  return <Dialog open onOpenChange={next => { if (!next && !busy) onCancel(); }}>
    <DialogPopup showCloseButton={false} aria-label="Confirm action" className="max-w-md p-5">
      <p className="text-xs text-muted-foreground">This runs</p>
      <p className="mt-1 font-mono text-xs text-foreground/90">{statement}</p>
      {note && <p className="mt-2 text-[11px] leading-relaxed text-muted-foreground">{note}</p>}
      {requiresBody && (readOnlyBody
        ? <p className="mt-4 max-h-40 overflow-y-auto whitespace-pre-wrap rounded-lg border border-border bg-card p-3 text-[12.5px] leading-relaxed text-foreground" aria-label="Comment body">{body}</p>
        : <textarea value={body} onChange={event => onBody(event.target.value)} aria-label="Comment body" placeholder="Write a comment…" className="mt-4 min-h-28 w-full resize-y rounded-lg border border-input bg-card p-3 text-[13px] leading-relaxed" />)}
      <div className="mt-4 flex justify-end gap-2">
        <button type="button" onClick={onCancel} disabled={busy} className="min-h-8 rounded-lg px-3 text-[13px] font-medium hover:bg-accent disabled:opacity-50">Cancel</button>
        <button type="button" onClick={onConfirm} disabled={busy || !ready} className="min-h-8 rounded-lg bg-primary px-3 text-[13px] font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50">{confirmLabel}</button>
      </div>
    </DialogPopup>
  </Dialog>;
}
