import { useEffect, useState } from "react";
import { Skeleton, SkeletonRegion, SkeletonRows } from "@/components/ui/skeleton";
import { AlertCircle, ExternalLink, RefreshCw } from "lucide-react";
import type { WorkBoard, WorkFactAction, WorkTask } from "../protocol/generated/protocol";
import { hasOpenableEvidence, sourceLabel, type TaskAction } from "./workTasks";
import { recentIntegrationActivity } from "./workActivity";
import { logoForSourceKind } from "./connectorLogos";
import { lastRunLine, toolsReadLine } from "./workDashboard";
import { Button } from "@/components/ui/button";

export type WorkActionOutcome = { ok: true } | { ok: false; reason: string };

export type WorkViewProps = {
  board?: WorkBoard;
  error?: string;
  refreshError?: string;
  onRefresh: () => void;
  onAction: (action: WorkFactAction) => Promise<WorkActionOutcome>;
  now?: Date;
  onTaskAction?: (task: WorkTask, action: TaskAction) => Promise<WorkActionOutcome>;
  onTogglePin?: (task: WorkTask) => Promise<WorkActionOutcome>;
  onOpenEvidence?: (task: WorkTask) => void;
  onOpenTask?: (task: WorkTask) => void;
  onRunBriefing?: () => void;
  onOpenSettings?: () => void;
};

export function WorkView({ board, error, refreshError, onRefresh, now, onOpenEvidence, onRunBriefing, onOpenSettings }: WorkViewProps) {
  const [clock, setClock] = useState(() => Date.now());
  useEffect(() => {
    if (now) return;
    const timer = window.setInterval(() => setClock(Date.now()), 30_000);
    return () => window.clearInterval(timer);
  }, [now]);
  const currentTime = now ?? new Date(clock);
  const items = recentIntegrationActivity(board?.tasks ?? [], currentTime);
  const configured = board !== undefined && board.suggestions.state !== "not_configured";
  const running = board?.suggestions.state === "running";
  const refresh = configured && onRunBriefing ? onRunBriefing : onRefresh;

  return (
    <section aria-label="Work" className="mx-auto flex min-h-0 w-full max-w-page flex-1 flex-col overflow-y-auto">
      <header className="flex flex-wrap items-start gap-3 px-5 pb-5 pt-6 sm:px-8" data-tauri-drag-region="deep">
        <div className="min-w-0">
          <h2 className="font-display text-title font-semibold tracking-tight">Past 24 hours</h2>
          {board || error
            ? <p aria-live="polite" className="mt-1 text-xs text-muted-foreground">{board ? "Activity from your connected integrations." : "Activity could not be loaded."}</p>
            : <Skeleton className="mt-2 h-2.5 w-56" />}
        </div>
        <Button variant="outline" size="sm" disabled={running} loading={running} onClick={refresh} className="ml-auto">
          <RefreshCw size={13} aria-hidden="true" />
          Refresh
        </Button>
      </header>

      <div className="flex flex-col gap-3 px-5 pb-6 sm:px-8">
        {board && !configured && (
          <div className="u-glass-soft rounded-xl border border-border p-4">
            <h3 className="text-sm font-medium">Connect your work</h3>
            <p className="mt-1 text-xs leading-relaxed text-muted-foreground">Choose a briefing model and your connected tools in Settings → Work to see recent Slack, GitHub, and other integration activity.</p>
            {onOpenSettings && <Button variant="outline" size="sm" onClick={onOpenSettings} className="mt-3">Set up integrations</Button>}
          </div>
        )}

        {board && running && <SkeletonRegion label="Reading your connected integrations"><Skeleton className="h-2.5 w-72 max-w-full" /></SkeletonRegion>}
        {board && board.latestRun && !running && (
          <div aria-label="Briefing status" className="text-xs leading-relaxed text-muted-foreground">
            {lastRunLine(board.latestRun, currentTime)} {toolsReadLine(board.sources)}
          </div>
        )}
        {(refreshError || board?.suggestions.state === "degraded") && (
          <div role="status" className="flex items-start gap-2 rounded-lg border border-border p-3 text-xs text-muted-foreground">
            <AlertCircle size={14} className="shrink-0" aria-hidden="true" />
            <p>Could not refresh all activity. {refreshError ?? board?.suggestions.detail ?? "Try again."} Only previously read items still within the past 24 hours are shown.</p>
          </div>
        )}

        {!board && error ? (
          <div role="alert" className="rounded-xl border border-border p-5">
            <h3 className="text-sm font-medium">Activity could not be loaded</h3>
            <p className="mt-1 text-xs text-muted-foreground">{error}</p>
            <Button variant="outline" size="sm" onClick={onRefresh} className="mt-3">Try again</Button>
          </div>
        ) : !board || (running && items.length === 0) ? (
          <SkeletonRows label="Loading activity" rows={3} lead="dot" />
        ) : items.length === 0 ? (
          <div className="rounded-xl border border-border p-6 text-center">
            <h3 className="text-sm font-medium">No recent activity to show</h3>
            <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
              {configured ? "Refresh to check your connected tools for updates from the past 24 hours." : "Recent activity will appear after you set up and refresh your integrations."}
            </p>
          </div>
        ) : (
          <ul aria-label="Integration activity" className="flex flex-col gap-2">
            {items.map(item => {
              const Logo = logoForSourceKind(item.sourceKind);
              return (
                <li key={item.id} className="u-glass-soft flex gap-3 rounded-xl border border-border p-4">
                  {Logo && <span className="mt-0.5 shrink-0 text-muted-foreground" aria-hidden="true"><Logo size={16} /></span>}
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-caption text-muted-foreground">
                      <span>{sourceLabel(item)}</span>
                      <time dateTime={item.sourceActivityAt!}>{new Date(item.sourceActivityAt!).toLocaleString(undefined, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" })}</time>
                    </div>
                    <h3 className="mt-1 text-sm font-medium [overflow-wrap:anywhere]">{item.title}</h3>
                    <p className="mt-1 text-xs leading-relaxed text-muted-foreground [overflow-wrap:anywhere]">{item.why}</p>
                    {onOpenEvidence && hasOpenableEvidence(item) && item.evidenceTarget?.kind === "externalLink" && (
                      <Button variant="outline" size="sm" onClick={() => onOpenEvidence(item)} className="mt-3">
                        <ExternalLink size={12} aria-hidden="true" />Open in {item.evidenceTarget.host}
                      </Button>
                    )}
                  </div>
                </li>
              );
            })}
          </ul>
        )}
      </div>
    </section>
  );
}

export default WorkView;
