import { useEffect, useId, useRef, useState, type KeyboardEvent } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { ArrowUpRight, CloudOff, PanelRight, RotateCw } from "lucide-react";
import type { SessionPullRequest } from "../protocol/generated/protocol";
import { formatElapsed, type BackgroundTaskView } from "../backgroundTasks";
import { chatPrKey, chatPrStatus, checkTone, freshness, orderChecks } from "../chatPullRequests";
import { orderPullRequests, shelfPhrase } from "../workSummary";
import { MOTION_DURATION, useMotionTransition } from "../motion";
import { cn } from "@/lib/utils";
import { AttachPullRequest, CHECK_ICON, PullRequestGlyph } from "./ChatPullRequests";
import { SHELF_TONE, TaskKindGlyph } from "./WorkShelf";

// The detail behind the composer shelf: every pull request this chat opened
// or had attached, and every task its agent left running. One row grammar for
// both (glyph, name, status, time); one row open at a time. Nothing here
// floats over the conversation: the pane lives in the dock beside it.

export type WorkFocus = { number?: number; nonce: number };

const SECTION = "flex items-center gap-2 px-2 pb-1 pt-3 text-[10.5px] font-medium uppercase tracking-[0.08em] text-faint";
const ACTION = "inline-flex h-7 items-center gap-1 rounded-md border border-border-card px-2.5 text-[12px] text-foreground transition-colors hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

function useClock(intervalMs: number | null) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    setNow(Date.now());
    if (intervalMs === null) return;
    const id = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(id);
  }, [intervalMs]);
  return now;
}

function PullRequestDetail({ pr, now, onFix, onOpenGithub, onRetry, retrying }: {
  pr: SessionPullRequest;
  now: number;
  onFix?: (pr: SessionPullRequest) => void;
  onOpenGithub?: (number: number, tab: "conversation" | "checks") => void;
  onRetry: () => void;
  retrying: boolean;
}) {
  const ordered = orderChecks(pr.checkDetails);
  const failing = pr.state === "open" && chatPrStatus(pr).phase === "failing";
  return <div data-work-pr-detail className="mb-2 ml-[1.15rem] border-l border-border pl-3">
    {pr.stale && <div role="status" className="my-1.5 flex items-center gap-2 rounded-lg border border-border px-2.5 py-1.5 text-[11.5px] text-muted-foreground">
      <CloudOff size={12} className="shrink-0" aria-hidden="true" />
      <span className="min-w-0 flex-1">GitHub didn’t answer, so this is the last state Bridge saw{pr.error ? <span className="block truncate text-[10.5px] text-faint" title={pr.error}>{pr.error}</span> : null}</span>
      <button type="button" onClick={onRetry} disabled={retrying} className="inline-flex h-6 shrink-0 items-center gap-1 rounded-md border border-border-card px-2 text-[11.5px] text-foreground transition-colors hover:bg-accent disabled:opacity-60">
        <RotateCw size={11} className={cn(retrying && "animate-spin")} aria-hidden="true" /> Retry
      </button>
    </div>}
    {ordered.length > 0
      ? <ul aria-label={`Checks on #${pr.number}`} className="grid">
        {ordered.map((check, index) => {
          const tone = checkTone(check);
          const { icon: Icon, className, label } = CHECK_ICON[tone];
          return <li key={`${check.workflow}/${check.name}/${index}`} className="group/check flex h-7 min-w-0 items-center gap-2 text-[12px]">
            <Icon size={13} className={cn("shrink-0", className)} aria-label={label} />
            <span className="min-w-0 flex-1 truncate text-body">{check.name}</span>
            <span className="shrink-0 truncate text-[11px] text-faint">{tone === "skipped" ? "skipped" : check.workflow}</span>
            {check.logUrl && <a href={check.logUrl} target="_blank" rel="noreferrer" aria-label={`Logs for ${check.name}`} className={cn("inline-flex shrink-0 items-center gap-0.5 rounded px-1 text-[11px] transition-colors hover:bg-accent hover:text-foreground", tone === "failed" ? "text-destructive" : "text-muted-foreground opacity-0 group-hover/check:opacity-100 focus-visible:opacity-100")}>
              Logs <ArrowUpRight size={10} aria-hidden="true" />
            </a>}
          </li>;
        })}
      </ul>
      : <p className="py-1.5 text-[12px] text-faint">{pr.state === "open" ? "No checks reported yet." : "Checks no longer matter on a settled pull request."}</p>}
    <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
      {failing && onFix && <button type="button" onClick={() => onFix(pr)} className={cn(ACTION, "border-transparent bg-primary text-primary-foreground hover:bg-primary/90")}>Fix the failing check</button>}
      {onOpenGithub && <button type="button" onClick={() => onOpenGithub(pr.number, failing ? "checks" : "conversation")} className={ACTION}>
        <PanelRight size={12} aria-hidden="true" /> Open in GitHub pane
      </button>}
      <a href={pr.url} target="_blank" rel="noreferrer" className={ACTION}>GitHub <ArrowUpRight size={11} aria-hidden="true" /></a>
      <span className="ml-auto text-[11px] tabular-nums text-faint" title={pr.fetchedAt ?? undefined}>{pr.stale ? "last known · " : "updated "}{freshness(pr.fetchedAt, now)}</span>
    </div>
  </div>;
}

