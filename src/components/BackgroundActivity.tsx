import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { Check, ChevronUp, Square, X } from "lucide-react";
import type { ConversationItem } from "../conversation";
import { describeBackgroundTask, formatElapsed, type BackgroundOutcome, type BackgroundTaskKind, type BackgroundTaskView } from "../backgroundTasks";
import { cn } from "@/lib/utils";
import { MOTION_DURATION, useMotionTransition } from "../motion";
import { HarnessMark } from "./harnessMarks";

/// Work an agent left running after its reply: a test run it backgrounded, a
/// watcher it armed. The model itself is idle and can be talked to, so this is
/// not the thinking row and the session is not `working`; it is a small island
/// at the foot of the chat that says the agent still has work going, and opens
/// into the list when asked.

/// The panel floats over the transcript, so the scroller is what clips it.
function scrollParent(element: HTMLElement | null): HTMLElement | null {
  for (let node = element?.parentElement; node; node = node.parentElement) {
    if (/(auto|scroll)/.test(getComputedStyle(node).overflowY)) return node;
  }
  return null;
}

type Placement = { side: "above" | "below"; maxHeight: number };
const PANEL_GAP = 8;
const PANEL_MAX = 360;

/// Opens toward whichever side of the island has more room inside the
/// scroller, and never taller than that room: a short chat has its island near
/// the top, where an upward panel would be cut off.
function placePanel(anchor: HTMLElement | null): Placement {
  const scroller = scrollParent(anchor);
  if (!anchor || !scroller) return { side: "above", maxHeight: PANEL_MAX };
  const box = anchor.getBoundingClientRect();
  const frame = scroller.getBoundingClientRect();
  const above = box.top - frame.top - PANEL_GAP * 2;
  const below = frame.bottom - box.bottom - PANEL_GAP * 2;
  const side = above >= Math.min(PANEL_MAX, below) ? "above" : "below";
  return { side, maxHeight: Math.max(120, Math.min(PANEL_MAX, side === "above" ? above : below)) };
}

function useNow(intervalMs: number): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(id);
  }, [intervalMs]);
  return now;
}

const DOT_DELAYS = ["[animation-delay:0ms]", "[animation-delay:240ms]", "[animation-delay:480ms]", "[animation-delay:720ms]", "[animation-delay:960ms]"];
const SHOWN_DOTS = DOT_DELAYS.length;

/// A running task breathes; a watcher is a still ring, because it is waiting
/// rather than doing.
function TaskGlyph({ kind, index = 0, className }: { kind: BackgroundTaskKind; index?: number; className?: string }) {
  return kind === "watching"
    ? <span className={cn("block shrink-0 rounded-full border border-muted-foreground", className)} aria-hidden="true"/>
    : <span className={cn("block shrink-0 rounded-full bg-foreground/75 animate-[thinking-pulse_2s_ease-in-out_infinite] motion-reduce:animate-none", DOT_DELAYS[index % SHOWN_DOTS], className)} aria-hidden="true"/>;
}

function TaskRow({ task, open, onToggle }: { task: BackgroundTaskView; open: boolean; onToggle: () => void }) {
  const reveal = useMotionTransition(MOTION_DURATION.reveal);
  return <li className="min-w-0">
    <button
      type="button"
      aria-expanded={open}
      onClick={onToggle}
      title={task.command}
      className="flex w-full min-w-0 flex-col gap-0.5 rounded-lg px-2.5 py-2 text-left transition-colors hover:bg-accent/60 focus-visible:bg-accent/60"
    >
      <span className="flex w-full min-w-0 items-center gap-2.5">
        <TaskGlyph kind={task.kind} className="size-1.5"/>
        <span className="min-w-0 flex-1 truncate text-[13px] text-foreground">{task.label}</span>
        <span className="sr-only">{task.kind === "watching" ? "Watching" : "Running"}</span>
        {task.elapsedMs !== undefined && <span className="shrink-0 text-[11.5px] tabular-nums text-faint">{formatElapsed(task.elapsedMs)}</span>}
      </span>
      {task.detail && <span className="block truncate pl-4 font-mono text-[11px] text-muted-foreground">{task.detail}</span>}
    </button>
    <AnimatePresence initial={false}>
      {open && <motion.div
        key="output"
        initial={{ height: 0, opacity: 0 }}
        animate={{ height: "auto", opacity: 1 }}
        exit={{ height: 0, opacity: 0 }}
        transition={reveal}
        className="overflow-hidden"
      >
        <div className="mb-1.5 ml-6 mr-2 rounded-md bg-code px-3 py-2 font-mono text-[11px] leading-relaxed">
          {task.outputTail.length > 0
            ? <pre className="max-h-40 overflow-auto whitespace-pre-wrap break-words text-muted-foreground">{task.outputTail.join("\n")}</pre>
            : <p className="text-muted-foreground">{task.logFiles.length
              ? <>Output goes to {task.logFiles.map((file, index) => <span key={file} className="whitespace-nowrap text-foreground/80">{file}{index < task.logFiles.length - 1 ? ", " : ""}</span>)}</>
              : "No output yet"}</p>}
          {task.command && <p className="mt-1.5 truncate border-t border-border pt-1.5 text-faint" title={task.command}><span aria-hidden="true">$ </span>{task.command}</p>}
        </div>
      </motion.div>}
    </AnimatePresence>
  </li>;
}

