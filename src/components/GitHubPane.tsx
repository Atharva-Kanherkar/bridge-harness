import { useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  CircleCheck,
  CircleDashed,
  CircleDot,
  CircleSlash,
  CircleX,
  ExternalLink,
  FolderGit2,
  GitMerge,
  GitPullRequest,
  GitPullRequestDraft,
  LoaderCircle,
  MessageSquare,
  RefreshCw,
  SquareArrowOutUpRight,
} from "lucide-react";
import { bridgeApi } from "../api";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { rollupState, type PullRequestListItem, type RollupState } from "../githubSurface";
import type {
  GithubAction,
  GithubCheckoutResult,
  GithubChecksResult,
  GithubMergeConfigResult,
  GithubPullRequestResult,
  GithubRepository,
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
  /** An outside ask (sidebar row, CI toast) to open one PR. Nonce distinguishes
   * "open it again" from a re-render. */
  intent?: { number: number; nonce: number };
  onJumpToFile: (path: string, line: number | undefined, headBranch: string) => void;
};

type Detail = { result: GithubPullRequestResult; checks: GithubChecksResult };

const ROLLUP: Record<RollupState, { icon: typeof CircleCheck; className: string; label: string }> = {
  failing: { icon: CircleX, className: "text-destructive", label: "Failing" },
  running: { icon: LoaderCircle, className: "animate-spin text-warning", label: "Running" },
  passing: { icon: CircleCheck, className: "text-success", label: "Passing" },
  none: { icon: CircleDashed, className: "text-muted-foreground/60", label: "No checks" },
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
    case "skipped": case "cancelled": case "neutral": return { icon: CircleSlash, className: "text-muted-foreground/60" };
    default: return { icon: CircleDashed, className: "text-muted-foreground/60" };
  }
}

/** Centered empty/availability state — the pane never renders blank. */
function PaneNotice({ icon: Icon, title, children }: { icon: typeof GitPullRequest; title: string; children?: React.ReactNode }) {
  return <div className="flex h-full flex-col items-center justify-center gap-2 px-8 text-center">
    <span className="grid size-10 place-items-center rounded-xl border border-border bg-card text-muted-foreground"><Icon size={18} strokeWidth={1.6} aria-hidden="true" /></span>
    <p className="mt-1 text-[13px] font-medium text-foreground">{title}</p>
    <div className="max-w-[26rem] text-[12px] leading-relaxed text-muted-foreground">{children}</div>
  </div>;
}

