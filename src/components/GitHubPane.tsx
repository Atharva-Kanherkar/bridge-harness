import { Dialog, DialogPopup } from "@/components/ui/dialog";
import { useCallback, useEffect, useId, useMemo, useReducer, useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion, type Variants } from "framer-motion";
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
  Eye,
  FileDiff,
  FolderGit2,
  GitBranch,
  GitCommitHorizontal,
  GitMerge,
  GitPullRequest,
  GitPullRequestDraft,
  Info,
  ListTodo,
  MessageSquare,
  RefreshCw,
  RotateCcw,
  Search,
  Sparkles,
  SquareArrowOutUpRight,
  Tag,
  ThumbsUp,
  TriangleAlert,
  X,
  MessageSquarePlus,
  MessageSquareDot,
  Plus,
} from "lucide-react";
import { bridgeApi } from "../api";
import { errorMessage } from "../errors";
import { rememberSurface, surfaceSnapshot } from "../githubSurfaceCache";
import { cn } from "@/lib/utils";
import { ActionSheet, ActionSheetHost } from "./github/ActionSheet";
import { MergeBox, initialMergeState, mergeReducer, type MergeState, type OverflowItem } from "./github/MergeBox";
import { Badge } from "@/components/ui/badge";
import { Skeleton, SkeletonDot, SkeletonRegion, SkeletonRows, SkeletonText, pendingClass } from "@/components/ui/skeleton";
import { Avatar, AvatarScope, Monogram } from "./github/Avatar";
import { Composer } from "./github/Composer";
import { GithubMarkdown } from "./github/GithubMarkdown";
import { GithubMediaScope } from "./github/Media";
import { activeJobFor, ReviewJobsView, ReviewStatusLine, runningCount, usePullRequestReviews } from "./github/ReviewJobs";
import { ReviewThreads } from "./github/ReviewThreads";
import { commentAnchor, parsePatch } from "./github/patch";
import { ProviderLoginPane } from "./ProviderLoginPane";
import { relativeTime } from "./workDashboard";
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
import type { GithubLinkView, PullRequestView } from "../githubLinks";
import type { LucideIcon } from "lucide-react";
import type {
  GithubAction,
  GithubCheckoutResult,
  GithubChecksResult,
  GithubIssueResult,
  GithubIssuesResult,
  GithubLabel,
  GithubMergeConfigResult,
  GithubPullRequestResult,
  GithubRepoCandidate,
  GithubRepository,
  GithubRepositoryResult,
  GithubStatusResult,
  MergeStrategy,
  ReviewDecision,
  ReviewJob,
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
  /** An outside ask (CI toast, a clicked GitHub link) to show one view. Nonce
   * distinguishes "open it again" from a re-render. */
  intent?: { view: GithubLinkView; nonce: number };
  onJumpToFile: (path: string, line: number | undefined, headBranch: string) => void;
  /** `dock` (default) is the 440px chat pane. `page` is Pulls's full-width
   *  screen: the repo slug never truncates, and at 1100px and up an open pull
   *  request or issue sits beside its list instead of replacing it. */
  layout?: "dock" | "page";
};

type Detail = { result: GithubPullRequestResult; checks: GithubChecksResult };
type SurfaceTab = "pulls" | "issues" | "repository";
type PullRequestTab = PullRequestView;

const ROLLUP: Record<RollupState, { icon: typeof CircleCheck; className: string; live?: boolean; label: string }> = {
  failing: { icon: CircleX, className: "text-destructive", label: "Failing" },
  running: { icon: CircleDashed, className: "text-warning", live: true, label: "Running" },
  passing: { icon: CircleCheck, className: "text-success", label: "Passing" },
  none: { icon: CircleDashed, className: "text-muted-foreground", label: "No checks" },
};

// Colour on this surface is a verdict, never decoration: green means good to
// go, amber means waiting or at risk, red means blocked. Everything else, an
// open PR, a pending review, a merged branch, stays on the chrome's neutral ink,
// so a glance at a list finds the rows that need a hand.
const REVIEW: Record<ReviewDecision, { label: string; variant: "success" | "warning" | "outline" } | null> = {
  approved: { label: "Approved", variant: "success" },
  changesRequested: { label: "Changes requested", variant: "warning" },
  reviewRequired: { label: "Review required", variant: "outline" },
  none: null,
};

/** A pull request's lifecycle glyph. Neutral ink: the state is the shape. */
function pullRequestStateIcon(pr: { isDraft: boolean; state: string }) {
  return pr.isDraft ? GitPullRequestDraft : pr.state === "merged" ? GitMerge : GitPullRequest;
}

const SECTION_LABEL = "text-caption font-medium text-muted-foreground";

/** A section heading with an optional count, the one heading shape the pane
 *  uses so Description, Conversation and Commits read as one family. */
function SectionLabel({ children, count, className }: { children: React.ReactNode; count?: number; className?: string }) {
  return <h3 className={cn("mb-2.5 flex items-center gap-2", SECTION_LABEL, className)}>
    {children}
    {count !== undefined && <span className="rounded-full bg-muted px-1.5 font-mono text-micro tabular-nums tracking-normal text-muted-foreground">{count}</span>}
  </h3>;
}

/** A branch name as a quiet mono chip. */
function BranchChip({ name, className }: { name: string; className?: string }) {
  return <span title={name} className={cn("inline-block max-w-full truncate rounded-sm bg-muted px-1.5 py-px font-mono text-caption text-foreground/80", className)}>{name}</span>;
}

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
      "inline-flex min-h-7 shrink-0 items-center gap-1.5 rounded-md px-1.5 text-caption text-muted-foreground transition-colors hover:bg-accent hover:text-foreground",
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
/** Pin this PR to the current chat as a live status card. The server verifies
 * the PR belongs to the chat's repository before anything persists. */
function ShowInChat({ sessionId, url, number }: { sessionId: string; url: string; number: number }) {
  const [state, setState] = useState<"idle" | "busy" | "done" | "error">("idle");
  const [message, setMessage] = useState<string>();
  useEffect(() => { setState("idle"); setMessage(undefined); }, [sessionId, number]);
  const pin = async () => {
    setState("busy");
    try {
      const result = await bridgeApi.githubAttachPr(sessionId, url);
      setState("done"); setMessage(result.message);
    } catch (value) {
      setState("error"); setMessage(errorMessage(value));
    }
  };
  const Icon = state === "done" ? MessageSquareDot : MessageSquarePlus;
  const label = state === "done" ? `#${number} is shown in this chat` : state === "error" ? `Could not show #${number} in this chat: ${message}` : `Show #${number} as a live card in this chat`;
  return <button
    type="button"
    onClick={() => void pin()}
    disabled={state === "busy" || state === "done"}
    aria-label={label}
    title={label}
    className={cn("inline-grid size-7 place-items-center rounded-md transition-colors hover:bg-accent disabled:cursor-default", state === "done" ? "text-success" : state === "error" ? "text-destructive" : "text-muted-foreground hover:text-foreground")}
  >
    <Icon size={13} aria-hidden="true" className={cn(state === "busy" && "github-check-live")} />
  </button>;
}

function OpenOnGithub({ url, what, className }: { url: string; what: string; className?: string }) {
  return <a
    href={url}
    target="_blank"
    rel="noreferrer"
    // Leaving is the whole point of this affordance, so the link router never
    // claims it. A check's log URL in particular is often shaped
    // `/pull/<n>/checks`, which the pane would otherwise swallow into a checks
    // list holding no logs.
    data-system-browser
    aria-label={`Open ${what} on GitHub`}
    title="Open on GitHub"
    className={cn(HEADER_ICON, className)}
  ><SquareArrowOutUpRight size={13} strokeWidth={1.7} aria-hidden="true" /></a>;
}

/** Centered empty/availability state — the pane never renders blank. */
function PaneNotice({ icon: Icon, title, children }: { icon: typeof GitPullRequest; title: string; children?: React.ReactNode }) {
  return <div className="animate-page-mount flex h-full flex-col items-center justify-center gap-2 px-8 text-center">
    <span className="grid size-12 place-items-center rounded-2xl border border-border bg-card text-muted-foreground shadow-xs"><Icon size={19} strokeWidth={1.6} aria-hidden="true" /></span>
    <p className="mt-2 font-display text-body font-semibold text-foreground">{title}</p>
    <div className="max-w-[26rem] text-meta leading-relaxed text-muted-foreground">{children}</div>
  </div>;
}

/** A comment's relative age; omitted entirely when the timestamp is junk —
 * a raw ISO string (or "unknown") is noise, not information. */
function CommentTime({ iso }: { iso: string }) {
  if (Number.isNaN(Date.parse(iso))) return null;
  return <time dateTime={iso} title={new Date(iso).toLocaleString()} className="text-caption text-muted-foreground">{relativeTime(iso, new Date())}</time>;
}

/** Placeholder rows shaped like the shared list. One quiet loading state:
 *  the rows are the signal, with no spinner or status copy stacked on top. */
function ListSkeleton({ label }: { label: string }) {
  return <SkeletonRegion label={label} className="min-h-0 flex-1 overflow-y-auto px-3 py-3 sm:px-4">
    <SkeletonRows rows={6} lead="dot" />
  </SkeletonRegion>;
}

/** Placeholder shaped like a PR/issue detail: meta line, title, branch line,
 * then a few body bars. */
function DetailSkeleton({ label }: { label: string }) {
  return <SkeletonRegion label={label} className="min-h-0 flex-1 overflow-y-auto">
    <div className="flex flex-col gap-2 border-b border-border px-4 pb-3 pt-3.5">
      <Skeleton className="w-28" />
      <Skeleton index={1} className="h-4 w-4/5" />
      <Skeleton index={2} className="w-2/5" />
    </div>
    <SkeletonText index={3} className="px-4 py-4" />
  </SkeletonRegion>;
}

/** The check rollup as a bar rather than a number pair. Segments are widths,
 * so a re-read animates from the old proportions to the new ones and a reader
 * sees CI move without reading digits. */
export function RollupMeter({ checks, className }: { checks: PullRequestListItem["checks"]; className?: string }) {
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
    className="inline-flex max-w-[12rem] items-center gap-1.5 rounded-full border border-border bg-card px-2 py-0.5 text-caption text-muted-foreground"
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
      return <button key={label.name} type="button" disabled={disabled} onClick={() => onChange(label.name, active ? "remove" : "add")} className="flex items-center gap-2 rounded-md px-2 py-1 text-left text-caption text-foreground transition-colors hover:bg-accent disabled:opacity-50">
        <Tag size={11} className={active ? "text-primary" : "text-muted-foreground"} aria-hidden="true" />
        <span className="min-w-0 flex-1 truncate">{label.name}</span>
        <span className="text-caption text-muted-foreground">{active ? "Remove" : "Add"}</span>
      </button>;
    }) : <p className="px-2 py-1 text-caption text-muted-foreground">No repository labels.</p>}
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
      className="min-w-0 flex-1 bg-transparent text-meta text-foreground outline-none placeholder:text-muted-foreground [&::-webkit-search-cancel-button]:hidden"
    />
    {value && <button type="button" onClick={() => onChange("")} aria-label="Clear the filter" className="grid size-4 shrink-0 place-items-center rounded text-muted-foreground transition-colors hover:text-foreground">
      <X size={10} aria-hidden="true" />
    </button>}
  </label>;
}

