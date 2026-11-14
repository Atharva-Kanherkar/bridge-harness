import { Check, ChevronUp, CloudOff, Eye, Square, X } from "lucide-react";
import { useReducedMotion } from "framer-motion";
import type { SessionPullRequest } from "../protocol/generated/protocol";
import { formatElapsed, type BackgroundOutcome, type BackgroundTaskView, type BackgroundWorkStore } from "../backgroundTasks";
import { chatPrKey, chatPrStatus } from "../chatPullRequests";
import { orderPullRequests, prCounts, shelfLabel, shelfPhrase, taskSummary, type ShelfTone } from "../workSummary";
import { cn } from "@/lib/utils";
import { PullRequestGlyph, useStatusAnnouncements } from "./ChatPullRequests";
import { HarnessMark } from "./harnessMarks";
import { useBackgroundTasks, type BackgroundNotice } from "./useBackgroundTasks";

// The glance: one line at the top of the composer that says how this chat's
// pull requests and background work are doing. It is part of the composer
// frame, so it can never overlap the transcript or stack against another
// floating element, and it is absent when there is nothing to say. Detail
// lives in the Work pane; the whole line opens it.

export const SHELF_TONE: Record<ShelfTone, string> = {
  danger: "text-destructive",
  success: "text-success",
  info: "text-info",
  neutral: "text-muted-foreground",
  faint: "text-faint",
};

const OUTCOME_WORD: Record<BackgroundOutcome, string> = { done: "Done", failed: "Failed", stopped: "Stopped" };

function OutcomeGlyph({ outcome }: { outcome: BackgroundOutcome }) {
  if (outcome === "done") return <Check size={13} className="shrink-0 text-success" aria-hidden="true" />;
  if (outcome === "failed") return <X size={13} className="shrink-0 text-destructive" aria-hidden="true" />;
  return <Square size={9} className="shrink-0 fill-current text-muted-foreground" aria-hidden="true" />;
}

/** Running work carries the harness mark, breathing; a watcher waits, so it is an eye. */
export function TaskKindGlyph({ kind, harness, size = 13 }: { kind: BackgroundTaskView["kind"]; harness?: string | null; size?: number }) {
  const reducedMotion = useReducedMotion() ?? false;
  return kind === "watching"
    ? <Eye size={size} className="shrink-0 text-muted-foreground" aria-hidden="true" />
    : <HarnessMark harness={harness} size={size} live={!reducedMotion} />;
}

function PullRequestPart({ prs }: { prs: SessionPullRequest[] }) {
  if (prs.length === 1) {
    const pr = prs[0];
    const phrase = shelfPhrase(pr);
    return <span className="flex min-w-0 flex-1 items-center gap-2">
      <PullRequestGlyph pr={pr} />
      <span className="shrink-0 font-medium tabular-nums text-foreground">#{pr.number}</span>
      <span className="min-w-0 truncate text-body">{pr.title}</span>
      <span data-shelf-phrase className={cn("ml-auto shrink-0 pl-2", SHELF_TONE[phrase.tone])}>{phrase.text}</span>
      {pr.stale && <CloudOff size={12} className="shrink-0 text-faint" aria-hidden="true" />}
    </span>;
  }
  const counts = prCounts(prs);
  return <span className="flex min-w-0 flex-1 items-center gap-2">
    <span className="flex shrink-0 items-center -space-x-1">
      {prs.slice(0, 3).map(pr => <span key={chatPrKey(pr)} className="grid place-items-center rounded-full bg-card">
        <PullRequestGlyph pr={pr} />
      </span>)}
    </span>
    <span className="shrink-0 font-medium text-foreground">{prs.length} pull requests</span>
    <span data-shelf-phrase className="ml-auto min-w-0 truncate pl-2">
      {counts.map((part, index) => <span key={part.label}>
        {index > 0 && <span className="text-faint"> · </span>}
        <span className={SHELF_TONE[part.tone]}>{part.count} {part.label}</span>
      </span>)}
    </span>
    {prs.some(pr => pr.stale) && <CloudOff size={12} className="shrink-0 text-faint" aria-hidden="true" />}
  </span>;
}