const OUTCOME_WORD: Record<BackgroundOutcome, string> = { done: "Done", failed: "Failed", stopped: "Stopped" };
const NOTICE_MS = 4000;

type Notice = { key: string; label: string; outcome: BackgroundOutcome; more: number };

function OutcomeGlyph({ outcome }: { outcome: BackgroundOutcome }) {
  if (outcome === "done") return <Check size={13} className="shrink-0 text-success" aria-hidden="true"/>;
  if (outcome === "failed") return <X size={13} className="shrink-0 text-destructive" aria-hidden="true"/>;
  return <Square size={9} className="shrink-0 fill-current text-muted-foreground" aria-hidden="true"/>;
}

export function BackgroundActivity({ items, harness, scope, outcomeOf }: {
  items: ConversationItem[];
  harness?: string | null;
  /** The chat these tasks belong to; switching chats never announces the old one's tasks. */
  scope?: string;
  /** How a task that just left `items` ended, by its key. */
  outcomeOf?: (key: string) => BackgroundOutcome | undefined;
}) {
  const reducedMotion = useReducedMotion() ?? false;
  const panelId = useId();
  const root = useRef<HTMLDivElement>(null);
  const pill = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const [openTask, setOpenTask] = useState<string | null>(null);
  const [placement, setPlacement] = useState<Placement>({ side: "above", maxHeight: PANEL_MAX });
  const [notice, setNotice] = useState<Notice | null>(null);
  const now = useNow(1000);
  const tasks = useMemo(() => items.map(item => describeBackgroundTask(item, now)), [items, now]);
  const panelTransition = useMotionTransition(MOTION_DURATION.overlay);
  const swap = useMotionTransition(MOTION_DURATION.tick);
  const outcomes = useRef(outcomeOf);
  outcomes.current = outcomeOf;

  // A task that leaves the list gets one line in the island before it goes.
  const previous = useRef<{ scope?: string; labels: Map<string, string> }>({ scope, labels: new Map() });
  useEffect(() => {
    const labels = new Map(tasks.map(task => [task.key, task.label]));
    const before = previous.current;
    previous.current = { scope, labels };
    if (before.scope !== scope) { setNotice(null); return; }
    const finished = [...before.labels].flatMap(([key, label]) => {
      if (labels.has(key)) return [];
      const outcome = outcomes.current?.(key);
      return outcome ? [{ key, label, outcome }] : [];
    });
    if (!finished.length) return;
    const shown = finished.find(task => task.outcome === "failed") ?? finished[0];
    setNotice({ ...shown, more: finished.length - 1 });
  }, [tasks, scope]);
  useEffect(() => {
    if (!notice) return;
    const id = setTimeout(() => setNotice(null), NOTICE_MS);
    return () => clearTimeout(id);
  }, [notice]);

  useLayoutEffect(() => {
    if (open) setPlacement(placePanel(pill.current));
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const away = (event: PointerEvent) => { if (!root.current?.contains(event.target as Node)) setOpen(false); };
    const escape = (event: KeyboardEvent) => { if (event.key === "Escape") setOpen(false); };
    document.addEventListener("pointerdown", away);
    document.addEventListener("keydown", escape);
    return () => { document.removeEventListener("pointerdown", away); document.removeEventListener("keydown", escape); };
  }, [open]);

  if (!tasks.length && !notice) return null;
  const running = tasks.filter(task => task.kind === "running").length;
  const oldest = Math.max(0, ...tasks.map(task => task.elapsedMs ?? 0));
  const count = `${tasks.length} background ${tasks.length === 1 ? "task" : "tasks"}`;
  const summary = running > 0 ? "Working in background" : "Watching in background";
  const swapIn = { initial: { opacity: 0, y: 4 }, animate: { opacity: 1, y: 0 }, exit: { opacity: 0, y: -4 }, transition: swap };

  return <div ref={root} data-current-activity role="region" aria-label="Background work" className="pointer-events-none sticky bottom-4 z-10 mx-auto mt-3 flex w-full max-w-conversation">
    <div className="pointer-events-auto relative">
      <AnimatePresence>
        {open && tasks.length > 0 && <motion.div
          key="panel"
          id={panelId}
          data-background-panel
          initial={{ opacity: 0, y: placement.side === "above" ? 6 : -6, scale: 0.97 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          exit={{ opacity: 0, y: placement.side === "above" ? 6 : -6, scale: 0.97 }}
          transition={panelTransition}
          style={{ maxHeight: placement.maxHeight }}
          className={cn("u-overlay-strong absolute left-0 flex w-[min(26rem,calc(100vw-3rem))] flex-col overflow-hidden rounded-xl",
            placement.side === "above" ? "bottom-full mb-2 origin-bottom-left" : "top-full mt-2 origin-top-left")}
        >
          <p className="flex shrink-0 items-baseline justify-between px-4 pb-1 pt-3 text-[11px] font-medium uppercase tracking-[0.06em] text-faint">
            <span>In the background</span>
            <span className="tabular-nums">{tasks.length}</span>
          </p>
          <ul className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-1.5 pb-1.5">
            {tasks.map(task => <TaskRow key={task.key} task={task} open={openTask === task.key} onToggle={() => setOpenTask(current => current === task.key ? null : task.key)}/>)}
          </ul>
        </motion.div>}
      </AnimatePresence>
      <button
        ref={pill}
        data-background-pill
        type="button"
        aria-expanded={tasks.length ? open : undefined}
        aria-controls={open && tasks.length ? panelId : undefined}
        aria-label={notice ? `${notice.label}: ${OUTCOME_WORD[notice.outcome]}` : `${summary}, ${count}`}
        disabled={!tasks.length}
        onClick={() => setOpen(value => !value)}
        className="u-overlay group inline-flex h-8 max-w-[min(32rem,calc(100vw-3rem))] items-center overflow-hidden rounded-full pl-2.5 pr-3 text-[12.5px] text-muted-foreground transition-colors hover:text-foreground disabled:cursor-default"
      >
        <AnimatePresence mode="wait" initial={false}>
          {notice
            ? <motion.span key={`notice:${notice.key}`} data-background-notice role="status" className="flex min-w-0 items-center gap-2" {...swapIn}>
              <OutcomeGlyph outcome={notice.outcome}/>
              <span className="min-w-0 truncate text-foreground">{notice.label}</span>
              <span className="shrink-0 text-faint">{OUTCOME_WORD[notice.outcome]}{notice.more > 0 ? ` · +${notice.more}` : ""}</span>
            </motion.span>
            : <motion.span key="summary" className="flex min-w-0 items-center gap-2.5" {...swapIn}>
              <HarnessMark harness={harness} size={13} live={!reducedMotion}/>
              <span className="truncate text-foreground">{summary}</span>
              <span className="flex shrink-0 items-center gap-[3px]" aria-hidden="true">
                {tasks.slice(0, SHOWN_DOTS).map((task, index) => <TaskGlyph key={task.key} kind={task.kind} index={index} className="size-[5px]"/>)}
                {tasks.length > SHOWN_DOTS && <span className="ml-0.5 text-[11px] tabular-nums">+{tasks.length - SHOWN_DOTS}</span>}
              </span>
              {oldest > 0 && <span className="shrink-0 tabular-nums text-faint">{formatElapsed(oldest)}</span>}
              <ChevronUp size={12} className={cn("shrink-0 transition-transform duration-200", open && "rotate-180")} aria-hidden="true"/>
            </motion.span>}
        </AnimatePresence>
      </button>
    </div>
  </div>;
}