export function GitHubPane({ workspaceId, workspaceBranch, sessionId, intent, onJumpToFile, layout = "dock" }: GitHubPaneProps) {
  const page = layout === "page";
  const remembered = surfaceSnapshot(workspaceId);
  const [status, setStatus] = useState<GithubStatusResult | undefined>(remembered?.status);
  const [prs, setPrs] = useState<PullRequestListItem[] | undefined>(remembered?.prs);
  const [issues, setIssues] = useState<GithubIssuesResult["issues"] | undefined>(remembered?.issues);
  const [repositoryOverview, setRepositoryOverview] = useState<GithubRepositoryResult | undefined>(remembered?.repositoryOverview);
  const [surfaceTab, setSurfaceTab] = useState<SurfaceTab>("pulls");
  const [surfaceError, setSurfaceError] = useState<string>();
  const [tabErrors, setTabErrors] = useState<Partial<Record<SurfaceTab, string>>>({});
  const [refreshing, setRefreshing] = useState(false);
  const [query, setQuery] = useState("");
  const [facet, setFacet] = useState<PullRequestFacet>("all");
  // The dock's push direction: opening a detail pushes forward, back and the
  // surface switch step the other way or cross-fade (issue #1028).
  const reducedMotion = useReducedMotion();
  const [pushDirection, setPushDirection] = useState<"forward" | "back" | "none">("none");

  const [selected, setSelected] = useState<number>();
  const [detail, setDetail] = useState<Detail>();
  const [detailError, setDetailError] = useState<string>();
  const [connectOpen, setConnectOpen] = useState(false);
  const [selectedIssue, setSelectedIssue] = useState<number>();
  const [issueDetail, setIssueDetail] = useState<GithubIssueResult>();
  const [issueError, setIssueError] = useState<string>();

  const [signingIn, setSigningIn] = useState(false);

  const alive = useRef(true);
  const refreshingChecks = useRef(false);
  const surfaceLoad = useRef(0);
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);

  const loadSurface = useCallback(async (fresh = false) => {
    const generation = ++surfaceLoad.current;
    const current = () => alive.current && surfaceLoad.current === generation;
    setRefreshing(true);
    try {
      const next = await bridgeApi.githubStatus(workspaceId, fresh);
      if (!current()) return;
      setStatus(next);
      setSurfaceError(undefined);
      // Keep successful reads only while the repository identity is stable.
      // A reconnect or sign-out must clear the previous repository's content.
      const snapshot = rememberSurface(workspaceId, { status: next });
      setPrs(snapshot.prs);
      setIssues(snapshot.issues);
      setRepositoryOverview(snapshot.repositoryOverview);
      // With no repository resolved, every read below fails with the same
      // resolution error. Asking anyway only turns a setup step into three
      // error notices, so the pane offers Connect instead.
      if (next.availability.status === "available" && next.repository) {
        const settle = async <T,>(tab: SurfaceTab, read: Promise<T>, apply: (value: T) => void) => {
          try {
            const value = await read;
            if (!current()) return;
            apply(value);
            setTabErrors(current => {
              const nextErrors = { ...current };
              delete nextErrors[tab];
              return nextErrors;
            });
          } catch (error) {
            if (!current()) return;
            setTabErrors(current => ({
              ...current,
              [tab]: errorMessage(error),
            }));
          }
        };
        await Promise.all([
          settle("pulls", bridgeApi.githubPullRequests(workspaceId), value => {
            setPrs(value.pullRequests);
            rememberSurface(workspaceId, { status: next, prs: value.pullRequests });
          }),
          settle("issues", bridgeApi.githubIssues(workspaceId), value => {
            setIssues(value.issues);
            rememberSurface(workspaceId, { status: next, issues: value.issues });
          }),
          settle("repository", bridgeApi.githubRepository(workspaceId), value => {
            setRepositoryOverview(value);
            rememberSurface(workspaceId, { status: next, repositoryOverview: value });
          }),
        ]);
      } else {
        setPrs(undefined);
        setIssues(undefined);
        setRepositoryOverview(undefined);
        setTabErrors({});
      }
    } catch (error) {
      if (current()) setSurfaceError(errorMessage(error));
    } finally {
      if (current()) setRefreshing(false);
    }
  }, [workspaceId]);

  const openDetail = useCallback(async (number: number, retain = false) => {
    setPushDirection("forward");
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
      if (alive.current) setDetailError(errorMessage(error));
    }
  }, [workspaceId]);

  const openIssue = useCallback(async (number: number, retain = false) => {
    setPushDirection("forward");
    setSurfaceTab("issues");
    setSelectedIssue(number);
    if (!retain) setIssueDetail(undefined);
    setIssueError(undefined);
    try {
      const result = await bridgeApi.githubIssue(workspaceId, number);
      if (alive.current) setIssueDetail(result);
    } catch (error) {
      if (alive.current) setIssueError(errorMessage(error));
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
      if (alive.current) setDetailError(errorMessage(error));
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
    // Reset, and rehydrate: a workspace the surface has seen before paints
    // its remembered reads immediately and lets loadSurface revalidate them
    // in the background. An unseen workspace remembers nothing, so this is
    // the old clear-to-undefined behavior for it.
    const remembered = surfaceSnapshot(workspaceId);
    setStatus(remembered?.status); setPrs(remembered?.prs); setIssues(remembered?.issues); setRepositoryOverview(remembered?.repositoryOverview); setSurfaceError(undefined); setTabErrors({});
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

  // Deep links (CI toast, a GitHub link clicked anywhere in the app) land
  // here. A list or overview intent clears whatever detail was open, so the
  // pane shows the view that was asked for rather than the one it was left on.
  const seenIntent = useRef(0);
  const [pullRequestFocus, setPullRequestFocus] = useState<{ tab: PullRequestTab; nonce: number }>();
  useEffect(() => {
    if (!intent || intent.nonce === seenIntent.current) return;
    seenIntent.current = intent.nonce;
    const view = intent.view;
    const showSurface = (tab: SurfaceTab) => {
      setPushDirection("none");
      setSurfaceTab(tab);
      setSelected(undefined); setDetail(undefined); setDetailError(undefined);
      setSelectedIssue(undefined); setIssueDetail(undefined); setIssueError(undefined);
    };
    switch (view.kind) {
      case "pull":
        setPullRequestFocus({ tab: view.tab, nonce: intent.nonce });
        void openDetail(view.number);
        break;
      case "issue":
        void openIssue(view.number);
        break;
      case "pulls": showSurface("pulls"); break;
      case "issues": showSurface("issues"); break;
      case "repository": showSurface("repository"); break;
    }
  }, [intent, openDetail, openIssue]);

  const repoLabel = status?.repository ? `${status.repository.owner}/${status.repository.name}` : undefined;
  const mediaHost = status?.repository?.host ?? "github.com";
  const mediaScope = useMemo(() => ({ workspaceId, host: mediaHost }), [workspaceId, mediaHost]);
  const repositoryUrl = repositoryOverview?.url
    ?? (status?.repository ? `https://${status.repository.host}/${status.repository.owner}/${status.repository.name}` : undefined);
  const visiblePrs = useMemo(() => prs && filterPullRequests(prs, query, facet), [prs, query, facet]);
  const visibleIssues = useMemo(() => issues && filterIssues(issues, query), [issues, query]);
  const inDetail = (surfaceTab === "pulls" && selected !== undefined) || (surfaceTab === "issues" && selectedIssue !== undefined);
  const showsFilters = !inDetail && surfaceTab !== "repository" && status?.availability.status === "available" && !!status.repository;

  // Wide rows only when the list has the page to itself; beside an open item it
  // is a narrow rail and keeps the stacked dock shape.
  const wide = page && !inDetail;
  const pullList = visiblePrs && <div className={cn("min-h-0 flex-1 overflow-y-auto", wide ? "px-5 py-4" : "px-3 py-3 sm:px-4")}>
    <div className="divide-y divide-border overflow-hidden rounded-xl border border-border bg-card">
      {wide && <div aria-hidden="true" className={cn("grid items-center gap-4 bg-muted/40 px-4 py-2", PR_GRID, SECTION_LABEL)}>
        <span className="pl-7">Pull request</span><span>Checks</span><span>Review</span><span>Author</span>
      </div>}
      {visiblePrs.map((pr, index) => <PullRequestRow key={pr.number} pr={pr} index={index} wide={wide} active={pr.number === selected} onOpen={() => void openDetail(pr.number)} />)}
    </div>
  </div>;
  const issueList = visibleIssues && <IssueList issues={visibleIssues} wide={wide} active={selectedIssue} onOpen={number => void openIssue(number)} />;
  // Beside an open item on a wide page: the list it was opened from.
  const sideList = page && inDetail ? (surfaceTab === "pulls" ? pullList : issueList) : undefined;

  let body: React.ReactNode;
  if (surfaceError) {
    body = <PaneNotice icon={CircleX} title="GitHub is unreachable">{surfaceError}</PaneNotice>;
  } else if (!status) {
    body = <ListSkeleton label="Connecting to GitHub" />;
  } else if (status.availability.status === "notInstalled") {
    body = <PaneNotice icon={CircleSlash} title="GitHub CLI is not installed">Bridge drives GitHub through <code className="font-mono text-foreground/90">gh</code> — install it and sign in, and this pane fills in by itself.</PaneNotice>;
  } else if (status.availability.status === "notAuthenticated") {
    body = <PaneNotice icon={CircleDot} title="Sign in to GitHub">
      Bridge reads pull requests and issues through the GitHub CLI, which is not signed in yet.
      {signingIn
        ? <div className="text-left"><ProviderLoginPane provider="github" label="GitHub" onClose={() => { setSigningIn(false); void loadSurface(true); }} /></div>
        : <span className="mt-3 block">
          <button type="button" onClick={() => setSigningIn(true)} className="min-h-8 rounded-lg bg-primary px-3 text-ui font-medium text-primary-foreground transition-colors hover:bg-primary/90">Sign in to GitHub</button>
        </span>}
    </PaneNotice>;
  } else if (!status.repository) {
    // Not an error: the folder either has no git repository yet or no GitHub
    // remote. Both are one action away from working, so offer the action.
    body = <PaneNotice icon={FolderGit2} title="No GitHub repository connected">
      This folder has no GitHub remote, so there are no pull requests or issues to show. Connect it to a repository and this pane fills in.
      <span className="mt-3 block">
        <button type="button" onClick={() => setConnectOpen(true)} className="min-h-8 rounded-lg bg-primary px-3 text-ui font-medium text-primary-foreground transition-colors hover:bg-primary/90">Connect a repository</button>
      </span>
    </PaneNotice>;
  } else if (surfaceTab === "pulls" && selected !== undefined) {
    body = <PullRequestDetail
      // A new PR is a new detail: no merge state or in-flight result carries over.
      key={selected}
      workspaceId={workspaceId}
      workspaceBranch={workspaceBranch}
      sessionId={sessionId}
      repository={status.repository}
      repositoryUrl={repositoryUrl}
      number={selected}
      detail={detail}
      error={detailError}
      focus={pullRequestFocus}
      availableLabels={repositoryOverview?.labels ?? []}
      onBack={() => { setPushDirection("back"); setSelected(undefined); setDetail(undefined); setDetailError(undefined); }}
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
    body = pullList;
  } else if (surfaceTab === "issues" && selectedIssue !== undefined) {
    body = <IssueDetail
      workspaceId={workspaceId}
      repository={status.repository}
      repositoryUrl={repositoryUrl}
      number={selectedIssue}
      detail={issueDetail}
      error={issueError}
      availableLabels={repositoryOverview?.labels ?? []}
      onBack={() => { setPushDirection("back"); setSelectedIssue(undefined); setIssueDetail(undefined); setIssueError(undefined); }}
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
    body = issueList;
  } else if (tabErrors.repository && !repositoryOverview) {
    body = <PaneNotice icon={CircleX} title="Repository did not load">{tabErrors.repository}</PaneNotice>;
  } else if (!repositoryOverview) {
    body = <DetailSkeleton label="Loading repository" />;
  } else {
    body = <RepositoryOverview overview={repositoryOverview} />;
  }

  // What the pane's single column shows right now — the push key.
  const viewKey = surfaceTab === "pulls"
    ? (selected !== undefined ? `pull-${selected}` : "pulls")
    : surfaceTab === "issues"
      ? (selectedIssue !== undefined ? `issue-${selectedIssue}` : "issues")
      : "repository";

  const surfaces = [
    ["pulls", "Pull requests", GitPullRequest, prs?.length],
    ["issues", "Issues", ListTodo, issues?.length],
    ["repository", "Repository", Info, undefined],
  ] as const;
  const pickSurface = (id: SurfaceTab) => { setPushDirection("back"); setSurfaceTab(id); setSelected(undefined); setSelectedIssue(undefined); setQuery(""); };
  const refresh = () => { void loadSurface(true); if (selected !== undefined) void openDetail(selected, true); if (selectedIssue !== undefined) void openIssue(selectedIssue, true); };
  const refreshButton = <button type="button" onClick={refresh} aria-label="Refresh GitHub" title="Refresh" aria-busy={refreshing || undefined} className={cn(HEADER_ICON, pendingClass(refreshing))}>
    <RefreshCw size={13} strokeWidth={1.7} aria-hidden="true" />
  </button>;

  return <GithubMediaScope.Provider value={mediaScope}><section data-gh-pane-root="" className="relative flex h-full w-full flex-col" aria-label="GitHub repository">
    {page ? <header className="flex shrink-0 flex-col border-b border-border bg-background px-5">
      <div className="flex h-11 items-end gap-3">
        {/* Labelled tabs with live counts: at full width there is room to say
            what each surface is instead of making the reader decode icons. */}
        <div className="flex min-w-0 items-end gap-1" role="toolbar" aria-label="GitHub repository actions">
          {surfaces.map(([id, label, Icon, count]) => <button
            key={id}
            type="button"
            aria-label={label}
            aria-pressed={surfaceTab === id}
            data-active={surfaceTab === id}
            onClick={() => pickSurface(id)}
            className={cn(
              "relative -mb-px flex h-10 items-center gap-1.5 border-b-2 px-2.5 text-ui transition-colors",
              surfaceTab === id ? "border-foreground font-medium text-foreground" : "border-transparent text-muted-foreground hover:text-foreground",
            )}
          >
            <Icon size={14} strokeWidth={1.7} aria-hidden="true" />
            {label}
            {count !== undefined && <span className={cn("rounded-full px-1.5 font-mono text-micro tabular-nums", surfaceTab === id ? "bg-foreground text-background" : "bg-muted text-muted-foreground")}>{count}</span>}
          </button>)}
        </div>
        <div className="ml-auto flex shrink-0 items-center gap-0.5 self-center">
          {repoLabel && <CopyButton value={repoLabel} label={`Copy ${repoLabel}`} className="h-7 px-2">
            <span className="whitespace-nowrap font-mono text-caption">{repoLabel}</span>
          </CopyButton>}
          {repositoryUrl && <OpenOnGithub url={repositoryUrl} what={repoLabel ?? "the repository"} />}
          {refreshButton}
        </div>
      </div>
      {showsFilters && <div className="flex items-center gap-2 py-2.5">
        <ListSearch value={query} onChange={setQuery} placeholder={surfaceTab === "pulls" ? "Filter pull requests" : "Filter issues"} />
        {surfaceTab === "pulls" && <div className="u-segmented flex shrink-0 p-0.5" role="group" aria-label="Pull request filters">
          {PULL_REQUEST_FACETS.map(choice => <button
            key={choice.id}
            type="button"
            aria-pressed={facet === choice.id}
            data-active={facet === choice.id}
            onClick={() => setFacet(choice.id)}
            className="u-segmented-item rounded-md px-2.5 py-1 text-meta text-muted-foreground"
          >{choice.label}</button>)}
        </div>}
      </div>}
    </header> : <header className="flex shrink-0 flex-col gap-1.5 border-b border-border bg-background p-1.5">
      <div className="flex h-8 items-center gap-1.5">
        <div className="flex min-w-0 flex-1 items-center gap-1.5">
          <span className="inline-flex size-7 shrink-0 items-center justify-center text-muted-foreground">
            <FolderGit2 size={13} strokeWidth={1.7} aria-hidden="true" />
          </span>
          {repoLabel && <CopyButton value={repoLabel} label={`Copy ${repoLabel}`} className="h-7 min-w-0 shrink px-1.5">
            <span className="min-w-0 truncate font-mono text-caption">{repoLabel}</span>
          </CopyButton>}
        </div>
        <div className="u-segmented flex shrink-0 items-center p-0.5" role="toolbar" aria-label="GitHub repository actions">
          {surfaces.map(([id, label, Icon]) => <button
            key={id}
            type="button"
            aria-label={label}
            aria-pressed={surfaceTab === id}
            data-active={surfaceTab === id}
            title={label}
            onClick={() => pickSurface(id)}
            className={cn("u-segmented-item", HEADER_ICON)}
          ><Icon size={13} strokeWidth={1.7} aria-hidden="true" /></button>)}
          {repositoryUrl && <OpenOnGithub url={repositoryUrl} what={repoLabel ?? "the repository"} />}
          {refreshButton}
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
            className="u-segmented-item rounded-md px-2 py-1 text-caption text-muted-foreground"
          >{choice.label}</button>)}
        </div>}
      </div>}
    </header>}
    {connectOpen && <ConnectRepositoryDialog
      workspaceId={workspaceId}
      onCancel={() => setConnectOpen(false)}
      onConnected={() => { setConnectOpen(false); void loadSurface(true); }}
    />}
    {tabErrors[surfaceTab] && ((surfaceTab === "pulls" && prs) || (surfaceTab === "issues" && issues) || (surfaceTab === "repository" && repositoryOverview)) && <p role="alert" className="border-b border-border bg-warning/10 px-4 py-2 text-caption text-warning">Refresh failed: {tabErrors[surfaceTab]}</p>}
    {sideList
      ? <div className="flex min-h-0 flex-1">
          <div data-pulls-list className="hidden min-h-0 w-[min(420px,38%)] shrink-0 flex-col border-r border-border min-[1100px]:flex">{sideList}</div>
          <div className="flex min-h-0 min-w-0 flex-1 flex-col">{body}</div>
        </div>
      : <div className="relative flex min-h-0 flex-1 flex-col">
          <AnimatePresence initial={false} custom={pushDirection} mode="popLayout">
            <motion.div
              key={viewKey}
              custom={pushDirection}
              variants={reducedMotion ? PUSH_REDUCED_VARIANTS : PUSH_VARIANTS}
              initial="enter"
              animate="center"
              exit="exit"
              className="flex min-h-0 w-full flex-1 flex-col"
            >{body}</motion.div>
          </AnimatePresence>
        </div>}
  </section></GithubMediaScope.Provider>;
}