function PullRequestRow({ pr, open, onToggle, rowRef, ...detail }: {
  pr: SessionPullRequest;
  open: boolean;
  onToggle: () => void;
  rowRef?: (element: HTMLButtonElement | null) => void;
  now: number;
  onFix?: (pr: SessionPullRequest) => void;
  onOpenGithub?: (number: number, tab: "conversation" | "checks") => void;
  onRetry: () => void;
  retrying: boolean;
}) {
  const phrase = shelfPhrase(pr);
  const detailId = useId();
  return <li data-work-pr={pr.number} className="min-w-0">
    <button
      ref={rowRef}
      type="button"
      aria-expanded={open}
      aria-controls={open ? detailId : undefined}
      onClick={onToggle}
      className={cn("grid w-full min-w-0 grid-cols-[1rem_minmax(0,1fr)_auto] items-center gap-x-2.5 gap-y-0.5 rounded-lg px-2 py-2 text-left transition-colors hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring", open && "bg-accent")}
    >
      <PullRequestGlyph pr={pr} />
      <span className="flex min-w-0 items-baseline gap-1.5 text-[13px]">
        <span className="shrink-0 tabular-nums text-muted-foreground">#{pr.number}</span>
        <span className="min-w-0 truncate text-foreground">{pr.title}</span>
      </span>
      <span className={cn("shrink-0 text-[12px]", SHELF_TONE[phrase.tone])}>{phrase.text}</span>
      <span className="col-start-2 flex min-w-0 items-center gap-2 font-mono text-[11px] text-faint">
        <span className="min-w-0 truncate">{pr.headBranch}</span>
        {pr.headSha && <span className="shrink-0">{pr.headSha.slice(0, 7)}</span>}
        {pr.stale && <CloudOff size={11} className="shrink-0" aria-label="Last known state" />}
      </span>
    </button>
    {open && <div id={detailId}><PullRequestDetail pr={pr} {...detail} /></div>}
  </li>;
}

function TaskRow({ task, harness, open, onToggle }: { task: BackgroundTaskView; harness?: string | null; open: boolean; onToggle: () => void }) {
  const reveal = useMotionTransition(MOTION_DURATION.reveal);
  return <li data-work-task className="min-w-0">
    <button
      type="button"
      aria-expanded={open}
      onClick={onToggle}
      title={task.command}
      className={cn("grid w-full min-w-0 grid-cols-[1rem_minmax(0,1fr)_auto] items-center gap-x-2.5 gap-y-0.5 rounded-lg px-2 py-2 text-left transition-colors hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring", open && "bg-accent")}
    >
      <span className="grid place-items-center"><TaskKindGlyph kind={task.kind} harness={harness} /></span>
      <span className="min-w-0 truncate text-[13px] text-foreground">{task.label}</span>
      <span className="shrink-0 text-[12px] tabular-nums text-faint">{task.elapsedMs !== undefined ? formatElapsed(task.elapsedMs) : ""}</span>
      <span className="col-start-2 min-w-0 truncate text-[11px] text-faint">
        {task.kind === "watching" ? "Watching" : "Running"}{task.detail ? <span className="font-mono"> · {task.detail}</span> : null}
      </span>
    </button>
    <AnimatePresence initial={false}>
      {open && <motion.div key="output" initial={{ height: 0, opacity: 0 }} animate={{ height: "auto", opacity: 1 }} exit={{ height: 0, opacity: 0 }} transition={reveal} className="overflow-hidden">
        <div data-work-task-output className="mb-2 ml-[1.15rem] rounded-md border border-border bg-code px-3 py-2 font-mono text-[11px] leading-relaxed">
          {task.outputTail.length > 0
            ? <pre className="max-h-48 overflow-auto whitespace-pre-wrap break-words text-muted-foreground">{task.outputTail.join("\n")}</pre>
            : <p className="text-muted-foreground">{task.logFiles.length
              ? <>Output goes to {task.logFiles.map((file, index) => <span key={file} className="whitespace-nowrap text-foreground/80">{file}{index < task.logFiles.length - 1 ? ", " : ""}</span>)}</>
              : "No output yet"}</p>}
          {task.command && <p className="mt-1.5 truncate border-t border-border pt-1.5 text-faint" title={task.command}><span aria-hidden="true">$ </span>{task.command}</p>}
        </div>
      </motion.div>}
    </AnimatePresence>
  </li>;
}