export function GitHubPane({ workspaceId, workspaceBranch, intent, onJumpToFile }: GitHubPaneProps) {
  const [status, setStatus] = useState<GithubStatusResult>();
  const [prs, setPrs] = useState<PullRequestListItem[]>();
  const [listError, setListError] = useState<string>();
  const [refreshing, setRefreshing] = useState(false);

  const [selected, setSelected] = useState<number>();
  const [detail, setDetail] = useState<Detail>();
  const [detailError, setDetailError] = useState<string>();

  const alive = useRef(true);
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);

  const loadList = useCallback(async () => {
    setRefreshing(true);
    try {
      const next = await bridgeApi.githubStatus(workspaceId);
      if (!alive.current) return;
      setStatus(next);
      setListError(undefined);
      if (next.availability.status === "available") {
        const list = await bridgeApi.githubPullRequests(workspaceId);
        if (alive.current) setPrs(list.pullRequests);
      }
    } catch (error) {
      if (alive.current) setListError(error instanceof Error ? error.message : String(error));
    } finally {
      if (alive.current) setRefreshing(false);
    }
  }, [workspaceId]);

  const openDetail = useCallback(async (number: number) => {
    setSelected(number);
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

  useEffect(() => {
    setStatus(undefined); setPrs(undefined); setListError(undefined);
    setSelected(undefined); setDetail(undefined); setDetailError(undefined);
    void loadList();
  }, [loadList]);

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
      void bridgeApi.githubPullRequests(workspaceId).then(value => { if (active) setPrs(value.pullRequests); }).catch(() => undefined);
      if (selectedRef.current === payload.number) void openDetail(payload.number);
    };
    void bridgeApi.onGithubChecksChanged(refetch).then(off => { if (active) offs.push(off); else off(); });
    void bridgeApi.onGithubCiFinished(refetch).then(off => { if (active) offs.push(off); else off(); });
    return () => { active = false; offs.forEach(off => off()); };
  }, [workspaceId, openDetail]);

  // Deep links (sidebar row, CI toast) land here.
  const seenIntent = useRef(0);
  useEffect(() => {
    if (!intent || intent.nonce === seenIntent.current) return;
    seenIntent.current = intent.nonce;
    setDetail(undefined);
    void openDetail(intent.number);
  }, [intent, openDetail]);

  const repoLabel = status?.repository ? `${status.repository.owner}/${status.repository.name}` : undefined;

  let body: React.ReactNode;
  if (listError) {
    body = <PaneNotice icon={CircleX} title="GitHub is unreachable">{listError}</PaneNotice>;
  } else if (!status) {
    body = <PaneNotice icon={LoaderCircle} title="Checking GitHub…"><span className="inline-flex items-center gap-1.5"><LoaderCircle size={12} className="animate-spin" aria-hidden="true" /> Resolving the CLI and repository.</span></PaneNotice>;
  } else if (status.availability.status === "notInstalled") {
    body = <PaneNotice icon={CircleSlash} title="GitHub CLI is not installed">Bridge drives GitHub through <code className="font-mono text-foreground/90">gh</code> — install it and sign in, and this pane fills in by itself.</PaneNotice>;
  } else if (status.availability.status === "notAuthenticated") {
    body = <PaneNotice icon={CircleDot} title="Sign in to GitHub">Run <code className="rounded-md border border-border bg-card px-1.5 py-0.5 font-mono text-[11.5px] text-foreground/90">{status.availability.remediation}</code> in a terminal, then refresh.</PaneNotice>;
  } else if (selected !== undefined) {
    body = <PullRequestDetail
      workspaceId={workspaceId}
      workspaceBranch={workspaceBranch}
      repository={status.repository}
      number={selected}
      detail={detail}
      error={detailError}
      onBack={() => { setSelected(undefined); setDetail(undefined); setDetailError(undefined); void loadList(); }}
      onActed={() => { void loadList(); void openDetail(selected); }}
      onJumpToFile={onJumpToFile}
    />;
  } else if (!prs) {
    body = <PaneNotice icon={LoaderCircle} title="Loading pull requests…" />;
  } else if (!prs.length) {
    body = <PaneNotice icon={GitPullRequest} title="No open pull requests">{repoLabel ? `${repoLabel} has nothing waiting on you.` : "This repository has nothing waiting on you."}</PaneNotice>;
  } else {
    body = <div className="min-h-0 flex-1 overflow-y-auto px-3 py-3 sm:px-4">
      <div className="divide-y divide-border overflow-hidden rounded-lg border border-border bg-card">
        {prs.map(pr => <PullRequestRow key={pr.number} pr={pr} onOpen={() => void openDetail(pr.number)} />)}
      </div>
    </div>;
  }

  return <section className="relative flex h-full w-full flex-col" aria-label="GitHub pull requests">
    <header className="flex h-10 shrink-0 items-center gap-2 border-b border-border px-3.5">
      <GitPullRequest size={14} className="shrink-0 text-muted-foreground" aria-hidden="true" />
      <span className="truncate text-[12px] font-medium text-foreground">Pull requests</span>
      {repoLabel && <span className="truncate font-mono text-[11px] text-muted-foreground">{repoLabel}</span>}
      <button
        type="button"
        onClick={() => { void loadList(); if (selected !== undefined) void openDetail(selected); }}
        aria-label="Refresh pull requests"
        title="Refresh"
        className="ml-auto grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
      >
        <RefreshCw size={12.5} className={cn(refreshing && "animate-spin")} aria-hidden="true" />
      </button>
    </header>
    {body}
  </section>;
}