const PR_GRID = "grid-cols-[minmax(0,1fr)_9.5rem_8.5rem_9rem]";

/** The checks column: a glyph, a count and the proportions bar, all in the
 *  rollup's verdict colour so a red row is findable from across the room. */
function ChecksCell({ pr }: { pr: PullRequestListItem }) {
  if (!pr.checks.total) return <span className="text-caption text-muted-foreground">No checks</span>;
  const rollup = ROLLUP[rollupState(pr)];
  const RollupIcon = rollup.icon;
  return <span className="flex min-w-0 flex-col gap-1.5" title={`${rollup.label} — ${pr.checks.passed}/${pr.checks.total} checks passed`}>
    <span className={cn("inline-flex items-center gap-1.5 text-caption tabular-nums", rollup.className)}>
      <RollupIcon size={13} aria-hidden="true" className={cn(rollup.live && "github-check-live")} />
      <span className="font-medium"><RollLabel value={rollup.label} /></span>
      <span className="font-mono text-caption text-muted-foreground">{pr.checks.passed}/{pr.checks.total}</span>
    </span>
    <RollupMeter checks={pr.checks} className="w-full max-w-28" />
  </span>;
}

function PullRequestRow({ pr, index, wide, active, onOpen }: { pr: PullRequestListItem; index: number; wide: boolean; active: boolean; onOpen: () => void }) {
  const rollup = ROLLUP[rollupState(pr)];
  const RollupIcon = rollup.icon;
  const review = REVIEW[pr.reviewDecision];
  const StateIcon = pullRequestStateIcon(pr);
  const stateTone = pr.isDraft || pr.state === "merged" ? "text-muted-foreground" : "text-foreground";
  const actions = <span className="u-glass-soft absolute bottom-2.5 right-1.5 flex items-center rounded-md opacity-0 transition-opacity focus-within:opacity-100 group-hover:opacity-100">
    <CopyButton value={pr.url} label={`Copy the link to #${pr.number}`} />
    <OpenOnGithub url={pr.url} what={`#${pr.number}`} />
  </span>;
  const rowClass = cn("github-row-enter group relative transition-colors hover:bg-accent/50", active && "bg-accent/60");
  const delay = { "--github-row-delay": `${Math.min(index, 12) * 28}ms` } as React.CSSProperties;

  if (wide) return <div className={rowClass} style={delay}>
    <button type="button" onClick={onOpen} className={cn("grid w-full items-center gap-4 px-4 py-3.5 text-left", PR_GRID)}>
      <span className="flex min-w-0 items-start gap-3">
        <StateIcon size={16} strokeWidth={1.8} aria-hidden="true" className={cn("mt-0.5 shrink-0", stateTone)} />
        <span className="min-w-0 flex-1">
          <span className="flex flex-wrap items-center gap-x-2 gap-y-1">
            <span className="min-w-0 text-ui font-medium leading-5 text-foreground group-hover:underline group-hover:decoration-border group-hover:underline-offset-4">{pr.title}</span>
            {pr.isDraft && <Badge variant="outline" size="sm">Draft</Badge>}
            {pr.mergeability === "conflicting" && <Badge variant="warning" size="sm"><TriangleAlert aria-hidden="true" />Conflicts</Badge>}
          </span>
          <span className="mt-1.5 flex min-w-0 items-center gap-2 text-caption text-muted-foreground">
            <span className="shrink-0 font-mono tabular-nums">#{pr.number}</span>
            <BranchChip name={pr.headBranch} className="min-w-0" />
          </span>
        </span>
      </span>
      <ChecksCell pr={pr} />
      <span className="min-w-0">{review ? <Badge variant={review.variant} size="sm">{review.label}</Badge> : <span className="text-caption text-muted-foreground">—</span>}</span>
      <span className="flex min-w-0 items-center gap-2 pr-12 text-meta text-muted-foreground">
        <Monogram login={pr.author?.login} />
        <span className="truncate">{pr.author?.login ?? "ghost"}</span>
      </span>
    </button>
    {actions}
  </div>;

  return <div className={cn(rowClass, "flex items-start")} style={delay}>
    <button type="button" onClick={onOpen} className="flex min-w-0 flex-1 items-start gap-2.5 px-3 py-3 text-left">
      <StateIcon size={15} strokeWidth={1.8} aria-hidden="true" className={cn("mt-0.5 shrink-0", stateTone)} />
      <span className="min-w-0 flex-1">
        <span className="flex flex-wrap items-start gap-x-1.5 gap-y-1">
          <span className="min-w-0 flex-1 text-ui font-medium leading-5 text-foreground">{pr.title}</span>
          {pr.mergeability === "conflicting" && <Badge variant="warning" size="sm">Conflicts</Badge>}
          {pr.isDraft && <Badge variant="outline" size="sm">Draft</Badge>}
          {review && <Badge variant={review.variant} size="sm">{review.label}</Badge>}
          {pr.checks.total > 0 && <span className={cn("inline-flex shrink-0 items-center gap-1 font-mono text-caption tabular-nums", rollup.className)} title={`${rollup.label} — ${pr.checks.passed}/${pr.checks.total} checks passed`}>
            <RollupIcon size={12} aria-hidden="true" className={cn(rollup.live && "github-check-live")} />
            {pr.checks.passed}/{pr.checks.total}
          </span>}
        </span>
        <span className="mt-1 flex items-center gap-1.5 pr-14 text-caption text-muted-foreground">
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
    {actions}
  </div>;
}

/** Every rendered patch line is a DOM node; unbounded patches froze the
 * webview on large PRs. Files above the eager threshold start collapsed and
 * single patches render at most this many lines. */
export const EAGER_PATCH_FILE_LIMIT = 6;
export const PATCH_LINE_LIMIT = 600;

/** Where a new review comment goes, and the write that posts it. */
type LineComment = (line: number, side: "left" | "right", body: string) => Promise<string | undefined>;

const PATCH_ROW: Record<string, string> = {
  add: "bg-success/10 text-success",
  del: "bg-destructive/10 text-destructive",
  hunk: "bg-muted text-muted-foreground",
};

