import { ExternalLink, LoaderCircle, X } from "lucide-react";
import { useState } from "react";
import { bridgeApi } from "../api";
import type { GithubAction, GithubChecksResult, GithubMergeConfigResult, GithubPullRequestResult, GithubRepository, MergeStrategy } from "../protocol/generated/protocol";

type PullRequestViewProps = {
  workspaceId: string;
  repository?: GithubRepository | null;
  result: GithubPullRequestResult;
  checks: GithubChecksResult;
  onActed: () => void;
  onClose: () => void;
};

/** A pending non-merge action awaiting its native confirmation. `build` turns
 * the (optional) composed body into the exact action sent to `github/act`. */
type Pending = { statement: string; requiresBody: boolean; build: (body: string) => GithubAction };

const checkTone = (conclusion: string | null | undefined) => conclusion === "success" ? "text-success" : conclusion === "failure" ? "text-destructive" : "text-warning";
const STRATEGY_LABEL: Record<MergeStrategy, string> = { merge: "Merge commit", squash: "Squash and merge", rebase: "Rebase and merge" };

/** Remote GitHub content is deliberately rendered through React text nodes —
 * never a markdown HTML renderer or dangerouslySetInnerHTML. Every mutation is
 * gated by a native confirmation naming the exact operation before `github/act`
 * runs; cancelling spawns nothing. */
export function PullRequestView({ workspaceId, repository, result, checks, onActed, onClose }: PullRequestViewProps) {
  const { pullRequest, reviewThreads } = result;
  const number = pullRequest.summary.number;
  const repoLabel = repository ? `${repository.owner}/${repository.name}` : "this repository";

  const [pending, setPending] = useState<Pending>();
  const [pendingBody, setPendingBody] = useState("");
  const [mergeOpen, setMergeOpen] = useState(false);
  const [mergeConfig, setMergeConfig] = useState<GithubMergeConfigResult>();
  const [strategy, setStrategy] = useState<MergeStrategy>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();

  const closeOverlays = () => { setPending(undefined); setPendingBody(""); setMergeOpen(false); setMergeConfig(undefined); setStrategy(undefined); };

  // The only path that reaches `github/act`. A refused `gh` write surfaces its
  // message verbatim and leaves the PR state untouched — no optimistic edit.
  const submit = async (action: GithubAction) => {
    setBusy(true); setError(undefined);
    try {
      const outcome = await bridgeApi.githubAct(workspaceId, action, true);
      closeOverlays();
      if (!outcome.executed) { setError(outcome.message); return; }
      onActed();
    } catch (value) {
      setError(value instanceof Error ? value.message : String(value));
    } finally { setBusy(false); }
  };

  const openMerge = async () => {
    setError(undefined); setMergeOpen(true); setMergeConfig(undefined); setStrategy(undefined);
    try {
      const config = await bridgeApi.githubMergeConfig(workspaceId);
      setMergeConfig(config); setStrategy(config.defaultStrategy);
    } catch (value) {
      setMergeOpen(false);
      setError(value instanceof Error ? value.message : String(value));
    }
  };

  const allowed = (config: GithubMergeConfigResult): MergeStrategy[] =>
    (["merge", "squash", "rebase"] as const).filter(name => config.strategies[name]);
  const rerunnable = checks.checks.some(check =>
    check.conclusion === "failure" || check.conclusion === "timedOut" || check.conclusion === "startupFailure"
  );

  return <section className="u-glass-popover fixed inset-y-3 right-3 z-50 flex w-[min(42rem,calc(100vw-1.5rem))] flex-col overflow-hidden rounded-xl border border-border shadow-2xl" aria-label={`Pull request #${number}`}>
    <header className="flex items-start gap-3 border-b border-border px-4 py-3">
      <div className="min-w-0 flex-1"><p className="text-xs text-muted-foreground">Pull request #{number}</p><h2 className="truncate font-display text-base font-semibold">{pullRequest.summary.title}</h2></div>
      <button type="button" onClick={onClose} aria-label="Close pull request" className="rounded-md p-1 text-muted-foreground hover:bg-accent hover:text-foreground"><X size={16} /></button>
    </header>

    {pullRequest.summary.state === "open" && <div className="flex flex-wrap items-center gap-2 border-b border-border px-4 py-2">
      <button type="button" onClick={() => void openMerge()} disabled={busy} className="rounded-md bg-success/15 px-2.5 py-1 text-xs font-medium text-success hover:bg-success/25 disabled:opacity-50">Merge</button>
      <button type="button" onClick={() => setPending({ statement: `submit approval on PR #${number} on ${repoLabel}`, requiresBody: false, build: () => ({ kind: "review", number, event: "approve", body: "" }) })} disabled={busy} className="rounded-md bg-accent px-2.5 py-1 text-xs font-medium hover:bg-accent/70 disabled:opacity-50">Approve</button>
      <button type="button" onClick={() => setPending({ statement: `submit requested changes on PR #${number} on ${repoLabel}`, requiresBody: true, build: body => ({ kind: "review", number, event: "requestChanges", body }) })} disabled={busy} className="rounded-md bg-accent px-2.5 py-1 text-xs font-medium hover:bg-accent/70 disabled:opacity-50">Request changes</button>
      {rerunnable && <button type="button" onClick={() => setPending({ statement: `re-run failed checks on PR #${number} on ${repoLabel}`, requiresBody: false, build: () => ({ kind: "rerun", number }) })} disabled={busy} className="rounded-md bg-accent px-2.5 py-1 text-xs font-medium hover:bg-accent/70 disabled:opacity-50">Re-run failed checks</button>}
    </div>}

    {error && <p role="alert" className="border-b border-border bg-destructive/10 px-4 py-2 text-xs text-destructive">{error}</p>}

    <div className="min-h-0 flex-1 space-y-5 overflow-y-auto p-4 text-sm">
      <section><h3 className="mb-1 text-xs font-semibold uppercase tracking-wide text-muted-foreground">Description</h3><p className="whitespace-pre-wrap leading-relaxed text-foreground/90">{pullRequest.body || "No description provided."}</p></section>
      <section><h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">Checks</h3>{checks.checks.length ? <ul className="space-y-1.5">{checks.checks.map(check => <li key={`${check.workflow}-${check.name}`} className="u-glass-soft flex items-center gap-2 rounded-lg px-3 py-2"><span className={`font-medium ${checkTone(check.conclusion)}`}>{check.conclusion ?? check.status}</span><span className="min-w-0 flex-1 truncate">{check.name}</span>{check.logUrl && <a href={check.logUrl} target="_blank" rel="noreferrer" aria-label={`Open logs for ${check.name}`} className="text-muted-foreground hover:text-foreground"><ExternalLink size={14} /></a>}</li>)}</ul> : <p className="text-muted-foreground">No checks reported.</p>}</section>
      <section><h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">Review threads</h3>{reviewThreads.length ? <div className="space-y-2">{reviewThreads.map(thread => {
        const rootId = thread.comments[0]?.databaseId ?? null;
        return <article key={thread.id} className="u-glass-soft rounded-lg p-3"><p className="mb-2 text-xs text-muted-foreground">{thread.path}{thread.line ? `:${thread.line}` : ""}{thread.isResolved ? " · Resolved" : ""}</p>{thread.comments.map(comment => <div key={comment.id} className="border-t border-border/70 pt-2 first:border-t-0 first:pt-0"><p className="text-xs font-medium">{comment.author?.login ?? "Ghost"}</p><p className="whitespace-pre-wrap text-foreground/90">{comment.body}</p></div>)}{rootId !== null && <button type="button" onClick={() => setPending({ statement: `reply to a review comment on PR #${number} on ${repoLabel}`, requiresBody: true, build: body => ({ kind: "reply", number, commentId: rootId, body }) })} disabled={busy} className="mt-2 rounded-md bg-accent px-2 py-0.5 text-[11px] font-medium hover:bg-accent/70 disabled:opacity-50">Reply</button>}</article>;
      })}</div> : <p className="text-muted-foreground">No review threads.</p>}</section>
    </div>

    {pending && <ConfirmOverlay statement={pending.statement} requiresBody={pending.requiresBody} body={pendingBody} onBody={setPendingBody} busy={busy} onCancel={closeOverlays} onConfirm={() => void submit(pending.build(pendingBody))} />}

    {mergeOpen && <div className="absolute inset-0 z-10 flex items-center justify-center bg-background/70 p-4" role="dialog" aria-label="Confirm merge">
      <div className="u-glass-popover w-full max-w-sm rounded-xl border border-border p-4 shadow-2xl">
        <h3 className="mb-2 font-display text-sm font-semibold">Merge pull request</h3>
        {!mergeConfig ? <p className="flex items-center gap-1 text-xs text-muted-foreground"><LoaderCircle className="animate-spin" size={12} /> Reading repository settings…</p> : <>
          <fieldset className="space-y-1.5">{allowed(mergeConfig).map(name => <label key={name} className="flex items-center gap-2 text-xs"><input type="radio" name="merge-strategy" value={name} checked={strategy === name} onChange={() => setStrategy(name)} />{STRATEGY_LABEL[name]}</label>)}</fieldset>
          <p className="mt-3 text-xs text-muted-foreground">This runs <code className="font-mono text-foreground/90">merge PR #{number} ({strategy}) on {repoLabel}</code>.</p>
          <div className="mt-4 flex justify-end gap-2">
            <button type="button" onClick={closeOverlays} disabled={busy} className="rounded-md px-3 py-1 text-xs font-medium hover:bg-accent disabled:opacity-50">Cancel</button>
            <button type="button" onClick={() => strategy && void submit({ kind: "merge", number, strategy })} disabled={busy || !strategy} className="rounded-md bg-success/20 px-3 py-1 text-xs font-medium text-success hover:bg-success/30 disabled:opacity-50">Merge</button>
          </div>
        </>}
      </div>
    </div>}
  </section>;
}