function TaskPart({ tasks, notice, harness, grow }: { tasks: BackgroundTaskView[]; notice: BackgroundNotice | null; harness?: string | null; grow: boolean }) {
  const frame = cn("flex min-w-0 items-center gap-2", grow ? "flex-1" : "max-w-[45%] shrink-0");
  if (notice) {
    return <span data-shelf-notice role="status" className={frame}>
      <OutcomeGlyph outcome={notice.outcome} />
      <span className="min-w-0 truncate text-foreground">{notice.label}</span>
      <span className="shrink-0 text-faint">{OUTCOME_WORD[notice.outcome]}{notice.more > 0 ? ` · +${notice.more}` : ""}</span>
    </span>;
  }
  const summary = taskSummary(tasks);
  if (summary.kind === "none") return null;
  if (summary.kind === "one") {
    const { task } = summary;
    return <span data-shelf-tasks className={frame}>
      <TaskKindGlyph kind={task.kind} harness={harness} />
      <span className="min-w-0 truncate text-body">{task.label}</span>
      {task.elapsedMs !== undefined && <span className="shrink-0 tabular-nums text-faint">{formatElapsed(task.elapsedMs)}</span>}
    </span>;
  }
  return <span data-shelf-tasks className={frame}>
    <TaskKindGlyph kind={summary.running ? "running" : "watching"} harness={harness} />
    <span className="min-w-0 truncate text-body">{summary.text}</span>
    {summary.oldestMs !== undefined && <span className="shrink-0 tabular-nums text-faint">{formatElapsed(summary.oldestMs)}</span>}
  </span>;
}

export function WorkShelf({ prs, tasks, notice, harness, onOpen, onFix }: {
  prs: SessionPullRequest[];
  tasks: BackgroundTaskView[];
  notice: BackgroundNotice | null;
  harness?: string | null;
  /** Open the Work pane, on this PR when the line is about exactly one. */
  onOpen: (number?: number) => void;
  /** Draft a fix prompt for a failing PR into the composer. */
  onFix?: (pr: SessionPullRequest) => void;
}) {
  const spoken = useStatusAnnouncements(prs);
  if (!prs.length && !tasks.length && !notice) return null;
  const ordered = orderPullRequests(prs);
  const lone = ordered.length === 1 ? ordered[0] : undefined;
  const fixable = lone && onFix && lone.state === "open" && chatPrStatus(lone).phase === "failing" ? lone : undefined;
  return <div data-work-shelf className="flex h-9 shrink-0 items-center gap-1 border-b border-border pr-1.5 text-[12.5px] text-muted-foreground">
    <button
      type="button"
      data-work-shelf-open
      onClick={() => onOpen(lone?.number)}
      aria-label={shelfLabel(ordered, taskSummary(tasks))}
      className="group/shelf flex h-full min-w-0 flex-1 items-center gap-3 rounded-tl-xl pl-3 pr-1 text-left transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
    >
      {ordered.length > 0 && <PullRequestPart prs={ordered} />}
      {(tasks.length > 0 || notice) && <>
        {ordered.length > 0 && <span className="h-4 w-px shrink-0 bg-border" aria-hidden="true" />}
        <TaskPart tasks={tasks} notice={notice} harness={harness} grow={ordered.length === 0} />
      </>}
      <ChevronUp size={13} className="shrink-0 text-faint transition-transform group-hover/shelf:-translate-y-px" aria-hidden="true" />
    </button>
    {fixable && <button
      type="button"
      data-work-shelf-fix
      onClick={() => onFix!(fixable)}
      className="inline-flex h-6 shrink-0 items-center rounded-md border border-border-card px-2 text-[11.5px] text-foreground transition-colors hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
    >
      Fix it
    </button>}
    <p aria-live="polite" className="sr-only">{spoken}</p>
  </div>;
}

/** The shelf for a chat: reads its background work itself, so the owner
 * (App, or a Mission Control tile) does not re-render on every tick of a
 * running task. A tile passes its own store; the open chat uses the global one. */
export function ChatWorkShelf({ sessionId, store, ...props }: Omit<Parameters<typeof WorkShelf>[0], "tasks" | "notice"> & { sessionId: string; store?: BackgroundWorkStore }) {
  const { tasks, notice } = useBackgroundTasks(sessionId, store);
  return <WorkShelf tasks={tasks} notice={notice} {...props} />;
}