function PatchView({ patch, fullDiffUrl, onComment }: { patch: string; fullDiffUrl: string; onComment?: LineComment }) {
  const lines = useMemo(() => parsePatch(patch), [patch]);
  const shown = lines.length > PATCH_LINE_LIMIT ? lines.slice(0, PATCH_LINE_LIMIT) : lines;
  // One open composer per file: starting a second comment moves it.
  const [composing, setComposing] = useState<number>();
  return <>
    <div role="group" className="max-h-[28rem] overflow-auto bg-background/50 py-2 font-mono text-caption leading-5" aria-label="File patch">{shown.map((line, index) => {
      const anchor = onComment ? commentAnchor(line) : null;
      return <div key={index}>
        <div className={cn("group/line relative flex w-max min-w-full whitespace-pre", PATCH_ROW[line.kind])}>
          <span aria-hidden="true" className="w-9 shrink-0 select-none pr-1.5 text-right tabular-nums text-muted-foreground/60">{line.oldLine ?? ""}</span>
          <span aria-hidden="true" className="w-9 shrink-0 select-none pr-2 text-right tabular-nums text-muted-foreground/60">{line.newLine ?? ""}</span>
          {anchor && <button
            type="button"
            onClick={() => setComposing(index)}
            aria-label={`Comment on line ${anchor.line}`}
            title={`Comment on line ${anchor.line}`}
            className="absolute left-[4.1rem] top-0.5 grid size-4 -translate-x-1/2 scale-75 place-items-center rounded-sm bg-primary text-primary-foreground opacity-0 shadow-xs transition-[opacity,transform] duration-150 hover:scale-110 focus-visible:scale-100 focus-visible:opacity-100 group-hover/line:scale-100 group-hover/line:opacity-100"
          ><Plus size={11} strokeWidth={2.4} aria-hidden="true" /></button>}
          <span className="pl-2 pr-3">{line.text || " "}</span>
        </div>
        {anchor && onComment && composing === index && <div className="sticky left-0 w-full max-w-full border-y border-border bg-card/80 p-2 font-sans">
          <Composer
            autoFocus
            label={`Comment on line ${anchor.line}`}
            placeholder={`Comment on line ${anchor.line}…`}
            submitLabel="Add comment"
            onSubmit={body => onComment(anchor.line, anchor.side, body)}
            onCancel={() => setComposing(undefined)}
          />
        </div>}
      </div>;
    })}</div>
    {shown.length < lines.length && <a href={fullDiffUrl} target="_blank" rel="noreferrer" data-system-browser className="flex items-center gap-1.5 border-t border-border px-3 py-2 text-caption text-muted-foreground transition-colors hover:text-foreground">
      <ExternalLink size={11} aria-hidden="true" />
      Patch truncated at {PATCH_LINE_LIMIT} lines — view the full diff on GitHub
    </a>}
  </>;
}

function FileChange({ file, defaultOpen, fullDiffUrl, onComment }: {
  file: GithubPullRequestResult["files"][number];
  defaultOpen: boolean;
  fullDiffUrl: string;
  onComment?: LineComment;
}) {
  const [open, setOpen] = useState(defaultOpen);
  return <article className="overflow-hidden rounded-lg border border-border bg-card">
    <div className={cn("flex items-center transition-colors hover:bg-accent/40", open && "border-b border-border")}>
      <button type="button" onClick={() => setOpen(value => !value)} aria-expanded={open} className="flex min-w-0 flex-1 items-center gap-2 px-3 py-2 text-left">
        <ChevronRight size={11} aria-hidden="true" className={cn("shrink-0 text-muted-foreground transition-transform duration-200", open && "rotate-90")} />
        <span className="min-w-0 flex-1 truncate font-mono text-caption text-foreground">{file.path}</span>
        <Badge variant="outline" size="sm">{file.status}</Badge>
        <span className="font-mono text-caption text-success">+{file.additions}</span>
        <span className="font-mono text-caption text-destructive">−{file.deletions}</span>
      </button>
      <CopyButton value={file.path} label={`Copy the path ${file.path}`} className="mr-1.5" />
    </div>
    {open && (file.patch ? <PatchView patch={file.patch} fullDiffUrl={fullDiffUrl} onComment={onComment} /> : <p className="px-3 py-4 text-center text-meta text-muted-foreground">Binary file or patch unavailable from GitHub.</p>)}
  </article>;
}

function CommentList({ comments, repositoryUrl }: {
  comments: Array<{ id: string; author?: { login: string } | null; body: string; createdAt: string; url: string }>;
  repositoryUrl?: string;
}) {
  const reduced = useReducedMotion();
  // The first paint staggers in; a comment that arrives later (the one just
  // posted) rises on its own without replaying the whole list.
  const mounted = useRef(false);
  useEffect(() => { mounted.current = true; }, []);
  return <div className="space-y-2">
    <AnimatePresence initial>
      {comments.map((comment, index) => <motion.article
        key={comment.id}
        layout={reduced ? false : "position"}
        initial={reduced ? false : { opacity: 0, y: 10 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.28, ease: [0.22, 1, 0.36, 1], delay: reduced || mounted.current ? 0 : Math.min(index, 8) * 0.04 }}
        className="group overflow-hidden rounded-lg border border-border bg-card"
      >
        <div className="flex items-center gap-2 border-b border-border/60 px-3 py-1.5">
          <Avatar login={comment.author?.login} className="size-[18px] text-micro" />
          <span className="text-caption font-medium text-foreground">{comment.author?.login ?? "ghost"}</span>
          <CommentTime iso={comment.createdAt} />
          <span className="ml-auto flex shrink-0 items-center opacity-0 transition-opacity focus-within:opacity-100 group-hover:opacity-100">
            <CopyButton value={comment.url} label="Copy the link to this comment" />
            <OpenOnGithub url={comment.url} what="this comment" />
          </span>
        </div>
        <div className="px-3 py-2"><GithubMarkdown text={comment.body} repositoryUrl={repositoryUrl} /></div>
      </motion.article>)}
    </AnimatePresence>
  </div>;
}

/** Checks, grouped by the workflow that produced them. Each group states its
 * own tally so a red workflow is findable without reading every row. */
function ChecksList({ checks, rollup }: { checks: GithubChecksResult["checks"]; rollup?: PullRequestListItem["checks"] }) {
  const groups = useMemo(() => groupChecksByWorkflow(checks), [checks]);
  const settled = useCheckSettle(checks);
  return <section aria-label="Checks" className="space-y-3">
    {rollup && rollup.total > 0 && <div className="rounded-lg border border-border bg-card px-3 py-2.5">
      <p className="flex items-center gap-2 text-caption text-muted-foreground">
        <span className="font-medium text-foreground">{rollup.passed}/{rollup.total} passed</span>
        {rollup.failed > 0 && <span className="text-destructive">{rollup.failed} failed</span>}
        {rollup.queued + rollup.inProgress > 0 && <span className="text-warning">{rollup.queued + rollup.inProgress} running</span>}
        {rollup.skipped + rollup.cancelled > 0 && <span>{rollup.skipped + rollup.cancelled} skipped</span>}
      </p>
      <RollupMeter checks={rollup} className="mt-2" />
    </div>}
    {groups.length ? groups.map(group => <div key={group.workflow} className="overflow-hidden rounded-lg border border-border bg-card">
      <p className={cn("border-b border-border/60 bg-muted/40 px-3 py-2", SECTION_LABEL)}>{group.workflow}</p>
      <div className="divide-y divide-border/60">
        {group.checks.map(check => {
          const tone = checkTone(check.conclusion, check.status);
          const ToneIcon = tone.icon;
          const key = `${check.workflow}·${check.name}`;
          const justSettled = settled.has(key);
          return <div key={key} className={cn("flex items-center gap-2.5 px-3 py-2", justSettled && "github-check-settle")}>
            <ToneIcon size={13.5} className={cn("shrink-0", tone.className, tone.live && "github-check-live", justSettled && "check-glyph-settle")} aria-hidden="true" />
            <span className="min-w-0 flex-1 truncate text-meta text-foreground">{check.name}</span>
            <span className="shrink-0 text-caption text-muted-foreground">{check.conclusion ?? check.status}</span>
            {check.logUrl && <OpenOnGithub url={check.logUrl} what={`logs for ${check.name}`} />}
          </div>;
        })}
      </div>
    </div>) : <p className="text-meta text-muted-foreground">No checks reported.</p>}
  </section>;
}

/** The commit list. The pane could show a PR's files and its checks but never
 * its commits, which is the one view that answers "what actually landed on
 * this branch". Commits ride the `pr view` read the detail already pays for. */