type ConfirmOverlayProps = { statement: string; requiresBody: boolean; body: string; onBody: (value: string) => void; busy: boolean; onCancel: () => void; onConfirm: () => void };

function ConfirmOverlay({ statement, requiresBody, body, onBody, busy, onCancel, onConfirm }: ConfirmOverlayProps) {
  const ready = !requiresBody || body.trim().length > 0;
  return <div className="absolute inset-0 z-10 flex items-center justify-center bg-background/70 p-4" role="dialog" aria-label="Confirm action">
    <div className="u-glass-popover w-full max-w-sm rounded-xl border border-border p-4 shadow-2xl">
      <p className="text-xs text-muted-foreground">This runs</p>
      <p className="mt-1 font-mono text-xs text-foreground/90">{statement}</p>
      {requiresBody && <textarea value={body} onChange={event => onBody(event.target.value)} aria-label="Comment body" placeholder="Write a comment…" className="mt-3 h-20 w-full resize-none rounded-md border border-border bg-transparent p-2 text-xs" />}
      <div className="mt-4 flex justify-end gap-2">
        <button type="button" onClick={onCancel} disabled={busy} className="rounded-md px-3 py-1 text-xs font-medium hover:bg-accent disabled:opacity-50">Cancel</button>
        <button type="button" onClick={onConfirm} disabled={busy || !ready} className="rounded-md bg-primary/20 px-3 py-1 text-xs font-medium text-primary hover:bg-primary/30 disabled:opacity-50">Confirm</button>
      </div>
    </div>
  </div>;
}
