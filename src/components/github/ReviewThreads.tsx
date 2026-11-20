import { useEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { Check, ChevronRight, CircleCheck, LoaderCircle, MessagesSquare, RotateCcw, SquareArrowOutUpRight } from "lucide-react";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { relativeTime } from "../workDashboard";
import { Avatar } from "./Avatar";
import { Composer } from "./Composer";
import { GithubMarkdown } from "./GithubMarkdown";
import { hunkTail } from "./patch";
import type { GithubAction, ReviewThread } from "../../protocol/generated/protocol";

// Review conversations: the part of a pull request a reviewer works through
// and closes. Each one is a card that says where it is, shows the lines it is
// about, lets you answer in place, and resolves with one button. A resolved
// conversation folds to a single line so the open ones are what the eye lands
// on; unfolding it is one click.

/** Run one write; `undefined` on success, otherwise GitHub's reason. */
export type ThreadWrite = (action: GithubAction) => Promise<string | undefined>;

type Filter = "all" | "open" | "resolved";

const EASE: [number, number, number, number] = [0.22, 1, 0.36, 1];

function Timestamp({ iso }: { iso: string }) {
  if (Number.isNaN(Date.parse(iso))) return null;
  return <time dateTime={iso} title={new Date(iso).toLocaleString()} className="text-caption text-muted-foreground">{relativeTime(iso, new Date())}</time>;
}

/** The lines a conversation was opened on. */
function Hunk({ hunk }: { hunk: string }) {
  const lines = useMemo(() => hunkTail(hunk), [hunk]);
  if (!lines.length) return null;
  return <pre aria-label="Commented lines" className="overflow-x-auto border-b border-border/70 bg-background/50 py-1 font-mono text-caption leading-5">
    {lines.map((line, index) => <span key={index} className={cn("flex whitespace-pre", line.kind === "add" && "bg-success/10 text-success", line.kind === "del" && "bg-destructive/10 text-destructive")}>
      <span aria-hidden="true" className="w-9 shrink-0 select-none pr-2 text-right tabular-nums text-muted-foreground/70">{line.newLine ?? line.oldLine ?? ""}</span>
      <span className="pr-3">{line.text || " "}</span>
    </span>)}
  </pre>;
}

function ThreadCard({ thread, index, number, repositoryUrl, headBranch, checkedOutHere, write, onJumpToFile }: {
  thread: ReviewThread;
  index: number;
  number: number;
  repositoryUrl?: string;
  headBranch: string;
  checkedOutHere: boolean;
  write: ThreadWrite;
  onJumpToFile: (path: string, line: number | undefined, headBranch: string) => void;
}) {
  const reduced = useReducedMotion();
  // What the reader just asked for, shown before GitHub's next read confirms
  // it. It is dropped the moment the thread's own state catches up or the
  // write is refused, so the card can never sit on a state GitHub rejected.
  const [intent, setIntent] = useState<boolean>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const resolved = intent ?? thread.isResolved;
  const [open, setOpen] = useState(!thread.isResolved);
  const [justResolved, setJustResolved] = useState(false);
  const alive = useRef(true);
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);
  useEffect(() => { setIntent(undefined); }, [thread.isResolved]);

  const rootId = thread.comments[0]?.databaseId ?? null;
  const line = thread.line ?? thread.originalLine ?? undefined;
  const first = thread.comments[0];
  const canToggle = resolved ? thread.viewerCanUnresolve || intent !== undefined : thread.viewerCanResolve || intent !== undefined;

  const toggle = async () => {
    const next = !resolved;
    setBusy(true); setError(undefined); setIntent(next);
    if (next) { setJustResolved(true); setOpen(false); } else setOpen(true);
    const failure = await write({ kind: "resolveThread", number, threadId: thread.id, resolved: next });
    if (!alive.current) return;
    setBusy(false);
    if (failure !== undefined) { setIntent(undefined); setError(failure); setOpen(true); setJustResolved(false); }
  };
  useEffect(() => {
    if (!justResolved) return;
    const timer = window.setTimeout(() => setJustResolved(false), 900);
    return () => window.clearTimeout(timer);
  }, [justResolved]);

  return <motion.article
    layout={reduced ? false : "position"}
    initial={reduced ? false : { opacity: 0, y: 10 }}
    animate={{ opacity: 1, y: 0 }}
    exit={reduced ? undefined : { opacity: 0, scale: 0.98 }}
    transition={{ duration: 0.28, ease: EASE, delay: reduced ? 0 : Math.min(index, 8) * 0.04 }}
    data-thread={thread.id}
    data-resolved={resolved || undefined}
    className={cn("overflow-hidden rounded-lg border bg-card transition-[border-color,box-shadow] duration-500", justResolved ? "border-success/50 ring-2 ring-success/15" : "border-border ring-0 ring-transparent", resolved && !justResolved && "bg-card/60")}
  >
    <div className={cn("flex items-center gap-1.5 px-2 py-1.5", open && "border-b border-border/70")}>
      <button
        type="button"
        onClick={() => setOpen(value => !value)}
        aria-expanded={open}
        aria-label={open ? "Collapse conversation" : "Expand conversation"}
        className="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
      >
        <ChevronRight size={12} aria-hidden="true" className={cn("transition-transform duration-200", open && "rotate-90")} />
      </button>
      <button
        type="button"
        onClick={() => onJumpToFile(thread.path, line === undefined ? undefined : Number(line), headBranch)}
        title={checkedOutHere ? "Open in the editor at this line" : `Open in the editor — ${headBranch} isn’t checked out here`}
        className="group inline-flex min-w-0 items-center gap-1.5 rounded-md px-1 py-0.5 font-mono text-caption text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
      >
        <span className="truncate">{thread.path}{line !== undefined ? `:${line}` : ""}</span>
        <SquareArrowOutUpRight size={11} className="shrink-0 opacity-0 transition-opacity group-hover:opacity-100" aria-hidden="true" />
      </button>
      <span className="ml-auto flex shrink-0 items-center gap-1.5">
        {thread.isOutdated && <Badge variant="outline" size="sm">Outdated</Badge>}
        <AnimatePresence initial={false}>
          {resolved && <motion.span
            key="resolved"
            initial={reduced ? false : { scale: 0.6, opacity: 0 }}
            animate={{ scale: 1, opacity: 1 }}
            exit={reduced ? undefined : { scale: 0.6, opacity: 0 }}
            transition={{ type: "spring", stiffness: 520, damping: 22 }}
          ><Badge variant="success" size="sm"><Check size={10} aria-hidden="true" />Resolved</Badge></motion.span>}
        </AnimatePresence>
        {canToggle && <button
          type="button"
          disabled={busy}
          onClick={() => void toggle()}
          className="inline-flex min-h-6 items-center gap-1 rounded-md border border-border bg-background px-2 py-0.5 text-caption font-medium text-foreground shadow-xs transition-[background-color,transform] hover:bg-accent active:scale-[0.97] disabled:opacity-60"
        >
          {busy ? <LoaderCircle size={11} className="animate-spin" aria-hidden="true" /> : resolved ? <RotateCcw size={11} aria-hidden="true" /> : <CircleCheck size={11} aria-hidden="true" />}
          {resolved ? "Unresolve" : "Resolve"}
        </button>}
      </span>
    </div>

    {!open && first && <button type="button" onClick={() => setOpen(true)} className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-meta text-muted-foreground transition-colors hover:bg-accent/40">
      <Avatar login={first.author?.login} className="size-4 text-micro" />
      <span className="shrink-0 font-medium text-foreground">{first.author?.login ?? "ghost"}</span>
      <span className="min-w-0 flex-1 truncate">{first.body.replace(/\s+/g, " ").slice(0, 160)}</span>
      {thread.comments.length > 1 && <span className="shrink-0 tabular-nums">{thread.comments.length - 1} {thread.comments.length === 2 ? "reply" : "replies"}</span>}
    </button>}

    <AnimatePresence initial={false}>
      {open && <motion.div
        key="body"
        initial={reduced ? false : { height: 0, opacity: 0 }}
        animate={{ height: "auto", opacity: 1 }}
        exit={reduced ? undefined : { height: 0, opacity: 0 }}
        transition={{ duration: 0.26, ease: EASE }}
        className="overflow-hidden"
      >
        {thread.diffHunk && <Hunk hunk={thread.diffHunk} />}
        <div className="space-y-3 px-3 py-2.5">
          <AnimatePresence initial={false}>
            {thread.comments.map(comment => <motion.div
              key={comment.id}
              layout={reduced ? false : "position"}
              initial={reduced ? false : { opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ duration: 0.24, ease: EASE }}
              className="flex gap-2"
            >
              <Avatar login={comment.author?.login} className="mt-0.5" />
              <div className="min-w-0 flex-1">
                <p className="flex items-baseline gap-2 text-caption">
                  <span className="font-medium text-foreground">{comment.author?.login ?? "ghost"}</span>
                  <Timestamp iso={comment.createdAt} />
                </p>
                <div className="mt-0.5"><GithubMarkdown text={comment.body} repositoryUrl={repositoryUrl} /></div>
              </div>
            </motion.div>)}
          </AnimatePresence>
          {error !== undefined && <p role="alert" className="rounded-md bg-destructive/10 px-2 py-1 text-caption text-destructive">{error}</p>}
          {rootId !== null && thread.viewerCanReply !== false && <Composer
            collapsed
            label="Reply"
            placeholder="Reply…"
            submitLabel="Reply"
            onSubmit={body => write({ kind: "reply", number, commentId: rootId, body })}
          />}
        </div>
      </motion.div>}
    </AnimatePresence>
  </motion.article>;
}