function CommitList({ commits, prUrl }: { commits: GithubPullRequestResult["pullRequest"]["commits"]; prUrl: string }) {
  return <section aria-label="Commits">
    <SectionLabel count={commits.length}>Commits</SectionLabel>
    {commits.length ? <div className="divide-y divide-border overflow-hidden rounded-lg border border-border bg-card">
      {commits.map((commit, index) => <article
        key={commit.oid}
        className="github-row-enter group relative flex items-start gap-2.5 px-3 py-2.5 transition-colors hover:bg-accent/30"
        style={{ "--github-row-delay": `${Math.min(index, 12) * 24}ms` } as React.CSSProperties}
      >
        <GitCommitHorizontal size={14} className="mt-0.5 shrink-0 text-muted-foreground" aria-hidden="true" />
        <div className="min-w-0 flex-1">
          <p className="pr-14 text-meta font-medium leading-5 text-foreground">{commit.messageHeadline || "(no commit message)"}</p>
          {commit.messageBody.trim() && <p className="mt-0.5 whitespace-pre-line text-caption leading-relaxed text-muted-foreground">{commit.messageBody.trim()}</p>}
          <p className="mt-1 flex flex-wrap items-center gap-1.5 text-caption text-muted-foreground">
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
    </div> : <p className="text-meta text-muted-foreground">No commits reported for this pull request.</p>}
  </section>;
}

function IssueList({ issues, wide, active, onOpen }: { issues: GithubIssuesResult["issues"]; wide: boolean; active?: number; onOpen: (number: number) => void }) {
  return <div className={cn("min-h-0 flex-1 overflow-y-auto", wide ? "px-5 py-4" : "px-3 py-3 sm:px-4")}>
    <div className="divide-y divide-border overflow-hidden rounded-xl border border-border bg-card">
      {issues.map((issue, index) => <div
        key={issue.number}
        className={cn("github-row-enter group relative flex items-start transition-colors hover:bg-accent/50", active === issue.number && "bg-accent/60")}
        style={{ "--github-row-delay": `${Math.min(index, 12) * 28}ms` } as React.CSSProperties}
      >
        <button type="button" onClick={() => onOpen(issue.number)} className={cn("flex min-w-0 flex-1 items-start text-left", wide ? "gap-3 px-4 py-3.5" : "gap-2.5 px-3 py-3")}>
          <CircleDot size={wide ? 16 : 14} strokeWidth={1.8} className={cn("mt-0.5 shrink-0", issue.state === "open" ? "text-foreground" : "text-muted-foreground")} aria-hidden="true" />
          <span className="min-w-0 flex-1">
            <span className={cn("block pr-14 font-medium leading-5 text-foreground", wide ? "text-ui" : "text-ui")}>{issue.title}</span>
            <span className="mt-1 flex items-center gap-1.5 text-caption text-muted-foreground"><span className="font-mono tabular-nums">#{issue.number}</span><span aria-hidden="true">·</span>{wide && <Monogram login={issue.author?.login} className="size-4 text-micro" />}<span>{issue.author?.login ?? "ghost"}</span>{!Number.isNaN(Date.parse(issue.updatedAt)) && <><span aria-hidden="true">·</span><span>updated {relativeTime(issue.updatedAt, new Date())}</span></>}</span>
            {!!issue.labels.length && <span className="mt-2 flex flex-wrap gap-1">{issue.labels.map(label => <LabelBadge key={label.name} label={label} />)}</span>}
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
  return <div className="animate-page-mount @container min-h-0 flex-1 overflow-y-auto px-4 py-4 @3xl:px-5">
    <div className="rounded-xl border border-border bg-card p-5 shadow-xs">
      <div className="flex items-start gap-3">
        <span className="grid size-9 shrink-0 place-items-center rounded-lg border border-border bg-card"><FolderGit2 size={16} aria-hidden="true" /></span>
        <div className="min-w-0 flex-1">
          <h2 className="truncate font-display text-base font-semibold text-foreground">{overview.nameWithOwner}</h2>
          <p className="mt-1 text-meta leading-relaxed text-muted-foreground">{overview.description || "No repository description."}</p>
        </div>
        <Badge variant="outline" size="sm">{overview.visibility.toLowerCase()}</Badge>
      </div>
      <div className="mt-3 flex flex-wrap items-center gap-1">
        <CopyButton value={overview.url} label="Copy the repository link" className="border border-border bg-card px-2">Copy link</CopyButton>
        <CopyButton value={`git clone ${overview.url}.git`} label="Copy the clone command" className="border border-border bg-card px-2">Copy clone command</CopyButton>
        <OpenOnGithub url={overview.url} what={overview.nameWithOwner} />
      </div>
      <div className="mt-5 grid grid-cols-2 gap-2 @2xl:grid-cols-4">{facts.map(([label, value, Icon]) => <div key={label} className="rounded-lg bg-muted/50 px-3 py-3"><p className="flex items-center gap-1.5 text-caption text-muted-foreground"><Icon size={11} aria-hidden="true" />{label}</p><p className="mt-1.5 truncate text-message font-semibold tabular-nums text-foreground">{value}</p></div>)}</div>
      <section className="mt-5"><SectionLabel>Labels</SectionLabel><div className="flex flex-wrap gap-1.5">{overview.labels.length ? overview.labels.map(label => <LabelBadge key={label.name} label={label} />) : <p className="text-meta text-muted-foreground">No labels.</p>}</div></section>
    </div>
  </div>;
}

// ── Detail ───────────────────────────────────────────────────────────────────

type PendingAction = {
  icon: LucideIcon;
  tone: "neutral" | "success" | "destructive";
  /** What the sheet is called: "Approve pull request #341". */
  title: string;
  /** The consequence in one sentence. */
  consequence: string;
  /** The exact `gh` command behind the sheet's disclosure; actions that are
   *  not one gh invocation (checkout) omit it. */
  command?: string;
  requiresBody: boolean;
  body?: string;
  build: (body: string) => GithubAction;
  /** The verb the confirm button wears. */
  confirmLabel: string;
  destructive?: boolean;
};

/** The harnesses a review job can run on. Model, effort and instructions come
 * from Settings → Pull request reviewer, so the user only picks the agent
 * here; with Second opinion on, they pick two. Cursor Bugbot is not a local
 * job: it posts `cursor review` on the PR. */
const REVIEW_HARNESSES: ReadonlyArray<{ id: string; label: string }> = [
  { id: "claude", label: "Claude" },
  { id: "codex", label: "Codex" },
  { id: "opencode", label: "OpenCode" },
];
const BUGBOT = { id: "bugbot", label: "Cursor Bugbot", note: "reviews on GitHub" };

const ACTION_BUTTON = "inline-flex min-h-7 items-center gap-1.5 rounded-md border border-border bg-card px-2.5 py-1 text-meta font-medium text-foreground shadow-xs transition-colors hover:bg-accent active:scale-[0.98] disabled:opacity-50";
/** The one action a PR is for. Ink on paper, like every other primary button
 *  in the app, rather than a green that reads as a status. */
const PRIMARY_ACTION = "inline-flex min-h-7 items-center gap-1.5 rounded-md bg-primary px-3 py-1 text-meta font-medium text-primary-foreground shadow-xs transition-colors hover:bg-primary/90 active:scale-[0.98] disabled:opacity-50";
/** Ending something is available but never loud until a pointer is on it. */
const QUIET_ACTION = "ml-auto inline-flex min-h-7 items-center gap-1.5 rounded-md px-2.5 py-1 text-meta font-medium text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive active:scale-[0.98] disabled:opacity-50";

const PILL_BASE = "inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-caption font-medium capitalize shadow-xs";
const PILL_MERGED = "border-merged/45 bg-merged/15 text-merged";
const PILL_NEUTRAL = "border-border bg-card text-foreground";

/** The state pill. Between labels it tweens its width and cross-fades the
 *  glyph — Open slides up and out, Merged slides up and in (320ms); the
 *  merged verdict wears the one deliberate purple on this chrome. */
function StatePill({ icon: Icon, label, merged = false }: { icon: typeof GitPullRequest; label: string; merged?: boolean }) {
  const reduced = useReducedMotion();
  if (reduced) {
    return <span className={cn(PILL_BASE, merged ? PILL_MERGED : PILL_NEUTRAL)}>
      <Icon size={12} strokeWidth={1.9} aria-hidden="true" />{label}
    </span>;
  }
  return <motion.span layout className={cn(PILL_BASE, "overflow-hidden", merged ? PILL_MERGED : PILL_NEUTRAL)}>
    <AnimatePresence mode="popLayout" initial={false}>
      <motion.span
        key={label}
        layout
        className="flex items-center gap-1 whitespace-nowrap"
        initial={{ y: 8, opacity: 0 }}
        animate={{ y: 0, opacity: 1 }}
        exit={{ y: -8, opacity: 0 }}
        transition={{ duration: 0.32, ease: [0.22, 1, 0.36, 1] }}
      >
        <Icon size={12} strokeWidth={1.9} aria-hidden="true" />{label}
      </motion.span>
    </AnimatePresence>
  </motion.span>;
}

const TAB_ORDER: ReadonlyArray<PullRequestTab> = ["conversation", "changes", "commits", "checks"];

/** The tab panel slides in from the direction of travel (issue #1028); the
 *  underline beneath the strip glides via its layoutId. */
const TAB_PANEL_VARIANTS: Variants = {
  enter: (direction: number) => ({ x: direction >= 0 ? 18 : -18, opacity: 0 }),
  center: { x: 0, opacity: 1, transition: { duration: 0.32, ease: [0.22, 1, 0.36, 1] } },
  exit: (direction: number) => ({ x: direction >= 0 ? -18 : 18, opacity: 0, transition: { duration: 0.2, ease: "easeIn" } }),
};

const TAB_PANEL_REDUCED_VARIANTS: Variants = {
  enter: () => ({ opacity: 0 }),
  center: { opacity: 1, transition: { duration: 0.12 } },
  exit: () => ({ opacity: 0, transition: { duration: 0.12 } }),
};

/** List → detail push (issue #1028): the list exits left while the detail
 *  slides in from the right (320ms); back reverses; a plain surface switch
 *  cross-fades. Reduced motion fades only. */
const PUSH_VARIANTS: Variants = {
  enter: (direction: string) => direction === "forward"
    ? { x: 24, opacity: 0 }
    : direction === "back" ? { x: -24, opacity: 0 } : { opacity: 0 },
  center: { x: 0, opacity: 1, transition: { duration: 0.32, ease: [0.22, 1, 0.36, 1] } },
  exit: (direction: string) => {
    if (direction === "back") return { x: "30%", opacity: 0, transition: { duration: 0.32, ease: [0.22, 1, 0.36, 1] } };
    if (direction === "none") return { opacity: 0, transition: { duration: 0 } };
    return { x: "-30%", opacity: 0, transition: { duration: 0.32, ease: [0.22, 1, 0.36, 1] } };
  },
};

const PUSH_REDUCED_VARIANTS: Variants = {
  enter: () => ({ opacity: 0 }),
  center: { opacity: 1, transition: { duration: 0.12 } },
  exit: () => ({ opacity: 0, transition: { duration: 0.12 } }),
};

/** A label that rolls up to its new value when the verdict changes
 *  (Running → Passing), the mockup's vertical roll. */
function RollLabel({ value }: { value: string }) {
  const previous = useRef(value);
  const [roll, setRoll] = useState<{ from: string; to: string } | null>(null);
  useEffect(() => {
    if (previous.current === value) return;
    const from = previous.current;
    previous.current = value;
    setRoll({ from, to: value });
    const timer = window.setTimeout(() => setRoll(null), 450);
    return () => window.clearTimeout(timer);
  }, [value]);
  if (!roll) return <span>{value}</span>;
  return <span className="relative inline-block h-[15px] overflow-hidden align-baseline" role="status" aria-label={roll.to}>
    <span aria-hidden="true" className="github-roll-up absolute inset-x-0 top-0 block">
      <span className="block h-[15px] leading-[15px]">{roll.from}</span>
      <span className="block h-[15px] leading-[15px]">{roll.to}</span>
    </span>
  </span>;
}

/** A check that settles between two reads: one row flash, one glyph spring,
 *  then both sit still (issue #1028's check-settle). */
function useCheckSettle(checks: GithubChecksResult["checks"]): ReadonlySet<string> {
  const [settled, setSettled] = useState<ReadonlySet<string>>(() => new Set());
  const previousStatuses = useRef(new Map<string, string>());
  useEffect(() => {
    const next = new Set<string>();
    for (const check of checks) {
      const key = `${check.workflow}·${check.name}`;
      const was = previousStatuses.current.get(key);
      if (check.status === "completed" && check.conclusion === "success" && was !== undefined && was !== "completed") next.add(key);
      previousStatuses.current.set(key, check.status);
    }
    if (!next.size) return;
    setSettled(next);
    const timer = window.setTimeout(() => setSettled(new Set()), 750);
    return () => window.clearTimeout(timer);
  }, [checks]);
  return settled;
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
  return <div className="sticky top-0 z-[1] flex border-b border-border bg-background/95 px-3.5 backdrop-blur" role="tablist" aria-label={label}>
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
      className={cn("relative px-2.5 py-2.5 text-meta capitalize transition-colors", active === tab.value ? "font-medium text-foreground" : "text-muted-foreground hover:text-foreground")}
    >
      {tab.label}
      {active === tab.value && <motion.span layoutId={`${id}-underline`} className="absolute inset-x-1.5 -bottom-px h-0.5 rounded-full bg-foreground" aria-hidden="true" />}
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
  /** Which tab a deep link asked for, if it asked. Nonce-gated like the pane's
   * own intent, so arriving from `/pull/12/files` selects Changes without
   * fighting the reader who then clicks Conversation. */
  focus?: { tab: PullRequestTab; nonce: number };
  availableLabels: GithubLabel[];
  onBack: () => void;
  onActed: () => void;
  onJumpToFile: (path: string, line: number | undefined, headBranch: string) => void;
};

function PullRequestDetail({ workspaceId, workspaceBranch, sessionId, repository, repositoryUrl, number, detail, error, focus, availableLabels, onBack, onActed, onJumpToFile }: PullRequestDetailProps) {
  const [pending, setPending] = useState<PendingAction>();
  const [reviewOpen, setReviewOpen] = useState(false);
  const [reviewBusy, setReviewBusy] = useState(false);
  const [reviewNotice, setReviewNotice] = useState<{ tone: "success" | "error" | "info"; text: string }>();
  // The job a start returned, so the Reviews section shows it at once (an
  // idempotent start sends no `reviews/job_updated`).
  const [startedJob, setStartedJob] = useState<ReviewJob>();
  const reviews = usePullRequestReviews(workspaceId, number, startedJob);
  // Bumped to bring the Reviews section into view from the status line.
  const [revealReviews, setRevealReviews] = useState(0);
  const [secondOpinion, setSecondOpinion] = useState(false);
  const [picked, setPicked] = useState<string[]>([]);
  // Picks belong to one opening of the picker on one PR.
  useEffect(() => { if (!reviewOpen) setPicked([]); }, [reviewOpen]);
  useEffect(() => { setPicked([]); }, [number]);
  useEffect(() => {
    if (!reviewOpen) return;
    let active = true;
    void bridgeApi.getReviewJobSettings()
      .then(settings => { if (active) setSecondOpinion(settings.secondOpinion === true); })
      .catch(() => { if (active) setSecondOpinion(false); });
    return () => { active = false; };
  }, [reviewOpen]);
  const [pendingBody, setPendingBody] = useState("");
  // The merge flow's one state machine: idle | confirming | merging | merged
  // | failed, in a reducer rather than five booleans (issue #1028). A detail
  // that already reports merged renders the receipt without replaying it.
  const [merge, dispatchMerge] = useReducer(mergeReducer, undefined, initialMergeState);
  const [mergeConfig, setMergeConfig] = useState<GithubMergeConfigResult>();
  const [checkoutOpen, setCheckoutOpen] = useState(false);
  const [checkout, setCheckout] = useState<GithubCheckoutResult>();
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string>();
  const [tab, setTab] = useState<PullRequestTab>("conversation");
  // The panel slides in from the direction of travel (issue #1028).
  const reducedMotion = useReducedMotion();
  const tabDirection = useRef(1);
  const selectTab = useCallback((next: PullRequestTab) => {
    tabDirection.current = Math.sign(TAB_ORDER.indexOf(next) - TAB_ORDER.indexOf(tab));
    setTab(next);
  }, [tab]);
  const seenFocus = useRef(0);
  useEffect(() => {
    if (!focus || focus.nonce === seenFocus.current) return;
    seenFocus.current = focus.nonce;
    setTab(focus.tab);
  }, [focus]);
  const detailId = useId();
  const [labelsOpen, setLabelsOpen] = useState(false);

  const repoLabel = repository ? `${repository.owner}/${repository.name}` : "this repository";
  const closeOverlays = () => { setPending(undefined); setPendingBody(""); setCheckoutOpen(false); setLabelsOpen(false); setReviewOpen(false); };

  // Queue a review job: Bridge prepares the PR, the reviewer drafts findings,
  // and the Reviews section below follows it live. Cursor Bugbot instead
  // posts `cursor review` and reviews on GitHub.
  const startReview = async (harnesses: string[]) => {
    setReviewBusy(true); setReviewNotice(undefined);
    try {
      if (harnesses[0] === BUGBOT.id) {
        const result = await bridgeApi.githubReview(workspaceId, number, BUGBOT.id, sessionId);
        setReviewNotice({ tone: result.status === "failed" ? "error" : "success", text: result.message });
        if (result.status !== "failed") onActed();
      } else {
        const existing = activeJobFor(reviews.jobs, harnesses);
        const result = await bridgeApi.startReview({ workspaceId, number, harnesses });
        setStartedJob(result.job);
        // A start that found a review already under way, or a draft waiting,
        // gets that job back: say so rather than look like nothing happened.
        if (existing?.id === result.job.id || reviews.jobs.some(job => job.id === result.job.id)) {
          const who = result.job.harnesses.map(id => REVIEW_HARNESSES.find(choice => choice.id === id)?.label ?? id).join(" and ");
          setReviewNotice({ tone: "info", text: result.job.status === "drafted" ? `${who} already has a draft waiting for you on this pull request.` : `${who} ${result.job.harnesses.length > 1 ? "are" : "is"} already reviewing this pull request.` });
        }
      }
      setReviewOpen(false);
    } catch (value) {
      setReviewNotice({ tone: "error", text: errorMessage(value) });
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
      setActionError(errorMessage(value));
    } finally { setBusy(false); }
  };

  // Comments, replies and resolving a conversation post on their own button
  // press: the button names the write, and none of them needs a second sheet.
  // The reason for a refusal goes back to the control that asked.
  const write = useCallback(async (action: GithubAction): Promise<string | undefined> => {
    try {
      const outcome = await bridgeApi.githubAct(workspaceId, action, true);
      if (!outcome.executed) return outcome.message;
      onActed();
      return undefined;
    } catch (value) {
      return errorMessage(value);
    }
  }, [workspaceId, onActed]);

  // Avatars GitHub returned with the review threads, for everyone on the PR.
  const threads = detail?.result.reviewThreads;
  const avatars = useMemo(() => {
    const map = new Map<string, string>();
    for (const thread of threads ?? []) {
      for (const comment of thread.comments) {
        if (comment.author?.avatarUrl) map.set(comment.author.login, comment.author.avatarUrl);
      }
    }
    return map;
  }, [threads]);

  // The merge box reads the repository's strategies when it mounts, not on
  // click, so the split button already shows the repo default. A failed read
  // leaves the button disabled rather than guessing at the repository rules.
  const detailReady = detail?.result.pullRequest.summary;
  const detailNumber = detailReady?.number;
  useEffect(() => {
    if (!detailReady || detailReady.state !== "open") return;
    let cancelled = false;
    void bridgeApi.githubMergeConfig(workspaceId)
      .then(config => {
        if (cancelled) return;
        setMergeConfig(config);
        // The split button already shows the repo default.
        dispatchMerge({ type: "config", strategy: config.defaultStrategy });
      })
      .catch(() => undefined);
    return () => { cancelled = true; };
    // Reload per pull request, not per render; the config is repository-level.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workspaceId, detailNumber]);

  // Reset the flow when another pull request opens in this detail.
  useEffect(() => {
    if (detailReady) dispatchMerge({ type: "reset", summary: detailReady });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [detailNumber]);

  // The merge itself: one confirmed write; the animation only plays on a real
  // `executed: true`, and a refusal keeps the form with the reason verbatim.
  const runMerge = async () => {
    if (merge.phase !== "confirming" && merge.phase !== "failed") return;
    dispatchMerge({ type: "merging" });
    setBusy(true); setActionError(undefined);
    try {
      const outcome = await bridgeApi.githubAct(workspaceId, {
        kind: "merge",
        number,
        strategy: merge.strategy,
        // The confirm form's fields; empty means `gh`'s own default.
        // Rebase writes no commit of its own; the hidden field is not sent.
        subject: merge.strategy === "rebase" ? undefined : merge.subject.trim() || undefined,
        body: undefined,
        deleteBranch: merge.deleteBranch || undefined,
      }, true);
      if (!outcome.executed) { dispatchMerge({ type: "fail", message: outcome.message }); return; }
      dispatchMerge({ type: "succeed" });
      onActed();
    } catch (value) {
      dispatchMerge({ type: "fail", message: errorMessage(value) });
    } finally { setBusy(false); }
  };

  // The confirm step's own chords: ⌘↵ merges, Esc collapses back to idle.
  const mergeOpenForKeys = merge.phase === "confirming" || merge.phase === "failed";
  useEffect(() => {
    if (!mergeOpenForKeys) return;
    const onKey = (event: KeyboardEvent) => {
      // Only keys meant for this pane: not one a sheet already took, not a
      // chord typed elsewhere in the app, not while a write is running.
      if (event.defaultPrevented || pending || checkoutOpen || busy) return;
      const target = event.target;
      const inPane = target === document.body || (target instanceof Element && target.closest("[data-gh-pane-root]") !== null);
      if (!inPane) return;
      if (event.key === "Escape") dispatchMerge({ type: "collapse" });
      else if (event.key === "Enter" && (event.metaKey || event.ctrlKey) && !event.altKey && !event.shiftKey) {
        event.preventDefault();
        void runMerge();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  });

  const runCheckout = async () => {
    setBusy(true); setActionError(undefined);
    try {
      const result = await bridgeApi.githubCheckout(workspaceId, number);
      closeOverlays();
      setCheckout(result);
    } catch (value) {
      setActionError(errorMessage(value));
    } finally { setBusy(false); }
  };

  const header = <div className="flex shrink-0 items-center gap-1 border-b border-border px-2 py-1.5">
    <button type="button" onClick={onBack} className="inline-flex min-h-7 items-center gap-1.5 rounded-md px-2 py-1 text-meta text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">
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
  const mergedNow = summary.state === "merged" || merge.phase === "merged";
  const StateIcon = pullRequestStateIcon(summary);
  const tabs = [
    { value: "conversation" as const, label: "conversation" },
    { value: "changes" as const, label: `changes ${pullRequest.changedFiles}` },
    { value: "commits" as const, label: `commits ${pullRequest.commits.length}` },
    { value: "checks" as const, label: `checks ${detail.checks.checks.length}` },
  ];

  // Secondary actions beyond Approve and Review ride the box's overflow menu
  // (at most three visible secondaries plus the overflow, per the issue).
  const overflow: OverflowItem[] = [];
  if (summary.state === "open") {
    overflow.push({ label: "Request changes", icon: MessageSquare, onSelect: () => setPending({
      title: `Request changes on #${number}`,
      consequence: `Blocks the merge until ${summary.author?.login ?? "the author"} pushes a fix or you dismiss the review.`,
      command: `gh pr review ${number} -R ${repoLabel} --request-changes --body …`,
      tone: "neutral",
      icon: MessageSquare,
      requiresBody: true,
      confirmLabel: "Request changes",
      build: body => ({ kind: "review", number, event: "requestChanges", body }),
    }) });
    if (summary.isDraft) overflow.push({ label: "Ready for review", icon: Eye, onSelect: () => setPending({
      title: `Mark #${number} ready for review`,
      consequence: "Reviewers are notified that the pull request is ready.",
      command: `gh pr ready ${number} -R ${repoLabel}`,
      tone: "neutral",
      icon: Eye,
      requiresBody: false,
      confirmLabel: "Ready for review",
      build: () => ({ kind: "ready", number }),
    }) });
    if (rerunnable) overflow.push({ label: "Re-run failed", icon: RotateCcw, onSelect: () => setPending({
      title: `Re-run failed checks on #${number}`,
      consequence: "The failed runs are queued again.",
      command: `gh run rerun <failed runs of #${number}> -R ${repoLabel}`,
      tone: "neutral",
      icon: RotateCcw,
      requiresBody: false,
      confirmLabel: "Re-run failed",
      build: () => ({ kind: "rerun", number }),
    }) });
    if (!checkedOutHere) overflow.push({ label: "Check out", icon: FolderGit2, onSelect: () => { setActionError(undefined); setCheckoutOpen(true); } });
    overflow.push({ label: "Close", icon: Ban, destructive: true, onSelect: () => setPending({
      title: `Close #${number} without merging`,
      consequence: "The branch stays on GitHub. You can reopen it from this pane.",
      command: `gh pr close ${number} -R ${repoLabel}`,
      tone: "destructive",
      icon: Ban,
      requiresBody: false,
      confirmLabel: "Close pull request",
      destructive: true,
      build: () => ({ kind: "setState", target: "pullRequest", number, operation: "close" }),
    }) });
  }

  return <AvatarScope.Provider value={avatars}><div className="relative flex min-h-0 flex-1 flex-col">
    {header}
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className={cn("animate-page-mount relative border-b border-border px-5 pb-4 pt-4", merge.celebrate && mergedNow && "merge-sweep play")}>
        <div className="flex items-center gap-2 text-caption text-muted-foreground">
          <StatePill icon={mergedNow ? GitMerge : StateIcon} label={mergedNow ? "merged" : summary.isDraft ? "draft" : summary.state} merged={mergedNow} />
          <span className="font-mono tabular-nums">#{summary.number}</span>
          <span className="flex min-w-0 items-center gap-1.5"><Avatar login={summary.author?.login} className="size-4 text-micro" /><span className="truncate">{summary.author?.login ?? "ghost"}</span></span>
          {review && <Badge variant={review.variant} size="sm">{review.label}</Badge>}
          <span className="ml-auto flex shrink-0 items-center">
            {sessionId && <ShowInChat sessionId={sessionId} url={summary.url} number={summary.number} />}
            <CopyButton value={summary.url} label={`Copy the link to #${summary.number}`} />
            <OpenOnGithub url={summary.url} what={`#${summary.number}`} />
          </span>
        </div>
        <h2 className="mt-2.5 font-display text-[19px] font-semibold leading-snug tracking-[-0.015em] text-foreground">{summary.title}</h2>
        <p className="mt-2 flex flex-wrap items-center gap-x-1.5 gap-y-1 text-meta text-muted-foreground">
          <b className="font-medium text-foreground">{summary.author?.login ?? "ghost"}</b>
          {mergedNow
            ? <>merged <b className="font-medium text-foreground">{pullRequest.commits.length} commits</b> into <BranchChip name={pullRequest.baseBranch} />{merge.celebrate && <span>· just now</span>}</>
            : <>wants to merge <b className="font-medium text-foreground">{pullRequest.commits.length} commits</b> into <BranchChip name={pullRequest.baseBranch} /> from <BranchChip name={summary.headBranch} />
              <CopyButton value={summary.headBranch} label={`Copy the branch name ${summary.headBranch}`} />
              {checkedOutHere && <span className="inline-flex items-center gap-1 rounded-full border border-border px-2 py-px text-caption text-foreground"><Check size={11} aria-hidden="true" />Checked out here</span>}
            </>}
        </p>
        {summary.state === "closed" && summary.mergeability === "conflicting" && <p className="mt-2 flex items-start gap-1.5 rounded-md border border-warning/25 bg-warning/10 px-2.5 py-1.5 text-caption text-warning">
          <TriangleAlert size={12} className="mt-0.5 shrink-0" aria-hidden="true" />
          <span>This branch has conflicts with {pullRequest.baseBranch}. GitHub reports its merge state as <span className="font-mono">{summary.mergeStateStatus.toLowerCase()}</span>.</span>
        </p>}
        {summary.state === "closed" && summary.checks.total > 0 && <div className="mt-3 flex items-center gap-3">
          <ChecksCell pr={summary} />
        </div>}
        <div className="mt-2 flex flex-wrap items-center gap-1.5">
          {pullRequest.labels.map(label => <LabelBadge key={label.name} label={label} />)}
          <button type="button" onClick={() => setLabelsOpen(value => !value)} className="inline-flex min-h-7 items-center gap-1 rounded-md px-2 py-1 text-caption text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">
            <Tag size={10.5} aria-hidden="true" /> Manage labels
          </button>
        </div>
        {labelsOpen && <LabelControls
          available={availableLabels}
          current={pullRequest.labels}
          disabled={busy}
          onChange={(label, operation) => setPending({
            title: `${operation === "add" ? "Add" : "Remove"} label “${label}”`,
            consequence: `The pull request is edited on ${repoLabel}.`,
            command: `gh pr edit ${number} -R ${repoLabel} ${operation === "add" ? "--add-label" : "--remove-label"} ${JSON.stringify(label)}`,
            tone: "neutral",
            icon: Tag,
            requiresBody: false,
            confirmLabel: operation === "add" ? "Add label" : "Remove label",
            build: () => ({ kind: "label", target: "pullRequest", number, label, operation }),
          })}
        />}

        {(summary.state === "open" || summary.state === "merged") && <>
          <MergeBox
            summary={summary}
            checks={detail.checks.checks}
            commitCount={pullRequest.commits.length}
            baseBranch={pullRequest.baseBranch}
            headBranch={summary.headBranch}
            prUrl={summary.url}
            config={mergeConfig}
            state={merge}
            dispatch={dispatchMerge}
            busy={busy}
            reviewBusy={reviewBusy}
            reviewOpen={reviewOpen}
            reviewing={runningCount(reviews.jobs)}
            onToggleReview={() => { setReviewNotice(undefined); setReviewOpen(value => !value); }}
            onApprove={() => setPending({
              title: `Approve pull request #${number}`,
              consequence: `Your review is posted as an approval. ${summary.author?.login ?? "the author"} is notified.`,
              command: `gh pr review ${number} -R ${repoLabel} --approve`,
              tone: "success",
              icon: ThumbsUp,
              requiresBody: false,
              confirmLabel: "Approve",
              build: () => ({ kind: "review", number, event: "approve", body: "" }),
            })}
            overflow={overflow}
            onConfirm={() => void runMerge()}
            onRestore={() => setPending({
              title: `Restore ${summary.headBranch}`,
              consequence: "Recreates the deleted head branch at its merge commit. Nothing is merged or changed.",
              command: `gh api repos/${repoLabel}/git/refs --method POST -f ref=refs/heads/${summary.headBranch} -f sha=…`,
              tone: "neutral",
              icon: RotateCcw,
              requiresBody: false,
              confirmLabel: "Restore branch",
              build: () => ({ kind: "restoreBranch", number, branch: summary.headBranch }),
            })}
          />
          {summary.state === "open" && reviewOpen && <div className="animate-page-mount mt-2 grid gap-1 rounded-lg border border-border bg-card p-1.5" role="group" aria-label="Review harness">
            <p className="px-2 pb-0.5 pt-1 text-caption font-semibold text-muted-foreground">RUN REVIEW WITH</p>
            {secondOpinion && <p className="px-2 pb-0.5 text-caption text-muted-foreground">Pick two for a second opinion.</p>}
            {REVIEW_HARNESSES.map(choice => {
              const checked = picked.includes(choice.id);
              return secondOpinion
                ? <button key={choice.id} type="button" role="checkbox" aria-checked={checked} disabled={reviewBusy || (!checked && picked.length >= 2)} onClick={() => setPicked(current => checked ? current.filter(id => id !== choice.id) : [...current, choice.id])} className="flex items-center gap-2 rounded-md px-2 py-1 text-left text-meta text-foreground transition-colors hover:bg-accent disabled:opacity-50">
                    <span aria-hidden="true" className={cn("grid size-3 place-items-center rounded-sm border border-border", checked && "border-primary bg-primary text-primary-foreground")}>{checked && <Check size={9} />}</span>
                    <span className="min-w-0 flex-1 truncate">{choice.label}</span>
                  </button>
                : (() => {
                    const inUse = activeJobFor(reviews.jobs, [choice.id]);
                    return <button key={choice.id} type="button" disabled={reviewBusy || !!inUse} onClick={() => void startReview([choice.id])} className="flex items-center gap-2 rounded-md px-2 py-1 text-left text-meta text-foreground transition-colors hover:bg-accent disabled:opacity-50">
                      {inUse && inUse.status !== "drafted"
                        ? <SkeletonDot className="size-[11px]" />
                        : <Sparkles size={11} className="text-muted-foreground" aria-hidden="true" />}
                      <span className="min-w-0 flex-1 truncate">{choice.label}</span>
                      {inUse && <span className="shrink-0 text-micro text-muted-foreground">{inUse.status === "drafted" ? "Draft waiting" : "Reviewing"}</span>}
                    </button>;
                  })();
            })}
            {secondOpinion && (() => {
              const inUse = picked.length > 0 ? activeJobFor(reviews.jobs, picked) : undefined;
              const names = picked.map(id => REVIEW_HARNESSES.find(choice => choice.id === id)?.label ?? id).join(" and ");
              return <button type="button" disabled={reviewBusy || picked.length === 0 || !!inUse} aria-busy={reviewBusy || undefined} onClick={() => void startReview(picked)} className={cn(PRIMARY_ACTION, "mx-1 my-0.5 justify-center", pendingClass(reviewBusy))}>
                {picked.length === 0 ? "Pick a harness" : inUse ? (inUse.status === "drafted" ? `A draft from ${names} is waiting` : `${names} already reviewing`) : `Review with ${names}`}
              </button>;
            })()}
            <button type="button" disabled={reviewBusy} onClick={() => void startReview([BUGBOT.id])} className="flex items-center gap-2 rounded-md px-2 py-1 text-left text-meta text-foreground transition-colors hover:bg-accent disabled:opacity-50">
              <Sparkles size={11} className="text-muted-foreground" aria-hidden="true" />
              <span className="min-w-0 flex-1 truncate">{BUGBOT.label}</span>
              <span className="shrink-0 text-micro text-muted-foreground">{BUGBOT.note}</span>
            </button>
          </div>}
          {summary.state === "open" && reviewNotice && <p role="status" className={cn("animate-page-mount mt-2 rounded-md border px-2.5 py-1.5 text-meta", reviewNotice.tone === "success" ? "border-success/25 bg-success/10 text-success" : reviewNotice.tone === "info" ? "border-border bg-muted/40 text-foreground" : "border-destructive/25 bg-destructive/10 text-destructive")}>{reviewNotice.text}</p>}
          <ReviewStatusLine reviews={reviews} onShow={() => { selectTab("conversation"); setRevealReviews(value => value + 1); }} />
        </>}
        {summary.state === "closed" && <div className="mt-3 flex flex-wrap items-center gap-1.5">
          <button type="button" disabled={busy} className={ACTION_BUTTON} onClick={() => setPending({
            title: `Reopen #${number}`,
            consequence: "The pull request is open again for review and comments.",
            command: `gh pr reopen ${number} -R ${repoLabel}`,
            tone: "neutral",
            icon: RotateCcw,
            requiresBody: false,
            confirmLabel: "Reopen",
            build: () => ({ kind: "setState", target: "pullRequest", number, operation: "reopen" }),
          })}><RotateCcw size={11.5} aria-hidden="true" /> Reopen</button>
        </div>}
      </div>

      {actionError && <p role="alert" className="border-b border-border bg-destructive/10 px-4 py-2 text-meta text-destructive">{actionError}</p>}
      {error && <p role="alert" className="border-b border-border bg-warning/10 px-4 py-2 text-meta text-warning">Live refresh failed: {error}</p>}
      {checkout && <p className="border-b border-border bg-success/10 px-4 py-2 text-meta text-success">
        {checkout.reused ? "Reusing the task worktree already on " : "Checked out into a task worktree on "}
        <span className="font-mono">{checkout.branch}</span>
        <span className="block truncate font-mono text-caption text-success/80" title={checkout.path}>{checkout.path}</span>
      </p>}

      <DetailTabs id={detailId} tabs={tabs} active={tab} onSelect={selectTab} label="Pull request detail" />

      <div id={`${detailId}-panel`} role="tabpanel" aria-labelledby={`${detailId}-${tab}`} tabIndex={0} className="relative min-h-0">
        <AnimatePresence mode="popLayout" initial={false} custom={tabDirection.current}>
          <motion.div
            key={tab}
            custom={tabDirection.current}
            variants={reducedMotion ? TAB_PANEL_REDUCED_VARIANTS : TAB_PANEL_VARIANTS}
            initial="enter"
            animate="center"
            exit="exit"
            className="space-y-6 px-5 py-5"
          >
        {tab === "checks" && <ChecksList checks={detail.checks.checks} rollup={summary.checks} />}

        {tab === "commits" && <CommitList commits={pullRequest.commits} prUrl={summary.url} />}

        {tab === "changes" && <section aria-label="Changed files">
          <div className="mb-3 flex items-center gap-2 text-caption text-muted-foreground">
            <FileDiff size={13} aria-hidden="true" />
            <span>{pullRequest.changedFiles} changed files</span>
            <span className="ml-auto font-mono text-success">+{pullRequest.additions}</span>
            <span className="font-mono text-destructive">−{pullRequest.deletions}</span>
          </div>
          {detail.result.files.length ? <div className="space-y-3">{detail.result.files.map(file => <FileChange
            key={file.path}
            file={file}
            defaultOpen={detail.result.files.length <= EAGER_PATCH_FILE_LIMIT}
            fullDiffUrl={`${summary.url}/files`}
            onComment={summary.state === "open" ? (line, side, body) => write({ kind: "reviewComment", number, path: file.path, line, side, body }) : undefined}
          />)}</div> : <p className="text-meta text-muted-foreground">No changed files reported.</p>}
        </section>}

        {tab === "conversation" && <>
        <section aria-label="Description">
          <SectionLabel>Description</SectionLabel>
          {pullRequest.body ? <GithubMarkdown text={pullRequest.body} repositoryUrl={repositoryUrl} /> : <p className="text-meta italic text-muted-foreground">No description provided.</p>}
        </section>

        <ReviewJobsView reviews={reviews} repositoryUrl={repositoryUrl} reveal={revealReviews} />

        <section aria-label="Conversation comments">
          <SectionLabel count={pullRequest.comments.length}>Conversation</SectionLabel>
          {pullRequest.comments.length ? <CommentList comments={pullRequest.comments} repositoryUrl={repositoryUrl} /> : <p className="text-meta text-muted-foreground">No conversation comments.</p>}
          <div className="mt-2.5">
            <Composer
              label="New comment"
              placeholder="Write a comment…"
              submitLabel="Comment"
              disabled={busy}
              onSubmit={body => write({ kind: "comment", target: "pullRequest", number, body })}
            />
          </div>
        </section>

        <ReviewThreads
          threads={reviewThreads}
          number={number}
          repositoryUrl={repositoryUrl}
          headBranch={summary.headBranch}
          checkedOutHere={checkedOutHere}
          write={write}
          onJumpToFile={onJumpToFile}
        />
        </>}
          </motion.div>
        </AnimatePresence>
      </div>
    </div>

    {/* The host keeps each sheet mounted for its exit motion. */}
    <ActionSheetHost>
      {pending && <ActionSheet
        key="pending"
        icon={pending.icon}
        tone={pending.tone}
        title={pending.title}
        consequence={pending.consequence}
        command={pending.command}
        requiresBody={pending.requiresBody}
        body={pending.body ?? pendingBody}
        onBody={setPendingBody}
        readOnlyBody={pending.body !== undefined}
        confirmLabel={pending.confirmLabel}
        destructive={pending.destructive}
        busy={busy}
        onCancel={closeOverlays}
        onConfirm={() => void submit(pending.build(pending.body ?? pendingBody))}
      />}

      {checkoutOpen && <ActionSheet
        key="checkout"
        icon={FolderGit2}
        tone="neutral"
        title={`Check out #${number} (${summary.headBranch})`}
        consequence="Creates an isolated worktree beside this one — your current checkout is untouched. Repeating it reuses the same worktree."
        requiresBody={false}
        body=""
        onBody={() => undefined}
        busy={busy}
        confirmLabel="Check out"
        onCancel={closeOverlays}
        onConfirm={() => void runCheckout()}
      />}
    </ActionSheetHost>
  </div></AvatarScope.Provider>;
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
  const header = <div className="flex shrink-0 items-center border-b border-border px-2 py-1.5"><button type="button" onClick={onBack} className="inline-flex min-h-7 items-center gap-1.5 rounded-md px-2 py-1 text-meta text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"><ArrowLeft size={12.5} aria-hidden="true" /> All issues</button></div>;

  const submit = async (action: GithubAction) => {
    setBusy(true); setActionError(undefined);
    try {
      const outcome = await bridgeApi.githubAct(workspaceId, action, true);
      if (!outcome.executed) { setActionError(outcome.message); return; }
      setPending(undefined); setLabelsOpen(false); onActed();
    } catch (value) {
      setActionError(errorMessage(value));
    } finally { setBusy(false); }
  };

  if (error && !detail) return <div className="flex min-h-0 flex-1 flex-col">{header}<PaneNotice icon={CircleX} title={`Issue #${number} did not load`}>{error}</PaneNotice></div>;
  if (!detail) return <div className="flex min-h-0 flex-1 flex-col">{header}<DetailSkeleton label={`Opening issue #${number}`} /></div>;
  const { issue } = detail;
  const open = issue.summary.state === "open";

  return <div className="relative flex min-h-0 flex-1 flex-col">
    {header}
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="animate-page-mount border-b border-border px-5 py-4">
        <div className="flex items-center gap-2 text-caption text-muted-foreground">
          <StatePill icon={CircleDot} label={issue.summary.state} />
          <span className="font-mono tabular-nums">#{issue.summary.number}</span>
          <span className="ml-auto flex shrink-0 items-center">
            <CopyButton value={issue.summary.url} label={`Copy the link to #${issue.summary.number}`} />
            <OpenOnGithub url={issue.summary.url} what={`#${issue.summary.number}`} />
          </span>
        </div>
        <h2 className="mt-2.5 font-display text-[19px] font-semibold leading-snug tracking-[-0.015em] text-foreground">{issue.summary.title}</h2>
        <p className="mt-1.5 flex items-center gap-1.5 text-caption text-muted-foreground"><Monogram login={issue.summary.author?.login} className="size-4 text-micro" />Opened by {issue.summary.author?.login ?? "ghost"}</p>
        <div className="mt-2 flex flex-wrap items-center gap-1.5">{issue.summary.labels.map(label => <LabelBadge key={label.name} label={label} />)}<button type="button" onClick={() => setLabelsOpen(value => !value)} className="inline-flex min-h-7 items-center gap-1 rounded-md px-2 py-1 text-caption text-muted-foreground hover:bg-accent hover:text-foreground"><Tag size={10.5} aria-hidden="true" /> Manage labels</button></div>
        {labelsOpen && <LabelControls available={availableLabels} current={issue.summary.labels} disabled={busy} onChange={(label, operation) => setPending({
          title: `${operation === "add" ? "Add" : "Remove"} label “${label}”`,
          consequence: `The issue is edited on ${repoLabel}.`,
          command: `gh issue edit ${number} -R ${repoLabel} ${operation === "add" ? "--add-label" : "--remove-label"} ${JSON.stringify(label)}`,
          tone: "neutral",
          icon: Tag,
          requiresBody: false,
          confirmLabel: operation === "add" ? "Add label" : "Remove label",
          build: () => ({ kind: "label", target: "issue", number, label, operation }),
        })} />}
        <div className="mt-3 flex flex-wrap items-center gap-1.5">
          <button type="button" disabled={busy} className={ACTION_BUTTON} onClick={() => setPending({
            title: open ? `Close #${number}` : `Reopen #${number}`,
            consequence: open
              ? "Comments stay, and you can reopen it from this pane."
              : "The issue is open again for comments and triage.",
            command: open ? `gh issue close ${number} -R ${repoLabel}` : `gh issue reopen ${number} -R ${repoLabel}`,
            tone: open ? "destructive" : "neutral",
            icon: open ? Ban : RotateCcw,
            requiresBody: false,
            confirmLabel: open ? "Close issue" : "Reopen issue",
            destructive: open,
            build: () => ({ kind: "setState", target: "issue", number, operation: open ? "close" : "reopen" }),
          })}>
            {open ? <><Ban size={11.5} aria-hidden="true" /> Close</> : <><RotateCcw size={11.5} aria-hidden="true" /> Reopen</>}
          </button>
        </div>
      </div>
      {actionError && <p role="alert" className="border-b border-border bg-destructive/10 px-4 py-2 text-meta text-destructive">{actionError}</p>}
      <div className="space-y-6 px-5 py-5">
        <section aria-label="Issue description"><SectionLabel>Description</SectionLabel>{issue.body ? <GithubMarkdown text={issue.body} repositoryUrl={repositoryUrl} /> : <p className="text-meta italic text-muted-foreground">No description provided.</p>}</section>
        <section aria-label="Issue comments">
          <SectionLabel count={issue.comments.length}>Comments</SectionLabel>
          {issue.comments.length ? <CommentList comments={issue.comments} repositoryUrl={repositoryUrl} /> : <p className="text-meta text-muted-foreground">No comments.</p>}
          <div className="mt-2.5">
            <Composer
              label="New comment"
              placeholder="Write a comment…"
              submitLabel="Comment"
              disabled={busy}
              onSubmit={async body => {
                try {
                  const outcome = await bridgeApi.githubAct(workspaceId, { kind: "comment", target: "issue", number, body }, true);
                  if (!outcome.executed) return outcome.message;
                  onActed();
                  return undefined;
                } catch (value) {
                  return errorMessage(value);
                }
              }}
            />
          </div>
        </section>
      </div>
    </div>
    <ActionSheetHost>
      {pending && <ActionSheet
        icon={pending.icon}
        tone={pending.tone}
        title={pending.title}
        consequence={pending.consequence}
        command={pending.command}
        requiresBody={pending.requiresBody}
        body={pending.body ?? ""}
        onBody={() => undefined}
        readOnlyBody={pending.body !== undefined}
        confirmLabel={pending.confirmLabel}
        destructive={pending.destructive}
        busy={busy}
        onCancel={() => setPending(undefined)}
        onConfirm={() => void submit(pending.build(pending.body ?? ""))}
      />}
    </ActionSheetHost>
  </div>;
}

/** Connect a workspace folder to a GitHub repository.
 *
 * The pane reaches this when `gh` is healthy but the folder resolves to no
 * repository — a setup step, which is why it is a dialog with an action rather
 * than an error notice. Search covers the common case (a repository the user
 * already owns); the URL field covers the rest, and is the same validation the
 * clone flow uses. */
function ConnectRepositoryDialog({ workspaceId, onCancel, onConnected }: {
  workspaceId: string;
  onCancel: () => void;
  onConnected: () => void;
}) {
  const [query, setQuery] = useState("");
  const [candidates, setCandidates] = useState<GithubRepoCandidate[]>();
  const [searchError, setSearchError] = useState<string>();
  const [searching, setSearching] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const alive = useRef(true);
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);

  // A pasted URL is already an answer — searching for it would only stall the
  // one interaction that needs no network round trip.
  const pastedUrl = /^(https?:\/\/github\.com\/|git@github\.com:|ssh:\/\/git@github\.com\/)/.test(query.trim())
    ? query.trim()
    : undefined;

  useEffect(() => {
    const needle = query.trim();
    if (pastedUrl || needle.length < 2) { setCandidates(undefined); setSearchError(undefined); return; }
    let cancelled = false;
    setSearching(true);
    const timer = setTimeout(() => {
      void bridgeApi.searchGithubRepos(needle)
        .then(result => { if (!cancelled && alive.current) { setCandidates(result.repositories); setSearchError(undefined); } })
        // A failed search is not an empty one. Reporting "no matches" for an
        // unreachable `gh` sends the user hunting for a repository that exists.
        .catch(value => { if (!cancelled && alive.current) { setCandidates(undefined); setSearchError(errorMessage(value)); } })
        .finally(() => { if (!cancelled && alive.current) setSearching(false); });
    }, 250);
    return () => { cancelled = true; clearTimeout(timer); setSearching(false); };
  }, [query, pastedUrl]);

  async function connect(remoteUrl: string) {
    setBusy(true);
    setError(undefined);
    try {
      await bridgeApi.githubConnect(workspaceId, remoteUrl);
      if (alive.current) onConnected();
    } catch (value) {
      if (alive.current) setError(errorMessage(value));
    } finally {
      if (alive.current) setBusy(false);
    }
  }

  return <Dialog open onOpenChange={next => { if (!next && !busy) onCancel(); }}>
    <DialogPopup showCloseButton={false} aria-label="Connect a repository" className="max-w-md p-5">
      <h2 className="font-display text-body font-semibold text-foreground">Connect a repository</h2>
      <p className="mt-1 text-meta leading-relaxed text-muted-foreground">Search your GitHub repositories, or paste a repository URL. Bridge points this folder&apos;s <code className="font-mono text-foreground/90">origin</code> at it.</p>
      <input
        autoFocus
        value={query}
        onChange={event => { setQuery(event.target.value); setError(undefined); }}
        onKeyDown={event => { if (event.key === "Enter" && pastedUrl && !busy) { event.preventDefault(); void connect(pastedUrl); } }}
        disabled={busy}
        aria-label="Repository search or URL"
        placeholder="owner/name, or https://github.com/owner/name"
        className="mt-4 h-9 w-full rounded-lg border border-input bg-card px-3 text-ui text-foreground outline-none placeholder:text-muted-foreground focus-visible:border-ring disabled:opacity-50"
      />
      {!pastedUrl && candidates !== undefined && <ul className="mt-2 max-h-56 overflow-y-auto rounded-lg border border-border" aria-label="Matching repositories">
        {candidates.length === 0
          ? searching
            ? <li><SkeletonRows label="Searching repositories" rows={3} meta={false} framed={false} /></li>
            : <li className="px-3 py-2.5 text-meta text-muted-foreground">No repository matches that. Paste its URL instead.</li>
          : candidates.map(candidate => <li key={candidate.nameWithOwner}>
            <button type="button" disabled={busy} onClick={() => void connect(candidate.url)} className="flex w-full items-center gap-2 px-3 py-2 text-left transition-colors hover:bg-accent disabled:opacity-50">
              <FolderGit2 size={13} strokeWidth={1.7} className="shrink-0 text-muted-foreground" aria-hidden="true" />
              <span className="min-w-0 flex-1 truncate font-mono text-meta text-foreground">{candidate.nameWithOwner}</span>
              {candidate.isPrivate && <Badge variant="outline" className="shrink-0 text-micro">Private</Badge>}
            </button>
          </li>)}
      </ul>}
      {searchError && !pastedUrl && <p role="alert" className="mt-2 text-meta text-destructive">Search failed: {searchError} You can still paste the repository URL.</p>}
      {error && <p role="alert" className="mt-3 text-meta text-destructive">{error}</p>}
      <div className="mt-4 flex justify-end gap-2">
        <button type="button" onClick={onCancel} disabled={busy} className="min-h-8 rounded-lg px-3 text-ui font-medium hover:bg-accent disabled:opacity-50">Cancel</button>
        <button type="button" onClick={() => pastedUrl && void connect(pastedUrl)} disabled={busy || !pastedUrl} className="min-h-8 rounded-lg bg-primary px-3 text-ui font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50">Connect</button>
      </div>
    </DialogPopup>
  </Dialog>;
}
