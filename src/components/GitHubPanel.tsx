import { useEffect, useState } from "react";
import { GitPullRequest, LoaderCircle } from "lucide-react";
import { bridgeApi } from "../api";
import { cn } from "@/lib/utils";
import { rollupState, type PullRequestListItem, type RollupState } from "../githubSurface";
import type { GithubStatusResult } from "../protocol/generated/protocol";

// The sidebar glance: "is CI green yet?" answered by a look at the rail. Rows
// are compact on purpose — a status dot, a number, a title. Everything richer
// (checks, threads, actions) lives in the GitHub dock pane, which `onOpen`
// deep-links into. The old in-sidebar detail overlay is gone: a `fixed`
// element inside the sidebar's transformed subtree could never escape it.

const DOT: Record<RollupState, string> = {
  failing: "bg-destructive",
  running: "bg-warning animate-pulse",
  passing: "bg-success",
  none: "bg-muted-foreground/40",
};

export function GitHubPanel({ workspaceId, onOpen }: { workspaceId?: string; onOpen?: (number: number) => void }) {
  const [status, setStatus] = useState<GithubStatusResult>();
  const [prs, setPrs] = useState<PullRequestListItem[]>();
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let active = true;
    setStatus(undefined); setPrs(undefined); setFailed(false);
    if (!workspaceId) return () => { active = false; };
    const load = () => bridgeApi.githubStatus(workspaceId).then(next => {
      if (!active) return;
      setStatus(next);
      if (next.availability.status === "available") {
        return bridgeApi.githubPullRequests(workspaceId).then(value => { if (active) setPrs(value.pullRequests); });
      }
    }).catch(() => { if (active) setFailed(true); });
    void load();
    const offs: Array<() => void> = [];
    const refetch = (payload: { workspaceId: string }) => { if (active && payload.workspaceId === workspaceId) void load(); };
    void bridgeApi.onGithubChecksChanged(refetch).then(off => { if (active) offs.push(off); else off(); });
    void bridgeApi.onGithubCiFinished(refetch).then(off => { if (active) offs.push(off); else off(); });
    return () => { active = false; offs.forEach(off => off()); };
  }, [workspaceId]);

  if (!workspaceId || failed) return null;
  if (!status) return <p className="mx-2 mb-2 flex items-center gap-1.5 px-1.5 text-[11px] text-muted-foreground"><LoaderCircle className="animate-spin" size={11} aria-hidden="true" /> Checking GitHub…</p>;
  if (status.availability.status === "notInstalled") return <p className="mx-2 mb-2 px-1.5 text-[11px] text-muted-foreground">GitHub CLI unavailable.</p>;
  if (status.availability.status === "notAuthenticated") return <p className="mx-2 mb-2 px-1.5 text-[11px] text-muted-foreground">Run <code className="font-mono text-foreground/80">{status.availability.remediation}</code> to connect GitHub.</p>;
  if (!prs?.length) return null;

  return <section className="mb-2" aria-label="Open pull requests">
    <p className="flex items-center gap-1.5 px-2 py-1 text-[10.5px] font-semibold tracking-[0.08em] text-muted-foreground/80">
      <GitPullRequest size={11} aria-hidden="true" /> PULL REQUESTS
      <span className="font-normal tabular-nums text-muted-foreground/60">{prs.length}</span>
    </p>
    {prs.map(pr => {
      const state = rollupState(pr);
      return <button
        type="button"
        key={pr.number}
        onClick={() => onOpen?.(pr.number)}
        title={`#${pr.number} ${pr.title} — checks ${state}`}
        className="flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left transition-colors hover:bg-accent"
      >
        <span className={cn("size-1.5 shrink-0 rounded-full", DOT[state])} aria-hidden="true" />
        <span className="shrink-0 font-mono text-[10.5px] tabular-nums text-muted-foreground">#{pr.number}</span>
        <span className="min-w-0 flex-1 truncate text-[12px] text-foreground/90">{pr.title}</span>
      </button>;
    })}
  </section>;
}