const FILTER_LABEL: Record<Filter, string> = { all: "All", open: "Open", resolved: "Resolved" };

export function ReviewThreads({ threads, number, repositoryUrl, headBranch, checkedOutHere, write, onJumpToFile }: {
  threads: ReviewThread[];
  number: number;
  repositoryUrl?: string;
  headBranch: string;
  checkedOutHere: boolean;
  write: ThreadWrite;
  onJumpToFile: (path: string, line: number | undefined, headBranch: string) => void;
}) {
  const [filter, setFilter] = useState<Filter>("all");
  const counts = { all: threads.length, open: threads.filter(thread => !thread.isResolved).length, resolved: threads.filter(thread => thread.isResolved).length };
  const shown = threads.filter(thread => filter === "all" || (filter === "resolved") === thread.isResolved);

  return <section aria-label="Review threads">
    <div className="mb-2.5 flex items-center gap-2">
      <h3 className="flex items-center gap-2 text-caption font-medium text-muted-foreground">
        Review threads
        <span className="font-mono text-caption tabular-nums tracking-normal text-muted-foreground">{threads.length}</span>
      </h3>
      {threads.length > 0 && <div role="radiogroup" aria-label="Filter conversations" className="ml-auto flex items-center gap-0.5 rounded-md border border-border bg-background/60 p-0.5">
        {(["all", "open", "resolved"] as const).map(value => <button
          key={value}
          type="button"
          role="radio"
          aria-checked={filter === value}
          onClick={() => setFilter(value)}
          className={cn("relative rounded-sm px-2 py-0.5 text-caption transition-colors", filter === value ? "font-medium text-foreground" : "text-muted-foreground hover:text-foreground")}
        >
          {filter === value && <motion.span layoutId={`thread-filter-${number}`} className="absolute inset-0 rounded-sm bg-card shadow-xs" transition={{ duration: 0.22, ease: EASE }} aria-hidden="true" />}
          <span className="relative">{FILTER_LABEL[value]} <span className="tabular-nums opacity-70">{counts[value]}</span></span>
        </button>)}
      </div>}
    </div>
    {threads.length === 0
      ? <p className="text-meta text-muted-foreground">No review threads.</p>
      : shown.length === 0
        ? <p className="flex items-center gap-2 rounded-lg border border-dashed border-border px-3 py-4 text-meta text-muted-foreground"><MessagesSquare size={13} aria-hidden="true" />{filter === "open" ? "Every conversation is resolved." : "Nothing resolved yet."}</p>
        : <div className="space-y-2.5">
          <AnimatePresence initial={false}>
            {shown.map((thread, index) => <ThreadCard
              key={thread.id}
              thread={thread}
              index={index}
              number={number}
              repositoryUrl={repositoryUrl}
              headBranch={headBranch}
              checkedOutHere={checkedOutHere}
              write={write}
              onJumpToFile={onJumpToFile}
            />)}
          </AnimatePresence>
        </div>}
  </section>;
}