function PullRequestRow({ pr, onOpen }: { pr: PullRequestListItem; onOpen: () => void }) {
  const rollup = ROLLUP[rollupState(pr)];
  const RollupIcon = rollup.icon;
  const review = REVIEW[pr.reviewDecision];
  const StateIcon = pr.isDraft ? GitPullRequestDraft : pr.state === "merged" ? GitMerge : GitPullRequest;
  return <button type="button" onClick={onOpen} className="group flex w-full items-start gap-2.5 px-3 py-2.5 text-left transition-colors hover:bg-accent/50">
    <StateIcon size={15} strokeWidth={1.8} aria-hidden="true" className={cn("mt-0.5 shrink-0", pr.isDraft ? "text-muted-foreground/70" : pr.state === "merged" ? "text-info" : "text-success")} />
    <span className="min-w-0 flex-1">
      <span className="block truncate text-[12.5px] font-medium leading-5 text-foreground">{pr.title}</span>
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
      {pr.checks.total > 0 && <span className={cn("inline-flex items-center gap-1 font-mono text-[10.5px] tabular-nums", rollup.className)} title={`${rollup.label} — ${pr.checks.passed}/${pr.checks.total} checks passed`}>
        <RollupIcon size={12} aria-hidden="true" />
        {pr.checks.passed}/{pr.checks.total}
      </span>}
    </span>
  </button>;
}

// ── Detail ───────────────────────────────────────────────────────────────────

type PendingAction = { statement: string; requiresBody: boolean; build: (body: string) => GithubAction };

type PullRequestDetailProps = {
  workspaceId: string;
  workspaceBranch: string | null;
  repository?: GithubRepository | null;
  number: number;
  detail?: Detail;
  error?: string;
  onBack: () => void;
  onActed: () => void;
  onJumpToFile: (path: string, line: number | undefined, headBranch: string) => void;
};

