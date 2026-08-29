import { GitPullRequest, LoaderCircle } from "lucide-react";
import { useEffect, useState } from "react";
import { bridgeApi } from "../api";
import type { GithubChecksResult, GithubPullRequestResult, GithubPullRequestsResult, GithubStatusResult } from "../protocol/generated/protocol";
import { PullRequestView } from "./PullRequestView";

type SelectedPullRequest = { result: GithubPullRequestResult; checks: GithubChecksResult };

function rollup(pr: GithubPullRequestsResult["pullRequests"][number]) {
  if (pr.checks.failed) return "failing";
  if (pr.checks.queued || pr.checks.inProgress) return "running";
  if (pr.checks.total) return "passing";
  return "none";
}

export function GitHubPanel({ workspaceId }: { workspaceId?: string }) {
  const [status, setStatus] = useState<GithubStatusResult>();
  const [pullRequests, setPullRequests] = useState<GithubPullRequestsResult>();
  const [error, setError] = useState<string>();
  const [selected, setSelected] = useState<SelectedPullRequest>();

  useEffect(() => {
    let active = true;
    setStatus(undefined); setPullRequests(undefined); setError(undefined); setSelected(undefined);
    if (!workspaceId) return () => { active = false; };
    void bridgeApi.githubStatus(workspaceId).then(next => {
      if (!active) return;
      setStatus(next);
      if (next.availability.status === "available") return bridgeApi.githubPullRequests(workspaceId).then(value => { if (active) setPullRequests(value); });
    }).catch(value => { if (active) setError(value instanceof Error ? value.message : String(value)); });
    return () => { active = false; };
  }, [workspaceId]);

  const open = async (number: number) => {
    if (!workspaceId) return;
    const [result, checks] = await Promise.all([bridgeApi.githubPullRequest(workspaceId, number), bridgeApi.githubChecks(workspaceId, number)]);
    setSelected({ result, checks });
  };

  useEffect(() => {
    if (!workspaceId) return;
    let active = true;
    // The unsubscribe must be kept for cleanup, not only for the
    // already-cancelled race: this effect re-runs on every `selected` change,
    // and a listener that is never removed piles up a duplicate refetch per
    // selection — each with a stale `selected` closure.
    let off: (() => void) | undefined;
    void bridgeApi.onGithubChecksChanged(({ workspaceId: changedWorkspace, number }) => {
      if (!active || changedWorkspace !== workspaceId) return;
      void bridgeApi.githubPullRequests(workspaceId).then(value => { if (active) setPullRequests(value); });
      if (selected?.result.pullRequest.summary.number === number) void open(number);
    }).then(unlisten => {
      if (!active) { unlisten(); return; }
      off = unlisten;
    });
    return () => { active = false; off?.(); };
  }, [workspaceId, selected]);

  if (!workspaceId) return null;
  if (error) return <p className="mx-2 mb-2 text-xs text-muted-foreground">GitHub is unavailable.</p>;
  if (!status) return <p className="mx-2 mb-2 flex items-center gap-1 text-xs text-muted-foreground"><LoaderCircle className="animate-spin" size={12} /> Checking GitHub…</p>;
  if (status.availability.status === "notInstalled") return <p className="mx-2 mb-2 text-xs text-muted-foreground">GitHub CLI unavailable.</p>;
  if (status.availability.status === "notAuthenticated") return <p className="mx-2 mb-2 text-xs text-muted-foreground">Run <code className="font-mono">{status.availability.remediation}</code> to connect GitHub.</p>;
  if (!pullRequests?.pullRequests.length) return null;
  return <><section className="mb-2 rounded-lg border border-sidebar-border/70 px-1 py-1" aria-label="Open pull requests"><p className="px-1.5 py-1 text-[11px] font-semibold text-muted-foreground">PULL REQUESTS</p>{pullRequests.pullRequests.map(pr => <button type="button" key={pr.number} onClick={() => void open(pr.number)} className="flex w-full items-start gap-2 rounded-md px-1.5 py-1.5 text-left hover:bg-accent"><GitPullRequest size={14} className="mt-0.5 shrink-0 text-muted-foreground"/><span className="min-w-0 flex-1"><span className="block truncate text-xs">#{pr.number} {pr.title}</span><span className="block truncate text-[10px] text-muted-foreground">{pr.headBranch} · {pr.author?.login ?? "Ghost"} · {pr.reviewDecision}</span></span><span className={`mt-0.5 rounded px-1 font-mono text-[10px] ${rollup(pr) === "failing" ? "bg-destructive/15 text-destructive" : rollup(pr) === "passing" ? "bg-success/15 text-success" : "bg-accent text-muted-foreground"}`}>{rollup(pr)}</span></button>)}</section>{selected && <PullRequestView {...selected} onClose={() => setSelected(undefined)} />}</>;
}
