import { ExternalLink, X } from "lucide-react";
import type { GithubChecksResult, GithubPullRequestResult } from "../protocol/generated/protocol";

type PullRequestViewProps = {
  result: GithubPullRequestResult;
  checks: GithubChecksResult;
  onClose: () => void;
};

const checkTone = (conclusion: string | null | undefined) => conclusion === "success" ? "text-success" : conclusion === "failure" ? "text-destructive" : "text-warning";

/** Remote GitHub content is deliberately rendered through React text nodes —
 * never a markdown HTML renderer or dangerouslySetInnerHTML. */
export function PullRequestView({ result, checks, onClose }: PullRequestViewProps) {
  const { pullRequest, reviewThreads } = result;
  return <section className="u-glass-popover fixed inset-y-3 right-3 z-50 flex w-[min(42rem,calc(100vw-1.5rem))] flex-col overflow-hidden rounded-xl border border-border shadow-2xl" aria-label={`Pull request #${pullRequest.summary.number}`}>
    <header className="flex items-start gap-3 border-b border-border px-4 py-3">
      <div className="min-w-0 flex-1"><p className="text-xs text-muted-foreground">Pull request #{pullRequest.summary.number}</p><h2 className="truncate font-display text-base font-semibold">{pullRequest.summary.title}</h2></div>
      <button type="button" onClick={onClose} aria-label="Close pull request" className="rounded-md p-1 text-muted-foreground hover:bg-accent hover:text-foreground"><X size={16} /></button>
    </header>
    <div className="min-h-0 flex-1 space-y-5 overflow-y-auto p-4 text-sm">
      <section><h3 className="mb-1 text-xs font-semibold uppercase tracking-wide text-muted-foreground">Description</h3><p className="whitespace-pre-wrap leading-relaxed text-foreground/90">{pullRequest.body || "No description provided."}</p></section>
      <section><h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">Checks</h3>{checks.checks.length ? <ul className="space-y-1.5">{checks.checks.map(check => <li key={`${check.workflow}-${check.name}`} className="u-glass-soft flex items-center gap-2 rounded-lg px-3 py-2"><span className={`font-medium ${checkTone(check.conclusion)}`}>{check.conclusion ?? check.status}</span><span className="min-w-0 flex-1 truncate">{check.name}</span>{check.logUrl && <a href={check.logUrl} target="_blank" rel="noreferrer" aria-label={`Open logs for ${check.name}`} className="text-muted-foreground hover:text-foreground"><ExternalLink size={14} /></a>}</li>)}</ul> : <p className="text-muted-foreground">No checks reported.</p>}</section>
      <section><h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">Review threads</h3>{reviewThreads.length ? <div className="space-y-2">{reviewThreads.map(thread => <article key={thread.id} className="u-glass-soft rounded-lg p-3"><p className="mb-2 text-xs text-muted-foreground">{thread.path}{thread.line ? `:${thread.line}` : ""}{thread.isResolved ? " · Resolved" : ""}</p>{thread.comments.map(comment => <div key={comment.id} className="border-t border-border/70 pt-2 first:border-t-0 first:pt-0"><p className="text-xs font-medium">{comment.author?.login ?? "Ghost"}</p><p className="whitespace-pre-wrap text-foreground/90">{comment.body}</p></div>)}</article>)}</div> : <p className="text-muted-foreground">No review threads.</p>}</section>
    </div>
  </section>;
}