function PullRequestDetail({ workspaceId, workspaceBranch, repository, number, detail, error, onBack, onActed, onJumpToFile }: PullRequestDetailProps) {
  const [pending, setPending] = useState<PendingAction>();
  const [pendingBody, setPendingBody] = useState("");
  const [mergeOpen, setMergeOpen] = useState(false);
  const [mergeConfig, setMergeConfig] = useState<GithubMergeConfigResult>();
  const [strategy, setStrategy] = useState<MergeStrategy>();
  const [checkoutOpen, setCheckoutOpen] = useState(false);
  const [checkout, setCheckout] = useState<GithubCheckoutResult>();
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string>();

  const repoLabel = repository ? `${repository.owner}/${repository.name}` : "this repository";
  const closeOverlays = () => { setPending(undefined); setPendingBody(""); setMergeOpen(false); setMergeConfig(undefined); setStrategy(undefined); setCheckoutOpen(false); };

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
    <button type="button" onClick={onBack} className="inline-flex items-center gap-1.5 rounded-md px-1.5 py-1 text-[11.5px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">
      <ArrowLeft size={12.5} aria-hidden="true" /> All pull requests
    </button>
  </div>;

  if (error) return <div className="flex min-h-0 flex-1 flex-col">{header}<PaneNotice icon={CircleX} title={`Pull request #${number} did not load`}>{error}</PaneNotice></div>;
  if (!detail) return <div className="flex min-h-0 flex-1 flex-col">{header}<PaneNotice icon={LoaderCircle} title={`Opening #${number}…`} /></div>;

  const { pullRequest, reviewThreads } = detail.result;
  const summary = pullRequest.summary;
  const review = REVIEW[summary.reviewDecision];
  const rerunnable = detail.checks.checks.some(check =>
    check.conclusion === "failure" || check.conclusion === "timedOut" || check.conclusion === "startupFailure"
  );
  const checkedOutHere = workspaceBranch === summary.headBranch;
  const StateIcon = summary.isDraft ? GitPullRequestDraft : summary.state === "merged" ? GitMerge : GitPullRequest;

  const actionButton = "rounded-md border border-border bg-card px-2.5 py-1 text-[11.5px] font-medium text-foreground transition-colors hover:bg-accent disabled:opacity-50";

  return <div className="relative flex min-h-0 flex-1 flex-col">
    {header}
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="border-b border-border px-4 pb-3 pt-3.5">
        <div className="flex items-center gap-2 text-[11px] text-muted-foreground">
          <StateIcon size={13} aria-hidden="true" className={cn(summary.isDraft ? "text-muted-foreground/70" : summary.state === "merged" ? "text-info" : summary.state === "closed" ? "text-destructive" : "text-success")} />
          <span className="tabular-nums">#{summary.number}</span>
          <span className="capitalize">{summary.isDraft ? "draft" : summary.state}</span>
          {review && <Badge variant={review.variant} size="sm">{review.label}</Badge>}
          <a href={summary.url} target="_blank" rel="noreferrer" aria-label={`Open #${summary.number} on GitHub`} title="Open on GitHub" className="ml-auto text-muted-foreground transition-colors hover:text-foreground"><SquareArrowOutUpRight size={12.5} aria-hidden="true" /></a>
        </div>
        <h2 className="mt-1.5 font-display text-[15px] font-semibold leading-snug tracking-[-0.01em] text-foreground">{summary.title}</h2>
        <p className="mt-1 font-mono text-[11px] text-muted-foreground">
          {summary.headBranch} <span aria-hidden="true">→</span> {pullRequest.baseBranch}
          {checkedOutHere && <span className="ml-1.5 text-success">· checked out here</span>}
        </p>

        {summary.state === "open" && <div className="mt-3 flex flex-wrap items-center gap-1.5">
          <button type="button" onClick={() => void openMerge()} disabled={busy} className="rounded-md border border-success/25 bg-success/10 px-2.5 py-1 text-[11.5px] font-medium text-success transition-colors hover:bg-success/20 disabled:opacity-50">Merge</button>
          <button type="button" disabled={busy} className={actionButton} onClick={() => setPending({ statement: `submit approval on PR #${number} on ${repoLabel}`, requiresBody: false, build: () => ({ kind: "review", number, event: "approve", body: "" }) })}>Approve</button>
          <button type="button" disabled={busy} className={actionButton} onClick={() => setPending({ statement: `submit requested changes on PR #${number} on ${repoLabel}`, requiresBody: true, build: body => ({ kind: "review", number, event: "requestChanges", body }) })}>Request changes</button>
          {rerunnable && <button type="button" disabled={busy} className={actionButton} onClick={() => setPending({ statement: `re-run failed checks on PR #${number} on ${repoLabel}`, requiresBody: false, build: () => ({ kind: "rerun", number }) })}>Re-run failed</button>}
          {!checkedOutHere && <button type="button" disabled={busy} className={actionButton} onClick={() => { setActionError(undefined); setCheckoutOpen(true); }}>
            <span className="inline-flex items-center gap-1.5"><FolderGit2 size={12} aria-hidden="true" /> Check out</span>
          </button>}
        </div>}
      </div>

      {actionError && <p role="alert" className="border-b border-border bg-destructive/10 px-4 py-2 text-[11.5px] text-destructive">{actionError}</p>}
      {checkout && <p className="border-b border-border bg-success/10 px-4 py-2 text-[11.5px] text-success">
        {checkout.reused ? "Reusing the task worktree already on " : "Checked out into a task worktree on "}
        <span className="font-mono">{checkout.branch}</span>
        <span className="block truncate font-mono text-[10.5px] text-success/80" title={checkout.path}>{checkout.path}</span>
      </p>}

      <div className="space-y-5 px-4 py-4">
        <section aria-label="Checks">
          <h3 className="mb-2 text-[10.5px] font-semibold tracking-[0.1em] text-muted-foreground/65">CHECKS</h3>
          {detail.checks.checks.length ? <div className="divide-y divide-border overflow-hidden rounded-lg border border-border bg-card">
            {detail.checks.checks.map(check => {
              const tone = checkTone(check.conclusion, check.status);
              const ToneIcon = tone.icon;
              return <div key={`${check.workflow}-${check.name}`} className="flex items-center gap-2.5 px-3 py-2">
                <ToneIcon size={13.5} className={cn("shrink-0", tone.className)} aria-hidden="true" />
                <span className="min-w-0 flex-1 truncate text-[12px] text-foreground">{check.name}</span>
                <span className="shrink-0 text-[10.5px] text-muted-foreground">{check.conclusion ?? check.status}</span>
                {check.logUrl && <a href={check.logUrl} target="_blank" rel="noreferrer" aria-label={`Open logs for ${check.name}`} title="Open logs" className="shrink-0 text-muted-foreground transition-colors hover:text-foreground"><ExternalLink size={12.5} aria-hidden="true" /></a>}
              </div>;
            })}
          </div> : <p className="text-[12px] text-muted-foreground">No checks reported.</p>}
        </section>

        <section aria-label="Description">
          <h3 className="mb-2 text-[10.5px] font-semibold tracking-[0.1em] text-muted-foreground/65">DESCRIPTION</h3>
          <p className="whitespace-pre-wrap text-[12.5px] leading-relaxed text-foreground/90">{pullRequest.body || "No description provided."}</p>
        </section>

        <section aria-label="Review threads">
          <h3 className="mb-2 text-[10.5px] font-semibold tracking-[0.1em] text-muted-foreground/65">REVIEW THREADS</h3>
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
                    <p className="flex items-baseline gap-1.5 text-[11px]">
                      <span className="font-medium text-foreground">{comment.author?.login ?? "ghost"}</span>
                    </p>
                    <p className="mt-0.5 whitespace-pre-wrap text-[12px] leading-relaxed text-foreground/90">{comment.body}</p>
                  </div>)}
                  {rootId !== null && <button
                    type="button"
                    disabled={busy}
                    onClick={() => setPending({ statement: `reply to a review comment on PR #${number} on ${repoLabel}`, requiresBody: true, build: body => ({ kind: "reply", number, commentId: rootId, body }) })}
                    className="inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-[11px] font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-50"
                  >
                    <MessageSquare size={11} aria-hidden="true" /> Reply
                  </button>}
                </div>
              </article>;
            })}
          </div> : <p className="text-[12px] text-muted-foreground">No review threads.</p>}
        </section>
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

    {mergeOpen && <div className="absolute inset-0 z-10 flex items-center justify-center bg-background/70 p-4" role="dialog" aria-label="Confirm merge">
      <div className="u-glass-popover w-full max-w-sm rounded-xl border border-border p-4 shadow-2xl">
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
            <button type="button" onClick={closeOverlays} disabled={busy} className="rounded-md px-3 py-1 text-xs font-medium hover:bg-accent disabled:opacity-50">Cancel</button>
            <button type="button" onClick={() => strategy && void submit({ kind: "merge", number, strategy })} disabled={busy || !strategy} className="rounded-md bg-success/20 px-3 py-1 text-xs font-medium text-success hover:bg-success/30 disabled:opacity-50">Merge</button>
          </div>
        </>}
      </div>
    </div>}
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
  return <div className="absolute inset-0 z-10 flex items-center justify-center bg-background/70 p-4" role="dialog" aria-label="Confirm action">
    <div className="u-glass-popover w-full max-w-sm rounded-xl border border-border p-4 shadow-2xl">
      <p className="text-xs text-muted-foreground">This runs</p>
      <p className="mt-1 font-mono text-xs text-foreground/90">{statement}</p>
      {note && <p className="mt-2 text-[11px] leading-relaxed text-muted-foreground">{note}</p>}
      {requiresBody && <textarea value={body} onChange={event => onBody(event.target.value)} aria-label="Comment body" placeholder="Write a comment…" className="mt-3 h-20 w-full resize-none rounded-md border border-border bg-transparent p-2 text-xs" />}
      <div className="mt-4 flex justify-end gap-2">
        <button type="button" onClick={onCancel} disabled={busy} className="rounded-md px-3 py-1 text-xs font-medium hover:bg-accent disabled:opacity-50">Cancel</button>
        <button type="button" onClick={onConfirm} disabled={busy || !ready} className="rounded-md bg-primary/20 px-3 py-1 text-xs font-medium text-primary hover:bg-primary/30 disabled:opacity-50">{confirmLabel}</button>
      </div>
    </div>
  </div>;
}