export function WorkPane({ prs, tasks, harness, refreshing, focus, onAttach, onRetry, onFix, onOpenGithub, onEscape }: {
  prs: SessionPullRequest[];
  tasks: BackgroundTaskView[];
  harness?: string | null;
  refreshing: boolean;
  /** Set by the shelf: open the pane on this PR. The nonce makes a repeat click count. */
  focus?: WorkFocus;
  onAttach: (reference: string) => Promise<unknown>;
  onRetry: () => void;
  onFix?: (pr: SessionPullRequest) => void;
  onOpenGithub?: (number: number, tab: "conversation" | "checks") => void;
  /** Escape hands focus back to whoever opened the pane. */
  onEscape?: () => void;
}) {
  const ordered = orderPullRequests(prs);
  const [openPr, setOpenPr] = useState<string | null>(null);
  const [openTask, setOpenTask] = useState<string | null>(null);
  const rows = useRef(new Map<number, HTMLButtonElement>());
  const now = useClock(prs.length ? 15_000 : null);

  // A shelf click names the PR to show: open it, bring it into view, focus it.
  // Without a number (several PRs) the worst one leads the list already.
  useEffect(() => {
    if (!focus) return;
    const target = focus.number !== undefined ? ordered.find(pr => pr.number === focus.number) : ordered[0];
    if (!target) return;
    setOpenPr(chatPrKey(target));
    requestAnimationFrame(() => {
      const row = rows.current.get(target.number);
      row?.scrollIntoView?.({ block: "nearest" });
      row?.focus({ preventScroll: true });
    });
    // Only a new request moves the pane; a refetch of the same list must not.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [focus?.nonce]);

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "Escape" || !onEscape) return;
    if (event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement) return;
    event.preventDefault();
    onEscape();
  };

  const empty = ordered.length === 0 && tasks.length === 0;
  return <div data-work-pane onKeyDown={onKeyDown} className="h-full overflow-y-auto overscroll-contain px-2 pb-4">
    {empty && <div className="px-2 pb-2 pt-6">
      <p className="text-[13px] font-medium text-foreground">Nothing running</p>
      <p className="mt-1 max-w-sm text-[12.5px] leading-relaxed text-muted-foreground">Pull requests this chat opens and work its agent leaves running show up here, and in one line above the composer.</p>
    </div>}
    <section aria-label="Pull requests">
      {!empty && <h3 className={SECTION}>Pull requests<span className="tabular-nums">{ordered.length || ""}</span></h3>}
      {ordered.length > 0 && <ul className="grid gap-px">
        {ordered.map(pr => {
          const key = chatPrKey(pr);
          return <PullRequestRow
            key={key}
            pr={pr}
            open={openPr === key}
            onToggle={() => setOpenPr(current => current === key ? null : key)}
            rowRef={element => { if (element) rows.current.set(pr.number, element); else rows.current.delete(pr.number); }}
            now={now}
            onFix={onFix}
            onOpenGithub={onOpenGithub}
            onRetry={onRetry}
            retrying={refreshing}
          />;
        })}
      </ul>}
      <div className="px-1 pt-1"><AttachPullRequest onAttach={onAttach} /></div>
    </section>
    {tasks.length > 0 && <section aria-label="In the background">
      <h3 className={SECTION}>In the background<span className="tabular-nums">{tasks.length}</span></h3>
      <ul className="grid gap-px">
        {tasks.map(task => <TaskRow key={task.key} task={task} harness={harness} open={openTask === task.key} onToggle={() => setOpenTask(current => current === task.key ? null : task.key)} />)}
      </ul>
    </section>}
  </div>;
}

