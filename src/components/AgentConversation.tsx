import { memo, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { AlertTriangle, Brain, Check, ChevronDown, ChevronRight, Circle, CornerDownRight, FilePlus2, FileText, Gauge, GitFork, Globe, ListChecks, LoaderCircle, Maximize2, Navigation, Pencil, Pin, RotateCcw, Search, SquareTerminal, Wrench, X } from "lucide-react";
import { attachmentUris, delegationChildSessionId, delegationFacet, foldWorkerDelegations, groupItems, isToolItem, mergeConversationProjections, projectSessionConversation, reduceConversation, sameItem, sameItems, toolCallDisplay, type ConversationItem, type ToolGlyph, type ToolVerb } from "../conversation";
import { humanizeApprovalReason, humanizeCheckKind, humanizeCheckStatus, humanizeResolution } from "../humanize";
import { pickGreeting, type GreetingPart } from "../greetings";
import type { AgentEvent, ApprovalDecision, CompletionSummary, ContinuationFidelity, Session, SessionEntry, SessionStartupPhase, WorkerRepositoryBinding, WorkerRuntimeRecord } from "../types";
import { latestUsageSnapshot, type UsageSnapshot } from "../usage";
import { describeError } from "../errors";
import { looksLikeDiff } from "./highlight";
import { PatchView } from "./DiffView";
import { FileLinkContext, Markdown, MentionText, parseFileRef, type FileLinks } from "./Markdown";
import { formatElapsed, harnessLabel, modelLabel } from "../utils";
import { cn } from "@/lib/utils";
import { MOTION_DURATION, useMotionStagger, useMotionTransition } from "../motion";
import { workerPanelModel, type WorkerPanelModel } from "./workerPanel";
import type { WorkerTone } from "./workerStatus";
import { bridgeApi } from "../api";
import { computeNarration, type NarrationView } from "../startupNarration";
import { HarnessMark } from "./harnessMarks";
import type { InteractionResolutionResult, QuestionAction } from "../protocol/generated/protocol";

type ResolvePermission = (eventId: number, decision: ApprovalDecision, optionId?: string) => Promise<InteractionResolutionResult | void> | void;
type ResolveQuestion = (eventId: number, action: QuestionAction, answers: Record<string, string[]>) => Promise<InteractionResolutionResult | void> | void;

function providerLabel(harness?: string | null): string | undefined {
  return harness ? harnessLabel(harness) : undefined;
}

// A conversation of prose messages, live tool-call cards, clickable thinking,
// and a turn's tool work folded into one activity group that expands into
// per-action rows. The folding rules themselves are pure data and live in
// `transcript/grouping.ts`; this file only draws what they decide.

/* ── Shared surfaces ─────────────────────────────────────────────────────
   Chrome is achromatic and elevation is a lightness ladder, so an alert is a
   plain card wearing a colored tick rather than a tinted panel. A full wash is
   held back for genuine failures. */

/// The user's turn is the only bubble in the transcript; the agent answers
/// straight onto the canvas. That asymmetry is what carries the hierarchy.
///
/// The entrance used to live here as `chat-message-enter`. It now belongs to the
/// `TranscriptRow` wrapper: a CSS animation replays on every remount and cannot
/// be told "only the row that just arrived", which is exactly what a transcript
/// needs.
const BUBBLE = "ml-auto w-fit max-w-[85%] whitespace-pre-wrap break-words rounded-2xl border border-border bg-card px-3.5 py-2 text-[14px] leading-[1.7] tracking-[-0.006em] text-foreground";
/// Transcript-level notice: a quiet card that reads as a margin note.
const NOTICE = "mb-4 rounded-lg border border-border border-l-2 bg-card px-3 py-2 text-xs text-muted-foreground";
/// A decision the user has to make — approvals, adoptions, stale bases.
const PANEL = "my-4 min-w-0 overflow-hidden rounded-lg border border-border border-l-2 bg-card";
/// Verbatim text — paths, commands, diffstats — sits in an inset code well.
const WELL = "block rounded-md border border-border bg-code px-2.5 py-2 font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap break-words overflow-x-auto text-foreground";
const BTN_PRIMARY = "inline-flex items-center gap-1.5 rounded-full bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground transition-colors hover:bg-primary/90 disabled:opacity-50";
const BTN_SECONDARY = "inline-flex items-center gap-1.5 rounded-full border border-input px-3 py-1.5 text-xs font-medium text-foreground transition-colors hover:bg-accent disabled:opacity-50";

/* ── Transcript motion ───────────────────────────────────────────────────
   Three moves, and only three: a row arriving rises 6px into place, a
   disclosure body animates its own height rather than snapping, and a status
   glyph is swapped rather than replaced. Everything is duration-collapsed under
   reduced motion by `useMotionTransition`. */

/// One row of the transcript. Lives under an `AnimatePresence initial={false}`,
/// so a row present on the first render appears without animating and only the
/// rows that actually arrive later rise in — switching sessions must not replay
/// the whole history.
function TranscriptRow({ tone = "quiet", className, id, entryId, children }: {
  tone?: "quiet" | "alert";
  className?: string;
  id?: string;
  entryId?: string;
  children: ReactNode;
}) {
  // An error has further to travel than an ordinary row: the extra 6px and the
  // slightly longer settle are what make it read as an interruption without
  // resorting to a shake, which this design system would wear badly.
  const alert = tone === "alert";
  const transition = useMotionTransition(alert ? MOTION_DURATION.reveal * 1.5 : MOTION_DURATION.reveal);
  return (
    <motion.div
      id={id}
      data-entry-id={entryId}
      className={className}
      layout="position"
      initial={{ opacity: 0, y: alert ? 12 : 6 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, y: -4 }}
      transition={transition}
    >
      {children}
    </motion.div>
  );
}

/// A disclosure body that animates its own height open and closed.
///
/// The one thing CSS genuinely cannot do here: `height: auto` is not an
/// animatable value, so tool output and activity groups used to snap.
function Disclosure({ open, className, children }: { open: boolean; className?: string; children: ReactNode }) {
  const transition = useMotionTransition();
  return (
    <AnimatePresence initial={false}>
      {open && (
        <motion.div
          className={cn("overflow-hidden", className)}
          initial={{ height: 0, opacity: 0 }}
          animate={{ height: "auto", opacity: 1 }}
          exit={{ height: 0, opacity: 0 }}
          transition={transition}
        >
          {children}
        </motion.div>
      )}
    </AnimatePresence>
  );
}

/// The one glyph that says what a tool call is doing right now.
///
/// Swapped through `AnimatePresence mode="wait"` so a run finishing reads as the
/// spinner giving way to the tick, rather than one glyph being overwritten by
/// another between two frames.
function StatusGlyph({ live, failed, succeeded }: { live: boolean; failed: boolean; succeeded: boolean }) {
  const transition = useMotionTransition(MOTION_DURATION.tick);
  const state = live ? "live" : failed ? "failed" : succeeded ? "ok" : "idle";
  return (
    <AnimatePresence mode="wait" initial={false}>
      {state !== "idle" && (
        <motion.span
          key={state}
          className="flex shrink-0 items-center"
          initial={{ opacity: 0, scale: 0.7 }}
          animate={{ opacity: 1, scale: 1 }}
          exit={{ opacity: 0, scale: 0.7 }}
          transition={transition}
        >
          {state === "live" && <LoaderCircle size={12} className="animate-spin text-muted-foreground" aria-hidden="true"/>}
          {state === "ok" && <Check size={12} className="text-success" aria-hidden="true"/>}
          {state === "failed" && <X size={12} className="text-destructive" aria-hidden="true"/>}
        </motion.span>
      )}
    </AnimatePresence>
  );
}

/* ── Tool-call presentation ─────────────────────────────────────────────── */

// The row is achromatic on purpose: the tool glyph identifies the action, and
// colour is left to the things that carry meaning — diffstats, exit codes and
// failures. Reading the call apart lives in `conversation.ts`; all that is left
// here is choosing an icon for the verb it reports.
const TOOL_ICON: Record<ToolGlyph, React.ReactNode> = {
  pencil: <Pencil size={12}/>,
  "file-plus": <FilePlus2 size={12}/>,
  file: <FileText size={12}/>,
  terminal: <SquareTerminal size={12}/>,
  search: <Search size={12}/>,
  globe: <Globe size={12}/>,
  fork: <GitFork size={12}/>,
  list: <ListChecks size={12}/>,
  wrench: <Wrench size={12}/>,
};

/// Reads and searches earn less ink than writes: they stay flat rows under a
/// group label, while an edit or a command becomes a card with a body.
const FLAT_VERBS = new Set<ToolVerb>(["read", "search"]);

/// What a collapsed run says it did, in the order the work reads: commands
/// first, then the files it looked at, then the files it changed. This is the
/// only description of a hundred steps most readers will ever want, so it
/// names them by verb and count rather than by a step total alone.
function summarize(items: ConversationItem[], live: boolean): string {
  const counts: Record<ToolVerb, number> = { edit: 0, read: 0, run: 0, search: 0, tool: 0 };
  for (const item of items) if (isToolItem(item)) counts[toolCallDisplay(item).verb] += 1;
  const noun = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;
  const parts: string[] = [];
  if (counts.run) parts.push(live ? `running ${noun(counts.run, "command", "commands")}` : `ran ${noun(counts.run, "command", "commands")}`);
  if (counts.read) parts.push(live ? `reading ${noun(counts.read, "file", "files")}` : `read ${noun(counts.read, "file", "files")}`);
  if (counts.edit) parts.push(live ? `editing ${noun(counts.edit, "file", "files")}` : `edited ${noun(counts.edit, "file", "files")}`);
  if (counts.search) parts.push(live ? `searching the web` : `searched the web`);
  if (counts.tool) parts.push(live ? `using ${noun(counts.tool, "tool", "tools")}` : `used ${noun(counts.tool, "tool", "tools")}`);
  if (!parts.length) return live ? "Working…" : "Done";
  const text = parts.join(", ");
  const sentence = text.charAt(0).toUpperCase() + text.slice(1);
  return live ? `${sentence}…` : sentence;
}

const OUTPUT_DISPLAY_CAP = 6000;

function CappedOutput({ text, className }: { text: string; className?: string }) {
  const [all, setAll] = useState(false);
  const clipped = !all && text.length > OUTPUT_DISPLAY_CAP;
  return (
    <div>
      <pre className={className}>{clipped ? text.slice(-OUTPUT_DISPLAY_CAP) : text}</pre>
      {clipped && (
        <button
          type="button"
          className="px-3.5 pb-2.5 text-left text-[11px] text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
          onClick={() => setAll(true)}
        >
          earlier output hidden — show all
        </button>
      )}
    </div>
  );
}

function formatThoughtDuration(ms: number): string {
  const total = Math.max(1, Math.round(ms / 1000));
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  if (minutes === 0) return `${seconds}s`;
  return `${minutes}m ${String(seconds).padStart(2, "0")}s`;
}

/// `exit 0` / `exit 2`, wherever the provider actually reports one — so a
/// command's outcome stops hiding inside a checkmark. Absent everywhere else:
/// an unreported exit code is not the same fact as a zero one.
function ExitChip({ code }: { code: number }) {
  return <span className={cn("shrink-0 rounded-full border border-border px-1.5 py-px text-[10px]", code === 0 ? "text-success" : "text-destructive")}>exit {code}</span>;
}

/// A command the way a terminal shows one: a `❯` prompt line carrying what ran,
/// and the output dimmed a step below it on the code ground.
function TerminalBlock({ command, output }: { command?: string; output?: string }) {
  return <div className="bg-code font-mono text-[11.5px] leading-[1.7]">
    {command && <div className="flex gap-2 px-3.5 pb-1 pt-2.5">
      <span className="shrink-0 select-none font-semibold text-success" aria-hidden="true">❯</span>
      <span className="min-w-0 whitespace-pre-wrap break-words text-foreground">{command}</span>
    </div>}
    {output && <CappedOutput text={output} className="max-h-[260px] overflow-auto whitespace-pre-wrap break-words px-3.5 pb-2.5 pl-[30px] text-muted-foreground"/>}
  </div>;
}

/// The quiet header over a run of reads and searches. Exploration is context,
/// not a step, so it gets one label and a hairline rather than a card each.
function GroupLabel({ children }: { children: ReactNode }) {
  return <div className="mb-1 flex items-center gap-2 pl-0.5 font-mono text-[10.5px] uppercase tracking-[0.08em] text-muted-foreground/70">
    {children}
    <span className="h-px flex-1 bg-border" aria-hidden="true"/>
  </div>;
}

/// One tool call in three layers: a glanceable summary row, the body it opens
/// into, and — for a patch — the remaining hunks one more click away.
///
/// An edit opens itself. The transcript used to make a diff something you had to
/// go looking for twice — expand the group, then expand the row — and even then
/// the patch was sliced to its last 8,000 characters, which cut hunks in half
/// and left the gutter lying about line numbers. What the model wrote is the
/// most important thing on the screen, so it is what the row shows by default.
const ActionRow = memo(function ActionRow({ item }: { item: ConversationItem }) {
  const call = toolCallDisplay(item);
  const live = call.status === "running";
  const failed = call.status === "failed";
  const succeeded = call.status === "completed";
  const body = call.patch ? "patch" : call.verb === "run" && (call.command || call.output) ? "terminal" : call.output ? "output" : null;
  // `null` is "nobody has decided yet", which is not the same as closed: a patch
  // arriving mid-stream should still open the row, while a reader who collapsed
  // one keeps it collapsed.
  const [toggled, setToggled] = useState<boolean | null>(null);
  const open = (toggled ?? !!call.patch) && !!body;
  // Reads and searches earn a flat row; writes and commands earn a card.
  const card = !FLAT_VERBS.has(call.verb);
  const label = `${live ? call.doing : call.done}${call.target ? ` ${call.target}` : ""}`;
  const path = call.path && call.path !== call.target ? call.path : undefined;
  // A path the workspace recognises is a link into the Code pane. It has to be
  // a sibling of the expand control, not a child — buttons do not nest.
  const links = useContext(FileLinkContext);
  const fileRef = path && (call.verb === "edit" || call.verb === "read") ? parseFileRef(path, links) : undefined;
  return (
    // No `initial`/`animate` of its own: the row inherits both from the group
    // that reveals it, which is what produces the stagger.
    <motion.div className="min-w-0" variants={ROW_VARIANTS}>
      <div className={cn("min-w-0", card && "overflow-hidden rounded-lg border border-border bg-card")}>
        <div
          className={cn(
            "group/row flex w-full min-w-0 items-center gap-2 px-3 py-1.5 font-mono text-[11px] text-muted-foreground transition-colors",
            body && "hover:bg-accent",
            !card && "rounded-lg",
          )}
        >
          <button
            type="button"
            className="flex min-w-0 shrink-0 items-center gap-2 text-left disabled:cursor-default"
            disabled={!body}
            onClick={() => body && setToggled(!open)}
          >
            <span className="shrink-0 text-muted-foreground/70" aria-hidden="true">{TOOL_ICON[call.glyph]}</span>
            <span className={cn("truncate", live && "text-foreground")}>{label}</span>
          </button>
          {/* flex-1 from a zero basis, so the path gives up room before the label does. */}
          {path && (fileRef
            ? <button
                type="button"
                onClick={() => links!.open(fileRef.path, fileRef.line)}
                aria-label={`Open ${fileRef.path} in the Code pane`}
                title={`Open ${fileRef.path} in the Code pane`}
                className="hidden min-w-0 flex-1 truncate text-left text-faint decoration-dotted underline-offset-2 hover:text-foreground hover:underline sm:block"
              >{path}</button>
            : <span className="hidden min-w-0 flex-1 truncate text-faint sm:block">{path}</span>)}
          <button
            type="button"
            className="ml-auto flex shrink-0 items-center gap-2 disabled:cursor-default"
            disabled={!body}
            aria-label={open ? "Collapse tool output" : "Expand tool output"}
            onClick={() => body && setToggled(!open)}
          >
            {call.verb === "edit" && call.additions !== undefined && (
              <span><b className="font-medium text-success">+{call.additions}</b> <b className="font-medium text-destructive">−{call.deletions ?? 0}</b></span>
            )}
            {call.exitCode !== undefined && <ExitChip code={call.exitCode}/>}
            {call.durationMs !== undefined && !live && <span className="text-faint">{Math.max(1, Math.round(call.durationMs / 1000))}s</span>}
            <StatusGlyph live={live} failed={failed} succeeded={succeeded}/>
            {body && <ChevronRight size={12} className={cn("text-muted-foreground/70 transition-transform", open && "rotate-90")} aria-hidden="true"/>}
          </button>
        </div>
        <Disclosure open={open} className={cn(card && "border-t border-border")}>
          {body === "patch" && <PatchView patch={call.patch ?? ""} path={call.path ?? ""} className="max-h-[420px] px-1" foldAfterHunks={1}/>}
          {body === "terminal" && <TerminalBlock command={call.command} output={call.output}/>}
          {body === "output" && (looksLikeDiff(call.output ?? "")
            ? <PatchView patch={call.output ?? ""} path={call.path ?? ""} className="max-h-[320px] px-1" foldAfterHunks={2}/>
            : <CappedOutput text={call.output ?? ""} className="max-h-[320px] overflow-auto whitespace-pre-wrap break-words bg-code p-3 font-mono text-[11.5px] leading-relaxed text-muted-foreground"/>)}
        </Disclosure>
      </div>
    </motion.div>
  );
}, (previous, next) => sameItem(previous.item, next.item));

/// A run of consecutive rows that belong together: exploration under one label,
/// a thought where the model paused to think, everything else on its own.
/// *Consecutive*, never sorted — reordering the transcript to tidy it would
/// destroy the one thing it is for.
type ActionChunk =
  | { kind: "explored"; key: string; items: ConversationItem[] }
  | { kind: "row"; key: string; item: ConversationItem }
  /// A thought or a plan update that fell inside the run. Drawn by the same
  /// components the top level uses; a run is not a different kind of thinking.
  | { kind: "note"; key: string; item: ConversationItem };

function chunkActions(items: ConversationItem[]): ActionChunk[] {
  const out: ActionChunk[] = [];
  for (const item of items) {
    if (!isToolItem(item)) {
      out.push({ kind: "note", key: item.key, item });
      continue;
    }
    if (!FLAT_VERBS.has(toolCallDisplay(item).verb)) {
      out.push({ kind: "row", key: item.key, item });
      continue;
    }
    const last = out[out.length - 1];
    if (last?.kind === "explored") { last.items.push(item); continue; }
    out.push({ kind: "explored", key: `explored-${item.key}`, items: [item] });
  }
  return out;
}

/// A run short enough to take in at a glance opens itself when it carries a
/// patch. What the model wrote is the most important thing on the screen, and a
/// diff the reader has to dig for twice is not an inline diff. Past this the
/// run is a flood, and a flood that opens itself is the defect this bound
/// exists to prevent.
const SELF_OPENING_STEPS = 3;

/// A turn's tool work as one row: what it did, how many steps it took, and —
/// only if the reader asks — every call in order.
///
/// Collapsed by default, and it stays that way when the run finishes. The old
/// behaviour latched a group open the moment it was ever live, which is
/// pleasant for a three-step turn and unusable for a hundred-step one: the
/// reader came back to a wall of a hundred open cards and no turn. Live, the
/// group names the step running right now, which is the one thing worth
/// watching; finished, it is a single line. A click is what opens it, and that
/// click sticks — through the rest of the run and past the moment it ends.
const ActivityGroup = memo(function ActivityGroup({ items }: { items: ConversationItem[] }) {
  const tools = useMemo(() => items.filter(isToolItem), [items]);
  // Live is a claim about the *work*, not about the transcript: a thought left
  // streaming by a provider that never settles it must not keep a finished run
  // spinning forever.
  const live = tools.some(item => item.status === "inProgress" || item.status === "streaming");
  const glance = tools.length <= SELF_OPENING_STEPS && tools.some(item => !!toolCallDisplay(item).patch);
  // `null` is "nobody has decided yet", which is not the same as closed.
  const [toggled, setToggled] = useState<boolean | null>(null);
  const expanded = toggled ?? glance;
  // Rows revealed together arrive one after another at the same 40ms cadence the
  // CSS entrance used, so an expanding group unfolds instead of appearing whole.
  const stagger = useMotionStagger();
  const chunks = useMemo(() => chunkActions(items), [items]);
  // One collapsed line for the whole run: the summary, then a mono step count on
  // the right. The count is the number of tool calls folded away, faint because
  // it is a measure of the work rather than the work itself.
  const stepCount = tools.length;
  // What the run is doing right now, for a reader watching it work. One line,
  // not the whole timeline: the point of collapsing is that the tail is where
  // the news is.
  const current = live ? toolCallDisplay(tools[tools.length - 1]) : undefined;
  // Wall-clock the run occupied, not the sum of call durations: overlapping
  // tool calls would otherwise be counted twice. Each item contributes the
  // window [start, start+duration]; the trailer reports the union's span, which
  // also folds in the reasoning gaps between calls. Falls back to nothing when
  // the projection carries no timestamps, rather than showing a wrong number.
  const spans = tools
    .map(item => {
      const start = item.createdAt ? Date.parse(item.createdAt) : NaN;
      return { start, end: start + (toolCallDisplay(item).durationMs ?? 0) };
    })
    .filter(span => Number.isFinite(span.start));
  const workedMs = spans.length ? Math.max(...spans.map(s => s.end)) - Math.min(...spans.map(s => s.start)) : 0;
  return (
    <div className="my-2.5 min-w-0">
      <button
        type="button"
        className="group flex w-full items-center gap-2 rounded-[7px] px-1 py-1 text-left text-[12px] text-muted-foreground transition-colors hover:text-foreground"
        aria-expanded={expanded}
        onClick={() => setToggled(!expanded)}
      >
        {live ? <PulseDot size={7}/> : <Check size={12} className="shrink-0 text-faint" aria-hidden="true"/>}
        <span className={cn("min-w-0 truncate", live && "text-foreground")}>{summarize(items, live)}</span>
        <span className="ml-auto shrink-0 font-mono text-[11px] tabular-nums text-faint">{stepCount} step{stepCount === 1 ? "" : "s"}</span>
        <ChevronDown size={13} className={cn("shrink-0 text-faint transition-transform", expanded && "rotate-180")} aria-hidden="true"/>
      </button>
      {/* Collapsed and still working: the step running right now, and nothing
          else. A reader watching a run wants the head of it, not its history. */}
      {current && !expanded && (
        <div className="flex min-w-0 items-center gap-2 pl-1 font-mono text-[11px] text-faint">
          <span className="shrink-0" aria-hidden="true">{TOOL_ICON[current.glyph]}</span>
          <span className="min-w-0 truncate">{current.doing}{current.target ? ` ${current.target}` : ""}</span>
          <StatusGlyph live failed={false} succeeded={false}/>
        </div>
      )}
      <Disclosure open={expanded}>
        <motion.div
          className="mt-1 grid min-w-0 gap-1.5"
          initial="hidden"
          animate="shown"
          variants={{ hidden: {}, shown: {} }}
          transition={stagger}
        >
          {chunks.map(chunk => chunk.kind === "row"
            ? <ActionRow key={chunk.key} item={chunk.item}/>
            : chunk.kind === "note"
            ? <div key={chunk.key} className="min-w-0">
                {chunk.item.type === "plan" ? <PlanCard item={chunk.item}/> : <Reasoning item={chunk.item}/>}
              </div>
            : <div key={chunk.key} className="min-w-0">
                <GroupLabel>Explored</GroupLabel>
                <div className="grid min-w-0 gap-0.5">
                  {chunk.items.map(item => <ActionRow key={item.key} item={item}/>)}
                </div>
              </div>)}
        </motion.div>
      </Disclosure>
      {/* The trailer a finished run leaves behind: how long the work took, stated
          the way the transcript states everything quiet — faint, mono, at rest. */}
      {!live && workedMs > 0 && (
        <div className="mt-1 flex items-center gap-1 pl-1 font-mono text-[11px] text-faint">
          <span>Worked for {formatThoughtDuration(workedMs)}</span>
          <ChevronRight size={12} className="text-faint" aria-hidden="true"/>
        </div>
      )}
    </div>
  );
  // The reducer rebuilds every item on every fold, so reference equality would
  // never hold and a live turn would re-render all hundred rows on every 50ms
  // flush. The signature says which rows a frame actually touched.
}, (previous, next) => sameItems(previous.items, next.items));

/// Variants an `ActionRow` inherits from the group that reveals it. Declared
/// once so the stagger and the row agree on what "hidden" means.
const ROW_VARIANTS = {
  hidden: { opacity: 0, y: 4 },
  shown: { opacity: 1, y: 0 },
};

/* ── Cold-start narration ─────────────────────────────────────────────────
   A self-contained status row: it subscribes to `session-startup` for its own
   session id and drives its own elapsed clock, so it needs nothing from
   `App.tsx` beyond the props this component already takes. */

/// Ties the pure narration computation in `startupNarration.ts` to the live
/// `session-startup` subscription and a tick clock. Resets whenever the
/// session id changes, so switching chats never carries over a stale phase.
function useStartupNarration({ sessionId, harness, model, switchingToLabel, hasPendingWork, streaming }: {
  sessionId?: string;
  harness?: string | null;
  model?: string | null;
  switchingToLabel: string | null;
  hasPendingWork: boolean;
  streaming: boolean;
}): NarrationView {
  const reducedMotion = useReducedMotion() ?? false;
  const [phase, setPhase] = useState<SessionStartupPhase | null>(null);
  const [startedAt, setStartedAt] = useState<number | null>(null);
  const [streamStartedAt, setStreamStartedAt] = useState<number | null>(null);
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    setPhase(null);
    setStartedAt(null);
    setStreamStartedAt(null);
  }, [sessionId]);

  useEffect(() => {
    if (!sessionId) return;
    let live = true;
    let off: (() => void) | undefined;
    void bridgeApi.onSessionStartup(payload => {
      if (!live || payload.sessionId !== sessionId) return;
      setPhase(payload.phase);
    }).then(unlisten => {
      if (!live) { unlisten(); return; }
      off = unlisten;
    });
    return () => { live = false; off?.(); };
  }, [sessionId]);

  useEffect(() => {
    // A model switch runs with no pending message, so it starts the clock too.
    if (hasPendingWork || switchingToLabel) { setStartedAt(value => value ?? Date.now()); return; }
    setStartedAt(null);
    setStreamStartedAt(null);
    setPhase(null);
  }, [hasPendingWork, switchingToLabel]);

  useEffect(() => {
    if (streaming) setStreamStartedAt(value => value ?? Date.now());
  }, [streaming]);

  // The only reason to keep re-rendering while idle: the elapsed counter and
  // the collapse-after-first-token timer both read the clock.
  useEffect(() => {
    if (!hasPendingWork && !switchingToLabel) return;
    const id = window.setInterval(() => setNow(Date.now()), 250);
    return () => window.clearInterval(id);
  }, [hasPendingWork, switchingToLabel]);

  return computeNarration({
    hasPendingWork,
    streaming,
    harnessName: harnessLabel(harness),
    // A session on its adapter's default model stores no model id, and
    // `modelLabel` renders that absence as an em dash — which would read as
    // "— is reading your message…". Name the harness instead.
    modelName: model ? modelLabel(model) : harnessLabel(harness),
    switchingToLabel,
    latestPhase: phase,
    startedAt,
    streamStartedAt,
    now,
    reducedMotion,
  });
}

/// The row itself: the harness's own mark, the elapsed seconds, the label —
/// in that order, so the eye lands on *which agent* before it reads *what it is
/// doing*.
///
/// `view.mounted` gates presence entirely, and the `HarnessMark` is rendered
/// unconditionally within that window; only the label/elapsed siblings mount and
/// unmount, so the collapse handoff never remounts the animation. The counter is
/// `tabular-nums` because a second ticking over must not reflow the label beside
/// it.
function StartupStatusRow({ view, harness }: { view: NarrationView; harness?: string | null }) {
  if (!view.mounted) return null;
  return (
    <div className="flex items-center gap-2">
      <HarnessMark harness={harness} live={!view.reducedMotion}/>
      {!view.collapsed && <span className="min-w-0 truncate text-[12px] font-medium">
        {/* Dimmer than the label: the counter is metadata, the label is the news. */}
        {view.showElapsed && <span className="text-muted-foreground/70 tabular-nums">{view.elapsedSeconds}s · </span>}
        <span className="text-shimmer">{view.label}</span>
      </span>}
    </div>
  );
}

function StallNotice({ onStop }: { onStop?: () => void }) {
  const [keepWaiting, setKeepWaiting] = useState(false);
  if (keepWaiting) return null;
  return (
    <div role="status" className={`${NOTICE} border-l-warning`}>
      <p>This turn has gone quiet. Stop it, or keep waiting.</p>
      <div className="mt-2 flex flex-wrap gap-2">
        <button type="button" className={BTN_SECONDARY} onClick={() => onStop?.()}>Stop</button>
        <button type="button" className={BTN_SECONDARY} onClick={() => setKeepWaiting(true)}>Keep waiting</button>
      </div>
    </div>
  );
}

/* ── Conversation ───────────────────────────────────────────────────────── */

export const AgentConversation = memo(function AgentConversation({ session, events = [], forestEntries, activeLeafId, repositoryDivergence, completion, continuationFidelity, workers, now, onResolve, onAnswerQuestion = async () => undefined, onOpenSession, onExpandWorker, onWaiveCompletion, onRefreshBase, onRetryWorker, onRetryCompaction, pendingAdoptions = [], onResolveAdoption, preview, working, pendingMessages = [], pendingAttachments = [], highlightEntryId, onRemember, workspaceFiles, onOpenFile, projectName, modelSwitch, onInterrupt, stopping }: { session?: Session; projectName?: string; events?: AgentEvent[]; forestEntries?: SessionEntry[]; activeLeafId?: string | null; repositoryDivergence?: string; completion?: CompletionSummary | null; continuationFidelity?: ContinuationFidelity; workers?: WorkerPanelSource; now?: number; onResolve: ResolvePermission; onAnswerQuestion?: ResolveQuestion; onOpenSession?: (sessionId: string) => void; onExpandWorker?: (sessionId: string) => void; onWaiveCompletion?: (attemptId: string, checkIds: string[], reason: string) => Promise<void>; onRefreshBase?: () => Promise<void>; onRetryWorker?: (childSessionId: string) => Promise<void>; onRetryCompaction?: () => Promise<void>; pendingAdoptions?: WorkerRepositoryBinding[]; onResolveAdoption?: (childSessionId: string, decision: "adopt" | "discard") => Promise<void>; preview?: boolean; working?: boolean; pendingMessages?: string[]; pendingAttachments?: string[]; highlightEntryId?: string | null; onRemember?: (text: string) => void; workspaceFiles?: readonly string[]; onOpenFile?: (path: string, line?: number) => void; modelSwitch?: { harness: string; label: string } | null; onInterrupt?: () => void; stopping?: boolean }) {
  const visibleItems = useMemo(() => {
    const durableItems = forestEntries?.length ? projectSessionConversation(forestEntries, activeLeafId ?? null) : [];
    const nextLiveItems = reduceConversation(events);
    const items = mergeConversationProjections(durableItems, nextLiveItems);
    // Folded after the merge, not inside either projection: mid-run the spawn is
    // already durable while the result is still only live.
    return foldWorkerDelegations(items.filter(item => item.type !== "raw"));
  }, [activeLeafId, events, forestEntries]);
  const renderedItems = useMemo(() => groupItems(visibleItems), [visibleItems]);

  // Every file name in the transcript resolves against this one set; without
  // an opener the transcript renders exactly as before.
  const fileLinks = useMemo<FileLinks | null>(() => {
    if (!onOpenFile || !workspaceFiles?.length) return null;
    const paths = new Set(workspaceFiles);
    return { has: path => paths.has(path), open: onOpenFile };
  }, [workspaceFiles, onOpenFile]);

  const streaming = visibleItems.some(item => item.status === "streaming" || item.status === "inProgress");
  // Hooks run unconditionally, ahead of the early returns below: the row
  // itself only renders past them, but its state still has to track every
  // render this component makes.
  const startupNarration = useStartupNarration({
    sessionId: session?.id,
    harness: session?.harness,
    model: session?.model,
    switchingToLabel: modelSwitch?.label ?? null,
    hasPendingWork: !!working || pendingMessages.length > 0,
    streaming,
  });
  if (!session && !preview) return <Empty title="No chat yet" copy="Start a chat from the sidebar, or open a workspace agent."/>;
  if (!visibleItems.length && !working && !modelSwitch && !pendingMessages.length && !completion && !pendingAdoptions.length && repositoryDivergence !== "diverged" && continuationFidelity !== "projected_at_boundary" && continuationFidelity !== "projected_mid_turn") return <GreetingEmpty seed={session?.id ?? session?.workspaceId ?? undefined} projectName={projectName} />;
  const existingUserTexts = new Set(visibleItems.filter(item => item.type === "message" && item.role === "user").map(item => item.text.trim()));
  // An image-only send has no words yet — its optimistic row is the image, so
  // an empty-text row would render as a blank bubble.
  const optimistic = pendingMessages.filter(text => text.trim().length > 0 && !existingUserTexts.has(text.trim()));
  const errorContext = { provider: providerLabel(session?.harness), snapshot: latestUsageSnapshot(events) };
  // Content-addressed, occurrence-counted keys for the optimistic bubbles: when
  // an earlier pending message lands as a real message, the bubbles after it
  // keep their identity — one ghost fades, and no survivor flips its text.
  const seenPending = new Map<string, number>();
  const pendingRows = optimistic.map(text => {
    const occurrence = seenPending.get(text) ?? 0;
    seenPending.set(text, occurrence + 1);
    return { key: `pending-${text}:${occurrence}`, text };
  });
  // Text and images of one send share a single bubble, matching the persisted
  // user row. Image-only sends still get that same bubble with no prose.
  const pendingAttachmentRows = pendingAttachments.map((dataUri, index) => ({ key: `pending-attachment-${index}`, dataUri }));
  const optimisticBubbles = pendingRows.length
    ? pendingRows.map((row, index) => ({
        key: row.key,
        text: row.text,
        attachments: index === pendingRows.length - 1 ? pendingAttachmentRows.map(row => row.dataUri) : [],
      }))
    : pendingAttachmentRows.length
      ? [{ key: pendingAttachmentRows[0].key, text: "", attachments: pendingAttachmentRows.map(row => row.dataUri) }]
      : [];
  const lastEventAt = events.length ? new Date(events[events.length - 1]?.createdAt ?? 0).getTime() : 0;
  const clock = now ?? Date.now();
  const stalled = !!working && !stopping && !streaming && lastEventAt > 0 && clock - lastEventAt > 45_000;
  const tailLength = visibleItems.length ? visibleItems[visibleItems.length - 1].text.length : 0;
  const scrollSignature = `${visibleItems.length}:${tailLength}:${optimistic.length}:${working ? 1 : 0}`;
  return <FileLinkContext.Provider value={fileLinks}><ScrollFollow signature={scrollSignature} className="absolute inset-0 overflow-y-auto overscroll-y-none scroll-smooth px-3 py-8 pb-24 sm:px-6 sm:py-10">
    <div className="mx-auto flex w-full min-w-0 max-w-3xl flex-col gap-6 sm:gap-8">
      {pendingAdoptions.map(binding => <AdoptionCard key={binding.sessionId} binding={binding} onResolve={onResolveAdoption}/>)}
      {completion && <VerificationCard summary={completion} onWaive={onWaiveCompletion}/>}
      {repositoryDivergence === "diverged" && <div role="alert" className={`${NOTICE} border-l-warning`}>This branch&apos;s context predates the current file state.</div>}
      {continuationFidelity === "projected_at_boundary" && <div role="status" className={`${NOTICE} border-l-info`}>Continuation restored from a phase-boundary projection; provider reasoning state was not transferred.</div>}
      {continuationFidelity === "projected_mid_turn" && <div role="alert" className={`${NOTICE} border-l-warning`}>Continuation fidelity degraded: context was projected mid-turn and provider reasoning state was lost.</div>}
      {preview && <div className="w-fit mx-auto mb-[22px] px-2.5 py-1 border border-dashed border-border rounded-full text-muted-foreground text-[10.5px] tracking-[0.04em]">Design preview — sample conversation</div>}
      {/* `initial={false}`: the rows already on screen when a session opens must
          not replay their entrance. Only what actually arrives afterwards rises
          into place — which is the difference between a transcript that breathes
          and one that flashes on every switch. */}
      {/* Keyed by session: a switch replaces the whole tree in one commit.
          Unkeyed, every old row's exit played at once — the scroll height
          doubled against `ScrollFollow` and hundreds of rows animated
          simultaneously on a long transcript. Within one session, genuine
          removals (a resolved optimistic bubble, the working shimmer) still
          get their exit. */}
      <AnimatePresence initial={false} key={session?.id ?? "preview"}>
        {renderedItems.map(entry => entry.kind === "group"
          ? <TranscriptRow key={entry.key}><ActivityGroup items={entry.items}/></TranscriptRow>
          : entry.kind === "raw-group" ? <TranscriptRow key={entry.key}><RawEventGroup items={entry.items}/></TranscriptRow>
          : <TranscriptRow
              key={entry.item.key}
              tone={entry.item.type === "error" || entry.item.status === "failed" ? "alert" : "quiet"}
              id={entry.item.entryId ? `forest-entry-${entry.item.entryId}` : undefined}
              entryId={entry.item.entryId}
              className={highlightEntryId && entry.item.entryId === highlightEntryId ? "rounded-xl bg-accent/60 ring-1 ring-ring/70" : undefined}
            >
              <ItemView item={entry.item} workers={workers} now={now} onResolve={onResolve} onAnswerQuestion={onAnswerQuestion} onOpenSession={onOpenSession} onExpandWorker={onExpandWorker} onRefreshBase={onRefreshBase} onRetryWorker={onRetryWorker} onRetryCompaction={onRetryCompaction} onRemember={onRemember} errorContext={errorContext}/>
            </TranscriptRow>)}
        {optimisticBubbles.map(bubble => <TranscriptRow key={bubble.key}><div className={BUBBLE}>
          {bubble.text ? <MentionText text={bubble.text}/> : null}
          {bubble.attachments.length > 0 && <div className="flex flex-wrap justify-end gap-1.5 pt-1.5">
            {bubble.attachments.map((dataUri, index) => <img key={index} src={dataUri} alt={`Image you attached, still sending ${index + 1}`} className="max-h-40 rounded-xl"/>)}
          </div>}
        </div></TranscriptRow>)}
        {startupNarration.mounted && <TranscriptRow key="working"><div className="flex justify-start"><StartupStatusRow view={startupNarration} harness={modelSwitch?.harness ?? session?.harness}/></div></TranscriptRow>}
        {stopping && <TranscriptRow key="stopping"><p role="status" className={`${NOTICE} border-l-info`}>Stopping…</p></TranscriptRow>}
        {stalled && <TranscriptRow key="stalled"><StallNotice onStop={onInterrupt}/></TranscriptRow>}
      </AnimatePresence>
    </div>
  </ScrollFollow></FileLinkContext.Provider>;
});

/// Changes that exist only in a worker's own worktree. The parent session cannot
/// finish while this is unresolved, so the choice has to be reachable here.
function AdoptionCard({ binding, onResolve }: { binding: WorkerRepositoryBinding; onResolve?: (childSessionId: string, decision: "adopt" | "discard") => Promise<void> }) {
  const [busy, setBusy] = useState<"adopt" | "discard" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const settling = binding.state === "settling";
  const act = async (decision: "adopt" | "discard") => {
    if (!onResolve) return;
    setBusy(decision); setError(null);
    try { await onResolve(binding.sessionId, decision); }
    catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); }
    finally { setBusy(null); }
  };
  return <div role="alert" className={`${PANEL} border-l-warning`}>
    <header className="flex flex-wrap items-baseline gap-x-2 gap-y-1 px-3.5 pt-3 sm:px-4">
      <b className="text-[13px] font-semibold text-foreground">Worker changes are not in your workspace yet</b>
      {settling && <small className="text-warning text-[10.5px] tracking-[0.03em]">settling…</small>}
    </header>
    <p className="mt-1.5 px-3.5 text-[12.5px] leading-relaxed text-muted-foreground sm:px-4">
      This worker wrote in its own worktree. Adopting merges those changes into your checkout; discarding throws them away. Until you choose, this session stays unfinished.
    </p>
    {binding.changedPaths.length > 0 && <code className={`mt-2 mx-3.5 sm:mx-4 max-h-40 overflow-y-auto ${WELL}`}>{binding.changedPaths.join("\n")}</code>}
    <small className="mt-1 block px-3.5 font-mono text-[10.5px] break-all text-muted-foreground/70 sm:px-4">
      {binding.diffstat ?? "no diffstat"} · {binding.worktreeBranch}{binding.dirty ? " · uncommitted" : ""}
    </small>
    <small className="mt-0.5 block px-3.5 font-mono text-[10.5px] break-all text-muted-foreground/70 sm:px-4">{binding.worktreePath}</small>
    {error && <p className="mt-1.5 px-3.5 text-[12px] leading-relaxed text-destructive sm:px-4">{error}</p>}
    <div className="flex flex-wrap items-center justify-end gap-[7px] px-3.5 py-3 sm:px-4">
      <button disabled={!!busy || settling || !onResolve} className={BTN_SECONDARY} onClick={() => act("discard")}>
        <X size={12} aria-hidden="true" /> {busy === "discard" ? "Discarding…" : "Discard"}
      </button>
      <button disabled={!!busy || settling || !onResolve} className={BTN_PRIMARY} onClick={() => act("adopt")}>
        <Check size={12} aria-hidden="true" /> {busy === "adopt" ? "Adopting…" : "Adopt changes"}
      </button>
    </div>
  </div>;
}

function VerificationCard({ summary, onWaive }: { summary: CompletionSummary; onWaive?: (attemptId: string, checkIds: string[], reason: string) => Promise<void> }) {
  const [waiverOpen, setWaiverOpen] = useState(false);
  const [waiverReason, setWaiverReason] = useState("");
  const [waiving, setWaiving] = useState(false);
  const [waiverError, setWaiverError] = useState<string>();
  const unresolved = summary.checks.filter(check => check.required && check.status !== "passed");
  const failedVerdict = summary.verdict === "changes_requested" || summary.verdict === "failed";
  // Neutral card plus a colored tick; only a real failure earns a wash.
  const tone = summary.verdict === "verified" ? "border-l-success" : summary.verdict === "waived" ? "border-l-warning" : failedVerdict ? "border-l-destructive bg-destructive/5" : "border-l-info";
  const title = summary.verdict === "verified" ? "Verified" : summary.verdict === "waived" ? "Verified with waiver" : summary.verdict === "changes_requested" ? "Changes requested" : summary.verdict === "superseded" ? "Evidence superseded" : summary.verdict === "failed" ? "Verification failed" : "Verifying";
  const statusIcon = (status: string) => status === "passed" ? <Check size={12} className="mt-0.5 shrink-0 text-success" aria-hidden="true"/> : status === "failed" ? <X size={12} className="mt-0.5 shrink-0 text-destructive" aria-hidden="true"/> : status === "skipped" || status === "blocked" || status === "stale" ? <AlertTriangle size={12} className="mt-0.5 shrink-0 text-warning" aria-hidden="true"/> : <Circle size={10} className="mt-1 shrink-0 text-muted-foreground" aria-hidden="true"/>;
  return <section aria-label="Completion verification" className={`min-w-0 overflow-hidden rounded-xl border border-border border-l-2 bg-card ${tone}`}>
    <div className="flex items-start gap-3 px-3.5 py-3 sm:px-4">
      <div className="min-w-0 flex-1"><div className="flex flex-wrap items-center gap-x-2 gap-y-0.5"><strong className="font-display text-sm text-foreground">{title}</strong><span className="text-[11px] text-muted-foreground">{summary.passedRequired} of {summary.totalRequired} required checks passed</span></div><p className="mt-1 font-mono text-[10.5px] break-all text-muted-foreground">revision {summary.repository.head.slice(0, 12)} · {summary.repository.dirtyDigest === "clean" ? "clean" : `tree ${summary.repository.dirtyDigest.slice(0, 8)}`}</p></div>
      {!summary.markdownCommitted && <span className="shrink-0 rounded-full border border-border px-2 py-0.5 text-[10px] text-muted-foreground">private contract</span>}
    </div>
    <details className="border-t border-border">
      <summary className="cursor-pointer px-3.5 py-2 text-xs text-foreground marker:text-muted-foreground sm:px-4">Proof and checks</summary>
      <div className="space-y-1 border-t border-border px-3.5 py-3 sm:px-4">
        {summary.checks.map(check => <div key={check.checkId} className="flex items-start gap-2 text-xs text-muted-foreground">{statusIcon(check.status)}<div className="min-w-0 flex-1"><div className="flex flex-wrap gap-x-2"><span className="text-foreground">{check.command || check.checkId}</span><span>{humanizeCheckKind(check.kind)}</span>{check.verifierFamily && <span>· {check.verifierFamily}</span>}</div>{check.detail && <p className="mt-0.5 truncate font-mono text-[10.5px]">{check.detail}</p>}</div><span className="shrink-0 text-[10px] tracking-wide">{humanizeCheckStatus(check.status)}</span></div>)}
        {summary.waiverReason && <p className="mt-2 rounded-md border border-border border-l-2 border-l-warning bg-background px-2 py-1.5 text-xs text-warning">Waiver: {summary.waiverReason}</p>}
        {onWaive && unresolved.length > 0 && !["verified", "waived", "superseded"].includes(summary.verdict) && <div className="pt-2">
          {!waiverOpen ? <button type="button" onClick={() => setWaiverOpen(true)} className="rounded-full border border-input px-2.5 py-1.5 text-xs font-medium text-warning transition-colors hover:bg-accent">Waive unresolved checks</button> : <form onSubmit={event => { event.preventDefault(); const reason = waiverReason.trim(); if (!reason) { setWaiverError("Explain why these checks can be waived."); return; } setWaiving(true); setWaiverError(undefined); void onWaive(summary.attemptId, unresolved.map(check => check.checkId), reason).then(() => { setWaiverOpen(false); setWaiverReason(""); }).catch(error => setWaiverError(error instanceof Error ? error.message : String(error))).finally(() => setWaiving(false)); }} className="space-y-2 rounded-lg border border-border border-l-2 border-l-warning bg-background p-2.5">
            <p className="text-xs text-warning">This records human-approved risk for: {unresolved.map(check => check.command || check.checkId).join(", ")}. It remains distinct from Verified.</p>
            <textarea autoFocus value={waiverReason} onChange={event => setWaiverReason(event.target.value)} rows={2} placeholder="Reason for waiver" aria-label="Waiver reason" className="w-full resize-none rounded-md border border-input bg-card px-2.5 py-2 text-xs text-foreground placeholder:text-muted-foreground"/>
            {waiverError && <p role="alert" className="text-xs text-destructive">{waiverError}</p>}
            <div className="flex flex-wrap gap-2"><button type="submit" disabled={waiving} className="rounded-full bg-warning px-2.5 py-1.5 text-xs font-semibold text-warning-foreground disabled:opacity-50">{waiving ? "Recording…" : `Waive ${unresolved.length} check${unresolved.length === 1 ? "" : "s"}`}</button><button type="button" disabled={waiving} onClick={() => { setWaiverOpen(false); setWaiverError(undefined); }} className="rounded-full border border-input px-2.5 py-1.5 text-xs text-muted-foreground disabled:opacity-50">Cancel</button></div>
          </form>}
        </div>}
      </div>
    </details>
  </section>;
}

function ScrollFollow({ signature, className, children }: { signature: string; className?: string; children: React.ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  const pinned = useRef(true);
  useEffect(() => {
    const el = ref.current;
    if (el && pinned.current) el.scrollTop = el.scrollHeight;
  }, [signature]);
  return <div ref={ref} className={className} onScroll={event => {
    const el = event.currentTarget;
    pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
  }}>{children}</div>;
}

function PulseDot({ size = 8 }: { size?: number }) {
  return <span className="inline-block flex-none rounded-full bg-muted-foreground/60 animate-[thinking-pulse_1.6s_ease-in-out_infinite]" style={{ width: size, height: size }} aria-hidden="true" />;
}

function GreetingEmpty({ seed, projectName }: { seed?: string; projectName?: string }) {
  // Stable per session so it doesn't reshuffle on every re-render, tinted by
  // time of day. A known project name lets the hero name it, dotted-underlined.
  const greeting = useMemo(() => pickGreeting(seed, projectName), [seed, projectName]);
  return <Empty title={greeting.headline} copy={greeting.hint} parts={greeting.parts} />;
}

function Empty({ title, copy, parts }: { title: string; copy: string; parts?: GreetingPart[] }) {
  return <div className="absolute inset-0 flex flex-col items-center justify-center px-4 text-center animate-page-enter sm:px-6">
    <div className="flex w-full max-w-[440px] flex-col items-center">
      <h2 className="font-display text-[26px] font-medium leading-[1.15] tracking-[-0.02em] text-foreground">
        {parts && parts.length > 1
          ? parts.map((part, index) => part.kind === "project"
            ? <span key={index} className="underline decoration-dotted decoration-muted-foreground/60 underline-offset-[6px]">{part.text}</span>
            : <span key={index}>{part.text}</span>)
          : title}
      </h2>
      <p className="mt-2.5 max-w-[380px] text-[13.5px] leading-relaxed tracking-[-0.004em] text-muted-foreground">{copy}</p>
    </div>
  </div>;
}

/// Prose, from either side of the conversation.
///
/// Memoized on the row's own signature rather than on object identity: a live
/// turn hands the transcript a freshly folded copy of every item twenty times a
/// second, and a settled message that re-renders on each of them is most of
/// what made a hundred-step turn stop responding. Streaming prose still
/// re-renders on every chunk, because its text length moves.
const MessageRow = memo(function MessageRow({ item, onRemember }: { item: ConversationItem; onRemember?: (text: string) => void }) {
  if (item.role === "user") {
    const attachments = attachmentUris(item.data);
    return <div className={BUBBLE}>
      <MentionText text={item.text}/>
      {attachments.length > 0 && <div className="flex flex-wrap justify-end gap-1.5 pt-1.5">
        {attachments.map((dataUri, index) => <img key={index} src={dataUri} alt={`Attached image ${index + 1}`} className="max-h-40 rounded-xl"/>)}
      </div>}
    </div>;
  }
  // No bubble, no card: the agent writes straight onto the canvas, in body
  // ink a step under `foreground` so prose reads as text rather than chrome.
  return <div className="group w-full min-w-0 text-[14px] text-body">
    {item.status === "streaming" && !item.text.trim() ? <div className="thinking-shimmer h-[2px] w-16 rounded-full" /> : <Markdown text={item.text} dim={item.status === "streaming"} />}
    {onRemember && item.status !== "streaming" && item.text.trim() !== "" && (
      <button
        type="button"
        aria-label="Remember this"
        title="Remember this"
        className="mt-1 inline-flex items-center gap-1.5 rounded-md px-1.5 py-0.5 text-[11px] text-muted-foreground opacity-0 transition-opacity hover:bg-accent hover:text-foreground focus-visible:opacity-100 group-hover:opacity-100"
        onClick={() => onRemember(item.text)}
      ><Pin size={12} aria-hidden="true" />Remember this</button>
    )}
  </div>;
}, (previous, next) => previous.onRemember === next.onRemember && sameItem(previous.item, next.item));

function ItemView({ item, workers, now, onResolve, onAnswerQuestion, onOpenSession, onExpandWorker, onRefreshBase, onRetryWorker, onRetryCompaction, onRemember, errorContext }: { item: ConversationItem; workers?: WorkerPanelSource; now?: number; onResolve: ResolvePermission; onAnswerQuestion: ResolveQuestion; onOpenSession?: (sessionId: string) => void; onExpandWorker?: (sessionId: string) => void; onRefreshBase?: () => Promise<void>; onRetryWorker?: (childSessionId: string) => Promise<void>; onRetryCompaction?: () => Promise<void>; onRemember?: (text: string) => void; errorContext?: { provider?: string; snapshot: UsageSnapshot | null } }) {
  if (item.type === "message") return <MessageRow item={item} onRemember={onRemember}/>;
  if (item.data.staleBase === true) return <StaleBaseCard item={item} onRefresh={onRefreshBase}/>;
  if (item.type === "reasoning") return <Reasoning item={item}/>;
  if (item.type === "plan") return <PlanCard item={item}/>;
  if (item.type === "approval") return <ApprovalCard item={item} onResolve={onResolve}/>;
  if (item.type === "permission") return <PermissionCard item={item} onResolve={onResolve}/>;
  if (item.type === "question") return <QuestionCard item={item} onResolve={onAnswerQuestion}/>;
  if (item.type === "delegation") return <DelegationRow item={item} workers={workers} now={now} onOpenSession={onOpenSession} onExpandWorker={onExpandWorker} onRetryWorker={onRetryWorker}/>;
  if (item.type === "checkpoint" || item.type === "compaction" || item.type === "branch-summary") return <ForestCard item={item} onRetryCompaction={onRetryCompaction}/>;
  if (item.data.freshProviderSession === true) return <ModelChangedRow item={item}/>;
  if (item.type === "raw") return <RawEvent item={item}/>;
  if (item.type === "error") return <ErrorCard item={item} errorContext={errorContext}/>;
  return <ActivityGroup items={[item]}/>;
}

/// A failure, stated plainly.
///
/// The row it sits in already rises further than an ordinary one (see
/// `TranscriptRow`'s alert tone). The only motion added here is the tick: it
/// arrives a beat after the card, so the eye is drawn to the mark that says
/// *what kind* of interruption this is. No shake — a graphite-and-paper
/// transcript should not flinch.
function ErrorCard({ item, errorContext }: { item: ConversationItem; errorContext?: { provider?: string; snapshot: UsageSnapshot | null } }) {
  const described = describeError(item.text, errorContext);
  const isUsage = described.kind === "usage-limit";
  const transition = useMotionTransition(MOTION_DURATION.tick, MOTION_DURATION.reveal);
  // A rate limit is a wait, not a failure — it gets the tick. A real error
  // is the one place a full wash is warranted.
  return <div role="alert" className={`my-4 flex min-w-0 gap-2.5 rounded-lg border border-l-2 p-3 ${isUsage ? "border-border border-l-warning bg-card text-warning" : "border-destructive/30 border-l-destructive bg-destructive/5 text-destructive"}`}>
    <motion.span
      className="mt-0.5 flex shrink-0"
      initial={{ opacity: 0, scale: 0.6 }}
      animate={{ opacity: 1, scale: 1 }}
      transition={transition}
    >
      {isUsage ? <Gauge size={14} aria-hidden="true" /> : <AlertTriangle size={14} aria-hidden="true" />}
    </motion.span>
    <div className="min-w-0"><b className="text-[12px]">{described.title}</b><p className="mt-1 text-[12px] leading-relaxed break-words text-muted-foreground">{described.message}</p></div>
  </div>;
}

/// A model switch, as a milestone the transcript reads past: one hairline
/// with the transition inline, in the same register as a group label. The
/// carried-context sentence lives in `title`-adjacent text and the payload;
/// the row keeps only the fact of the change.
function ModelChangedRow({ item }: { item: ConversationItem }) {
  const side = (harness: unknown, model: unknown) => [
    harness ? harnessLabel(String(harness)) : undefined,
    model ? modelLabel(String(model)) : undefined,
  ].filter(Boolean).join(" · ");
  const from = side(item.data.previousHarness, item.data.previousModel);
  const to = side(item.data.harness, item.data.model);
  return <div className="my-3 flex items-center gap-2 font-mono text-[10.5px] uppercase tracking-[0.08em] text-muted-foreground/70">
    <span className="h-px flex-1 bg-border" aria-hidden="true"/>
    <span className="shrink-0 normal-case tracking-normal">{from && to ? `${from} → ${to}` : item.title || "Model changed"}</span>
    <span className="h-px flex-1 bg-border" aria-hidden="true"/>
  </div>;
}

function ForestCard({ item, onRetryCompaction }: { item: ConversationItem; onRetryCompaction?: () => Promise<void> }) {
  const [retrying, setRetrying] = useState(false);
  const retryingRef = useRef(false);
  const [retryAccepted, setRetryAccepted] = useState(false);
  const [retryError, setRetryError] = useState<string>();
  const label = item.type === "checkpoint" ? "Checkpoint" : item.type === "compaction" ? "Context" : "Branch";
  const retryable = item.type === "compaction" && item.status === "failed" && item.data.retryable === true && !!onRetryCompaction;
  const recoveryAction = typeof item.data.recoveryAction === "string" ? item.data.recoveryAction : undefined;
  const retry = async () => {
    if (!retryable || retryingRef.current) return;
    retryingRef.current = true;
    setRetrying(true);
    setRetryError(undefined);
    try { await onRetryCompaction(); setRetryAccepted(true); }
    catch (cause) { setRetryError(cause instanceof Error ? cause.message : String(cause)); }
    finally { retryingRef.current = false; setRetrying(false); }
  };
  return <div role={item.status === "failed" ? "alert" : undefined} className={`my-2 min-w-0 px-3 py-2.5 border border-border rounded-lg bg-card ${item.type}`}>
    <header className="flex gap-2 items-center"><GitFork size={13} className="shrink-0" aria-hidden="true" /><b className="min-w-0 truncate text-foreground">{item.title || label}</b><small className="ml-auto shrink-0 text-muted-foreground">{item.status || "durable"}</small></header>
    {/* The former raw reason block only repeated the primary copy and exposed
        protocol diagnostics. Failure details now remain in the inspector data
        while the card renders the backend's classified message. */}
    {item.text && <p className="mt-2 text-muted-foreground text-[12px]">{item.text}</p>}
    {recoveryAction && <p className="mt-1.5 text-muted-foreground text-[11px] leading-relaxed">{recoveryAction}</p>}
    {retryError && <p className="mt-1.5 text-destructive text-[11px] leading-relaxed">{retryError}</p>}
    {retryable && <div className="mt-2.5 flex justify-end">
      <button type="button" disabled={retrying || retryAccepted} className={BTN_SECONDARY} onClick={() => void retry()}>
        {retrying ? <LoaderCircle size={12} className="animate-spin" aria-hidden="true"/> : retryAccepted ? <Check size={12} aria-hidden="true"/> : <RotateCcw size={12} aria-hidden="true"/>}
        {retrying ? "Retrying…" : retryAccepted ? "Retry requested" : "Retry compaction"}
      </button>
    </div>}
  </div>;
}

function RawEvent({ item }: { item: ConversationItem }) {
  return <details className="my-2 min-w-0 p-2 border border-border rounded-lg bg-card group [&_summary::-webkit-details-marker]:hidden">
    <summary className="flex items-center gap-[7px] cursor-pointer text-[11px] text-muted-foreground hover:text-foreground transition-colors"><SquareTerminal size={12} className="shrink-0" aria-hidden="true" /><span className="min-w-0 flex-1 truncate">{item.title || "Raw provider event"}</span><small className="shrink-0 text-muted-foreground group-open:hidden">inspect</small></summary>
    <pre className="max-h-[220px] overflow-auto mt-1.5 p-2 rounded-md border border-border bg-code font-mono text-[10px] whitespace-pre-wrap break-words text-muted-foreground">{JSON.stringify(item.data, null, 2)}</pre>
  </details>;
}

function RawEventGroup({ items }: { items: ConversationItem[] }) {
  return <details className="my-2 min-w-0 p-2 border border-border rounded-lg bg-card group [&_summary::-webkit-details-marker]:hidden">
    <summary className="flex items-center gap-[7px] cursor-pointer text-[11px] text-muted-foreground hover:text-foreground transition-colors"><SquareTerminal size={12} className="shrink-0" aria-hidden="true" /><span className="min-w-0 flex-1 truncate">{items.length} raw provider event{items.length === 1 ? "" : "s"}</span><small className="shrink-0 text-muted-foreground group-open:hidden">inspect</small></summary>
    <div>{items.map(item => <RawEvent key={item.key} item={item}/>)}</div>
  </details>;
}

const Reasoning = memo(function Reasoning({ item }: { item: ConversationItem }) {
  const streaming = item.status === "streaming";
  const text = item.text || stringList(item.data.summary);
  const durationMs = typeof item.data.durationMs === "number" ? item.data.durationMs : undefined;
  const lastLine = text.split("\n").map(line => line.trim()).filter(Boolean).at(-1);
  const label = durationMs !== undefined ? `Thought for ${formatThoughtDuration(durationMs)}` : "Thought for a moment";
  if (streaming) {
    const lines = text.split("\n").map(line => line.trim()).filter(Boolean);
    return (
      <div className="my-3 flex min-w-0 items-start gap-3 rounded-xl border border-border bg-card px-3.5 py-3 sm:px-4">
        <Brain size={14} className="mt-0.5 shrink-0 text-muted-foreground animate-[thinking-pulse_1.6s_ease-in-out_infinite]" aria-hidden="true"/>
        <div className="min-w-0 flex-1">
          <span className="text-shimmer text-[12px] font-medium">Thinking…</span>
          {lines.length > 0 && <div className="mt-1.5 space-y-0.5">
            {lines.map((line, index) => <p key={index} className={`whitespace-pre-wrap break-words text-[12px] leading-relaxed ${index === lines.length - 1 ? "text-muted-foreground" : "text-muted-foreground/70"}`}>{line}</p>)}
          </div>}
        </div>
      </div>
    );
  }
  return (
    <details className="group my-3 min-w-0 rounded-xl border border-border bg-card [&_summary::-webkit-details-marker]:hidden">
      <summary className="flex cursor-pointer items-center gap-2.5 px-3.5 py-2.5 text-[12px] text-muted-foreground transition-colors hover:text-foreground sm:px-4">
        <Brain size={13} className="shrink-0 text-muted-foreground/70" aria-hidden="true"/>
        <span className="min-w-0 flex-1 truncate">
          <span className="font-medium">{label}</span>
          {lastLine && <span className="ml-2 font-normal text-muted-foreground/70">{lastLine}</span>}
        </span>
        <ChevronRight size={12} className="ml-auto shrink-0 text-muted-foreground/70 transition-transform group-open:rotate-90" aria-hidden="true"/>
      </summary>
      <div className="border-t border-border px-3.5 py-3 text-muted-foreground sm:px-4">
        <Markdown text={text}/>
      </div>
    </details>
  );
}, (previous, next) => sameItem(previous.item, next.item));

function PlanCard({ item }: { item: ConversationItem }) {
  return <div className="my-[14px] min-w-0 border border-border rounded-lg bg-card overflow-hidden">
    <header className="flex items-center gap-2 px-3.5 py-2.5 border-b border-border text-muted-foreground sm:px-4"><FileText size={13} className="shrink-0" aria-hidden="true" /><b className="text-[12px] font-medium text-foreground">{item.title || "Plan"}</b></header>
    {planSteps(item.data).map((step, index) => <div className={`min-h-[30px] flex items-center gap-[9px] py-[2px] px-3.5 text-[12.5px] sm:px-4 ${step.status === "completed" ? "text-muted-foreground/70 line-through decoration-border" : step.status === "inProgress" ? "text-foreground" : "text-muted-foreground"}`} key={`${step.step}-${index}`}>
      {step.status === "completed" ? <Check size={12} className="flex-none text-muted-foreground/70" aria-hidden="true" /> : step.status === "inProgress" ? <PulseDot size={8}/> : <Circle size={8} className="flex-none text-muted-foreground/70" aria-hidden="true" />}
      <span className="min-w-0">{step.step}</span>
    </div>)}
  </div>;
}

type PermissionAction = { decision: ApprovalDecision; optionId?: string; label: string };

function offeredPermissionActions(data: Record<string, unknown>): PermissionAction[] {
  if (!Array.isArray(data.actions)) return [];
  return data.actions.flatMap(value => {
    if (!value || typeof value !== "object") return [];
    const action = value as Record<string, unknown>;
    const decision = action.decision;
    const label = action.label;
    if (!(["accept", "acceptForSession", "decline", "cancel"] as unknown[]).includes(decision) || typeof label !== "string") return [];
    return [{ decision: decision as ApprovalDecision, optionId: typeof action.optionId === "string" ? action.optionId : undefined, label }];
  });
}

function resolutionCopy(item: ConversationItem): string {
  const actor = typeof item.data.resolvedBy === "string" ? item.data.resolvedBy : "";
  const reason = typeof item.data.reason === "string" ? item.data.reason : "";
  const labels: Record<string, string> = {
    settling: "Applying decision…",
    allowed_once: "Allowed once",
    allowed_for_session: "Allowed for this session",
    accept: "Allowed once",
    acceptForSession: "Allowed for this session",
    answered: "Answered",
    declined: "Declined",
    decline: "Declined",
    cancelled: "Cancelled",
    cancel: "Cancelled",
    failed: "Could not be delivered",
  };
  const outcome = labels[item.status ?? ""] ?? item.status ?? "Resolved";
  return [outcome, actor ? `by ${actor}` : "", reason ? `— ${reason}` : ""].filter(Boolean).join(" ");
}

function PermissionCard({ item, onResolve }: { item: ConversationItem; onResolve: ResolvePermission }) {
  const actions = offeredPermissionActions(item.data);
  const pending = item.status === "pending";
  const settling = item.status === "settling";
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const act = async (action: PermissionAction) => {
    setBusy(action.optionId ?? action.decision);
    setError(null);
    try { await onResolve(item.eventId, action.decision, action.optionId); }
    catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); }
    finally { setBusy(null); }
  };
  return <div role={pending ? "alert" : "status"} className={`${PANEL} border-l-warning`}>
    <header className="flex flex-wrap items-baseline gap-x-2 gap-y-1 px-3.5 pt-3 sm:px-4">
      <b className="text-[13px] font-semibold text-foreground">{item.title || "Permission needed"}</b>
      {pending && <small className="text-[10.5px] tracking-[0.03em] text-warning">waiting for you</small>}
      {settling && <small className="text-[10.5px] tracking-[0.03em] text-warning">settling…</small>}
    </header>
    {item.text && <p className="mt-1.5 px-3.5 text-[12.5px] leading-relaxed text-muted-foreground sm:px-4">{item.text}</p>}
    {item.data.command ? <code className={`mx-3.5 mt-2.5 sm:mx-4 ${WELL}`}>{String(item.data.command)}</code> : null}
    {pending && actions.length > 0 && <div className="flex flex-wrap justify-end gap-[7px] px-3.5 py-3 sm:px-4">
      {actions.map(action => <button
        key={`${action.optionId ?? "bridge"}:${action.decision}`}
        disabled={!!busy}
        className={action.decision === "accept" ? BTN_PRIMARY : BTN_SECONDARY}
        onClick={() => void act(action)}
      >{action.decision === "decline" ? <X size={12} aria-hidden="true" /> : action.decision === "accept" ? <Check size={12} aria-hidden="true" /> : null}{busy === (action.optionId ?? action.decision) ? "Applying…" : action.label}</button>)}
    </div>}
    {pending && actions.length === 0 && <p className="px-3.5 py-3 text-[11.5px] text-muted-foreground sm:px-4">The provider offered no supported action.</p>}
    {!pending && <p className={`px-3.5 py-3 text-[11.5px] sm:px-4 ${item.status === "failed" ? "text-destructive" : "text-muted-foreground"}`}>{resolutionCopy(item)}</p>}
    {typeof item.data.failure === "string" && <p role="alert" className="px-3.5 pb-3 text-[11.5px] text-destructive sm:px-4">{item.data.failure}</p>}
    {error && <p role="alert" className="px-3.5 pb-3 text-[11.5px] text-destructive sm:px-4">{error}</p>}
  </div>;
}

type QuestionField = { id: string; prompt: string; options: string[]; multiple: boolean };

function questionFields(data: Record<string, unknown>): QuestionField[] {
  if (Array.isArray(data.questions)) {
    return data.questions.flatMap((value, index) => {
      if (!value || typeof value !== "object") return [];
      const question = value as Record<string, unknown>;
      const options = Array.isArray(question.options)
        ? question.options.flatMap(option => typeof option === "string" ? [option] : option && typeof option === "object" && typeof (option as Record<string, unknown>).label === "string" ? [String((option as Record<string, unknown>).label)] : [])
        : [];
      return [{
        id: typeof question.id === "string" ? question.id : String(index),
        prompt: String(question.question ?? question.header ?? "Answer"),
        options,
        multiple: question.multiple === true || question.multiSelect === true,
      }];
    });
  }
  const schema = data.requestedSchema && typeof data.requestedSchema === "object" ? data.requestedSchema as Record<string, unknown> : {};
  const properties = schema.properties && typeof schema.properties === "object" ? schema.properties as Record<string, unknown> : {};
  return Object.entries(properties).map(([id, value]) => {
    const property = value && typeof value === "object" ? value as Record<string, unknown> : {};
    return {
      id,
      prompt: String(property.title ?? property.description ?? id),
      options: Array.isArray(property.enum) ? property.enum.map(String) : [],
      multiple: property.type === "array",
    };
  });
}

function QuestionCard({ item, onResolve }: { item: ConversationItem; onResolve: ResolveQuestion }) {
  const fields = questionFields(item.data);
  const [answers, setAnswers] = useState<Record<string, string[]>>({});
  const [busy, setBusy] = useState<QuestionAction | null>(null);
  const [error, setError] = useState<string | null>(null);
  const pending = item.status === "pending";
  const setAnswer = (field: QuestionField, value: string) => setAnswers(current => ({
    ...current,
    [field.id]: field.multiple
      ? (current[field.id] ?? []).includes(value) ? (current[field.id] ?? []).filter(option => option !== value) : [...(current[field.id] ?? []), value]
      : [value],
  }));
  const act = async (action: QuestionAction) => {
    setBusy(action);
    setError(null);
    try { await onResolve(item.eventId, action, answers); }
    catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); }
    finally { setBusy(null); }
  };
  return <div role={pending ? "alert" : "status"} className={`${PANEL} border-l-info`}>
    <header className="flex flex-wrap items-baseline gap-x-2 gap-y-1 px-3.5 pt-3 sm:px-4">
      <b className="text-[13px] font-semibold text-foreground">{item.title || "Question"}</b>
      {pending && <small className="text-[10.5px] tracking-[0.03em] text-info">waiting for your answer</small>}
    </header>
    {item.text && <p className="mt-1.5 px-3.5 text-[12.5px] leading-relaxed text-muted-foreground sm:px-4">{item.text}</p>}
    {pending && <div className="space-y-3 px-3.5 py-3 sm:px-4">
      {fields.map(field => <fieldset key={field.id} className="space-y-2">
        <legend className="text-[12px] font-medium text-foreground">{field.prompt}</legend>
        {field.options.length > 0 ? <div className="flex flex-wrap gap-1.5">{field.options.map(option => <button
          type="button"
          key={option}
          aria-pressed={(answers[field.id] ?? []).includes(option)}
          className={(answers[field.id] ?? []).includes(option) ? BTN_PRIMARY : BTN_SECONDARY}
          onClick={() => setAnswer(field, option)}
        >{option}</button>)}</div> : <input
          value={answers[field.id]?.[0] ?? ""}
          onChange={event => setAnswers(current => ({ ...current, [field.id]: [event.target.value] }))}
          className="w-full rounded-lg border border-border bg-background px-3 py-2 text-[12.5px] text-foreground outline-none transition-colors placeholder:text-muted-foreground/60 focus:border-ring"
          placeholder="Type your answer"
        />}
      </fieldset>)}
      {fields.length === 0 && <p className="text-[11.5px] text-muted-foreground">This provider did not supply an answerable question shape.</p>}
      <div className="flex flex-wrap justify-end gap-[7px] pt-1">
        <button disabled={!!busy} className={BTN_SECONDARY} onClick={() => void act("decline")}><X size={12} aria-hidden="true" />{busy === "decline" ? "Declining…" : "Decline"}</button>
        <button disabled={!!busy || fields.length === 0 || !Object.values(answers).some(values => values.some(value => value.trim()))} className={BTN_PRIMARY} onClick={() => void act("answer")}><Check size={12} aria-hidden="true" />{busy === "answer" ? "Sending…" : "Send answer"}</button>
      </div>
    </div>}
    {!pending && <p className={`px-3.5 py-3 text-[11.5px] sm:px-4 ${item.status === "failed" ? "text-destructive" : "text-muted-foreground"}`}>{resolutionCopy(item)}</p>}
    {typeof item.data.failure === "string" && <p role="alert" className="px-3.5 pb-3 text-[11.5px] text-destructive sm:px-4">{item.data.failure}</p>}
    {error && <p role="alert" className="px-3.5 pb-3 text-[11.5px] text-destructive sm:px-4">{error}</p>}
  </div>;
}

function ApprovalCard({ item, onResolve }: { item: ConversationItem; onResolve: ResolvePermission }) {
  const transition = useMotionTransition(MOTION_DURATION.tick);
  const [busy, setBusy] = useState<ApprovalDecision | null>(null);
  const [error, setError] = useState<string | null>(null);
  const pending = item.status === "pending";
  const accepted = item.status === "accept" || item.status === "acceptForSession";
  const scope = Array.isArray(item.data.requestedOwnedPaths) ? item.data.requestedOwnedPaths.map(String) : [];
  // The machine-readable routing reason and its remediation are persisted on the
  // approval entry. Showing them is what turns "allow this?" into a decision the
  // user can actually make.
  const reason = typeof item.data.reason === "string" ? item.data.reason : "";
  const remediation = typeof item.data.remediation === "string" ? item.data.remediation : "";
  const human = reason ? humanizeApprovalReason(reason) : { title: item.title || "Approval needed", detail: undefined };
  return <div className={`${PANEL} border-l-warning`}>
    <header className="flex flex-wrap items-baseline gap-x-2 gap-y-1 pt-3 px-3.5 sm:px-4"><b className="text-[13px] font-semibold text-foreground">{human.title}</b>{pending && <small className="text-warning text-[10.5px] tracking-[0.03em]">waiting for you</small>}</header>
    {human.detail ? <p className="mt-1.5 px-3.5 text-muted-foreground text-[12.5px] leading-relaxed sm:px-4">{human.detail}</p> : null}
    {item.data.objective ? <p className="mt-1.5 px-3.5 text-muted-foreground text-[12.5px] leading-relaxed sm:px-4">{String(item.data.objective)}</p> : null}
    {scope.length > 0 && <div className="mt-2 px-3.5 sm:px-4">
      <small className="block text-muted-foreground/70 text-[10.5px] tracking-[0.03em] uppercase">Write scope</small>
      <code className={`mt-1 ${WELL}`}>{scope.join("\n")}</code>
    </div>}
    {remediation
      ? <p className="mt-2 px-3.5 text-muted-foreground text-[12.5px] leading-relaxed sm:px-4">{remediation}</p>
      : item.text && <p className="mt-1.5 px-3.5 text-muted-foreground text-[12.5px] leading-relaxed sm:px-4">{item.text}</p>}
    {item.data.command ? <code className={`mt-2.5 mx-3.5 sm:mx-4 ${WELL}`}>{String(item.data.command)}</code> : null}
    {item.data.cwd ? <small className="block pt-1.5 px-3.5 text-muted-foreground/70 font-mono text-[10.5px] break-all sm:px-4">{String(item.data.cwd)}</small> : null}
    {/* Resolving an approval swaps the actions for the outcome. `mode="wait"`
        lets the buttons leave before the verdict arrives, so the card reads as
        settling rather than as one row being overwritten by another. */}
    <AnimatePresence mode="wait" initial={false}>
      {pending
        ? <motion.div
            key="actions"
            className="flex flex-wrap justify-end gap-[7px] px-3.5 py-3 sm:px-4"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={transition}
          >
            <button disabled={!!busy} className={BTN_SECONDARY} onClick={() => { setBusy("decline"); setError(null); void Promise.resolve(onResolve(item.eventId, "decline")).catch(cause => setError(cause instanceof Error ? cause.message : String(cause))).finally(() => setBusy(null)); }}><X size={12} aria-hidden="true" /> Decline</button>
            {item.data.approvalType !== "delegation_path_scope" && <button disabled={!!busy} className={BTN_SECONDARY} onClick={() => { setBusy("acceptForSession"); setError(null); void Promise.resolve(onResolve(item.eventId, "acceptForSession")).catch(cause => setError(cause instanceof Error ? cause.message : String(cause))).finally(() => setBusy(null)); }}>Allow for session</button>}
            <button disabled={!!busy} className={BTN_PRIMARY} onClick={() => { setBusy("accept"); setError(null); void Promise.resolve(onResolve(item.eventId, "accept")).catch(cause => setError(cause instanceof Error ? cause.message : String(cause))).finally(() => setBusy(null)); }}><Check size={12} aria-hidden="true" /> {busy === "accept" ? "Allowing…" : "Allow once"}</button>
          </motion.div>
        : <motion.div
            key="resolved"
            className="flex items-center gap-1.5 px-3.5 pb-3 pt-2.5 text-muted-foreground text-[11.5px] sm:px-4"
            initial={{ opacity: 0, y: 4 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0 }}
            transition={transition}
          >{accepted ? <Check size={12} aria-hidden="true" /> : <X size={12} aria-hidden="true" />} {humanizeResolution(item.status ?? "resolved")}</motion.div>}
    </AnimatePresence>
    {reason && <details className="border-t border-border px-3.5 py-2 text-[11px] text-muted-foreground sm:px-4">
      <summary className="cursor-pointer">Policy</summary>
      <p className="mt-1 font-mono break-all">{reason}</p>
    </details>}
    {error && <p role="alert" className="px-3.5 pb-3 text-[11.5px] text-destructive sm:px-4">{error}</p>}
  </div>;
}

/// A workspace far behind its base branch: any change lands on stale code and
/// completion evidence gets stamped against it. The refresh action is a strict
/// fast-forward, so declining it by doing nothing is always safe.
function StaleBaseCard({ item, onRefresh }: { item: ConversationItem; onRefresh?: () => Promise<void> }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [refreshed, setRefreshed] = useState(false);
  const divergence = (item.data.divergence ?? {}) as Record<string, unknown>;
  const behind = Number(divergence.behind ?? 0);
  const ahead = Number(divergence.ahead ?? 0);
  const baseRef = String(divergence.baseRef ?? "its base branch");
  // Why a fast-forward is impossible, or null when it is possible. These are
  // `fast_forward_to_base`'s own refusals, asked before the call rather than
  // reported after it: a local commit or a dirty tree can only be resolved by
  // the user, so the button would fail every time it was pressed.
  const blocker = ahead > 0
    ? `This workspace has ${ahead} commit${ahead === 1 ? "" : "s"} that ${baseRef} does not, so it cannot be fast-forwarded. Rebase or merge onto ${baseRef} yourself, or keep working on the current revision.`
    : divergence.dirty === true
      ? `This workspace has uncommitted changes, so it cannot be fast-forwarded. Commit or stash them first, or keep working on the current revision.`
      : null;
  const refresh = async () => {
    if (!onRefresh) return;
    setBusy(true); setError(null);
    try { await onRefresh(); setRefreshed(true); }
    catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); }
    finally { setBusy(false); }
  };
  return <div role="alert" className={`${PANEL} border-l-warning`}>
    <header className="flex items-baseline gap-[9px] px-3.5 pt-3 sm:px-4">
      <b className="text-[13px] font-semibold text-foreground">{item.title || `Workspace is ${behind} commits behind ${baseRef}`}</b>
    </header>
    <p className="mt-1.5 px-3.5 text-[12.5px] leading-relaxed text-muted-foreground sm:px-4">{item.text}</p>
    <small className="mt-1 block px-3.5 font-mono text-[10.5px] break-all text-muted-foreground/70 sm:px-4">{behind} behind · {ahead} ahead · {baseRef}{divergence.dirty === true ? " · uncommitted changes" : ""}</small>
    {error && <p className="mt-1.5 px-3.5 text-[12px] leading-relaxed text-destructive sm:px-4">{error}</p>}
    {refreshed
      ? <div className="flex items-center gap-1.5 px-3.5 pb-3 pt-2.5 text-[11.5px] text-muted-foreground sm:px-4"><Check size={12} aria-hidden="true" /> Workspace refreshed onto {baseRef}</div>
      : blocker
        // Refresh is a strict fast-forward, and this card already has the
        // fields that decide whether one is possible. Offering the button
        // anyway meant the common case — a worktree with one commit on it —
        // presented an action whose only outcome was an error dialog.
        ? <div className="px-3.5 pb-3 pt-2.5 text-[11.5px] leading-relaxed text-muted-foreground sm:px-4">{blocker}</div>
        : <div className="flex flex-wrap items-center justify-end gap-[7px] px-3.5 py-3 sm:px-4">
            <span className="mr-auto text-[11.5px] text-muted-foreground/70">Or continue on the current revision.</span>
            <button disabled={busy || !onRefresh} className={BTN_PRIMARY} onClick={refresh}>{busy ? "Refreshing…" : "Refresh workspace"}</button>
          </div>}
  </div>;
}

function DelegationRow({ item, workers, now, onOpenSession, onExpandWorker, onRetryWorker }: { item: ConversationItem; workers?: WorkerPanelSource; now?: number; onOpenSession?: (sessionId: string) => void; onExpandWorker?: (sessionId: string) => void; onRetryWorker?: (childSessionId: string) => Promise<void> }) {
  // Every hook before the first early return: an item's facet changes under it
  // (a spawn becomes a result when the envelope lands), so the hook count must
  // not depend on which branch renders.
  const [open, setOpen] = useState(false);
  const facet = delegationFacet(item);
  const childSessionId = delegationChildSessionId(item);
  // The row the user watches while a worker runs. Everything needed to draw it
  // lives outside the event — the runtime record and the live stream — so a
  // conversation rendered without that context falls through to the quiet row.
  const panel = facet === "spawn" || facet === "result"
    ? childSessionId && workers ? workerPanelModel(childSessionId, workers.sessions, workers.runtimes, workers.events) : null
    : null;
  if (facet === "steered") return <SteerChip item={item} onOpenSession={onOpenSession}/>;
  // A background worker's own approval card renders on the worker's conversation,
  // which is normally not the selected one. This mirrored row is what makes the
  // block visible where the user is actually working.
  if (facet === "blocked") {
    const blocked = item.data.childBlocked === true;
    const paths = Array.isArray(item.data.ownedPaths) ? item.data.ownedPaths.map(String) : [];
    // The child session id travels on the event, so the mirror can hand the user
    // straight to the worker's conversation where the real approval lives —
    // otherwise the block is a dead end and the card is effectively lost.
    if (!blocked) {
      return <div className="my-3 flex min-w-0 items-center gap-[9px] px-2 -ml-2 text-muted-foreground text-[12.5px]">
        <Check size={13} className="shrink-0" aria-hidden="true" />
        <span className="min-w-0 truncate">{item.title || "Worker approval resolved"}</span>
      </div>;
    }
    return <div className="my-3 min-w-0 rounded-lg border border-border border-l-2 border-l-warning bg-card px-3 py-2 text-xs text-muted-foreground" role="alert">
      <div className="flex items-center gap-1.5 font-medium text-warning"><AlertTriangle size={13} className="shrink-0" aria-hidden="true" /> <span className="min-w-0">{item.title || "A worker needs your approval"}</span></div>
      {item.data.objective ? <p className="mt-1">{String(item.data.objective)}</p> : null}
      {item.text && <p className="mt-1">{item.text}</p>}
      {item.data.command ? <code className={`mt-1.5 ${WELL}`}>{String(item.data.command)}</code> : null}
      {item.data.cwd ? <small className="mt-1 block font-mono text-[10.5px] break-all text-muted-foreground/70">{String(item.data.cwd)}</small> : null}
      {paths.length > 0 && <small className="mt-1 block font-mono text-[10.5px] break-all text-muted-foreground/70">write scope: {paths.join(", ")}</small>}
      {childSessionId && onOpenSession
        ? <div className="mt-2 flex flex-wrap items-center gap-2">
            <button type="button" onClick={() => onOpenSession(childSessionId)} className="inline-flex items-center gap-1.5 rounded-full bg-primary px-2.5 py-1 text-[11px] font-medium text-primary-foreground transition-colors hover:bg-primary/90"><CornerDownRight size={12} aria-hidden="true" /> Open worker to approve</button>
            <span className="text-muted-foreground/70">The worker is idle until you do.</span>
          </div>
        : <p className="mt-1 text-muted-foreground/70">Open the worker&apos;s conversation to allow or decline. The worker is idle until you do.</p>}
    </div>;
  }
  if (facet === "rejected") {
    const reason = String(item.data.reason ?? item.text ?? "");
    const willRetry = item.data.willRetry === true;
    const launchFailed = item.data.launchFailed === true;
    return <div className="my-3 min-w-0 rounded-lg border border-border border-l-2 border-l-warning bg-card px-3 py-2 text-xs text-muted-foreground" role="alert">
      <div className="flex items-center gap-1.5 font-medium text-warning"><AlertTriangle size={13} className="shrink-0" aria-hidden="true" /> <span className="min-w-0">{launchFailed ? "Worker failed to start" : "Delegation rejected — no worker started"}</span></div>
      {reason && <p className="mt-1 font-mono text-[11px] leading-relaxed break-words text-foreground">{reason}</p>}
      <p className="mt-1 text-muted-foreground/70">{launchFailed ? (item.data.orchestratorNotified === true ? "The orchestrator was notified and will not wait for this worker." : "The orchestrator could not be notified; retry after fixing the launch failure.") : willRetry ? "Asked the orchestrator to correct and re-emit the request." : "Automatic correction limit reached; the orchestrator will not retry on its own."}</p>
    </div>;
  }
  const isResult = facet === "result";
  const model = String(item.data.modelLabel ?? item.data.model ?? "");
  const effort = item.data.effort ? String(item.data.effort) : "";
  // Bridge no longer spends a hidden turn retrying a cause it cannot show has
  // changed, so a terminal failure has to arrive with its real reason and the
  // action the user would otherwise have had no way to take.
  const failureCause = typeof item.data.failureCause === "string" ? item.data.failureCause : "";
  const failureClass = typeof item.data.failureClass === "string" ? item.data.failureClass : "";
  const retrySessionId = item.data.canRetry === true && typeof item.data.childSessionId === "string"
    ? item.data.childSessionId
    : undefined;
  if (isResult && failureCause) {
    return <WorkerFailureRow
      title={item.title || "Worker finished without completing"}
      summary={item.text}
      cause={failureCause}
      failureClass={failureClass}
      childSessionId={retrySessionId}
      onOpenSession={onOpenSession}
      onRetryWorker={onRetryWorker}
    />;
  }
  if (panel && childSessionId) {
    return <WorkerPanel
      model={panel}
      objective={item.text}
      modelLabel={model}
      effort={effort}
      now={now}
      onOpenSession={onOpenSession}
      onExpandWorker={onExpandWorker}
    />;
  }
  return <div className="my-3 min-w-0">
    <button className="w-full min-w-0 flex items-center gap-[9px] min-h-[30px] px-2 py-1 -ml-2 rounded-md text-left text-muted-foreground text-[12.5px] hover:bg-accent transition-colors" onClick={() => item.text && setOpen(value => !value)}>
      {isResult ? <CornerDownRight size={13} className="shrink-0" aria-hidden="true" /> : <GitFork size={13} className="shrink-0" aria-hidden="true" />}
      <span className="min-w-0 flex-1 overflow-hidden whitespace-nowrap text-ellipsis">{isResult ? "Subagent finished" : "Delegated"}{titleAddsInfo(item, isResult) && <b className="text-muted-foreground font-medium"> · {item.title}</b>}</span>
      {model && <em className="hidden flex-none font-mono text-[10px] text-muted-foreground/70 not-italic border border-border rounded px-1.5 py-0.5 sm:inline">{model}{effort ? ` · ${effort}` : ""}</em>}
      {item.text && <ChevronRight size={12} className={`shrink-0 transition-transform ${open ? "rotate-90" : ""}`} aria-hidden="true" />}
    </button>
    {open && item.text && <div className="my-1 ml-[5px] min-w-0 pl-[15px] border-l border-border text-muted-foreground text-[12.5px] leading-relaxed"><Markdown text={item.text}/></div>}
  </div>;
}

/// The sessions, runtime rows, and live event stream a worker panel reads.
///
/// Passed as one object because all three are the same fact from three angles —
/// who the worker is, what the host knows about it, and what it just did — and
/// a panel is useless with any of them missing.
export interface WorkerPanelSource {
  sessions: Session[];
  runtimes: WorkerRuntimeRecord[];
  /** The *global* live stream, not this session's slice: the worker's frames
   *  arrive under the worker's own session id. */
  events: AgentEvent[];
}

const PANEL_TONE_TEXT: Record<WorkerTone, string> = {
  working: "text-success",
  waiting: "text-warning",
  attention: "text-warning",
  warm: "text-info",
  done: "text-muted-foreground",
  failed: "text-destructive",
  stalled: "text-destructive",
  idle: "text-muted-foreground",
};

/// A clock that only ticks while there is something to count.
///
/// A finished panel showing a frozen elapsed time is correct; re-rendering it
/// every second forever is not, and a transcript can hold many of these.
function useLiveClock(active: boolean, override?: number): number {
  const [tick, setTick] = useState(Date.now);
  useEffect(() => {
    if (override !== undefined || !active) return;
    const timer = window.setInterval(() => setTick(Date.now()), 1_000);
    return () => window.clearInterval(timer);
  }, [active, override]);
  return override ?? tick;
}

/// One worker, live, inside the conversation that delegated it.
///
/// The gap this closes: the user sat in the orchestrator chat watching a static
/// "Delegating to a worker…" line while the actual work happened somewhere they
/// were not looking. Same card carries the run and the outcome, so a worker is
/// one place in the transcript rather than two.
function WorkerPanel({ model, objective, modelLabel, effort, now, onOpenSession, onExpandWorker }: {
  model: WorkerPanelModel;
  objective?: string;
  modelLabel?: string;
  effort?: string;
  now?: number;
  onOpenSession?: (sessionId: string) => void;
  onExpandWorker?: (sessionId: string) => void;
}) {
  const live = !model.reported;
  const clock = useLiveClock(live, now);
  // A finished worker reports how long it took, not how long ago it started.
  const elapsedAt = !live && model.endedAt ? Date.parse(model.endedAt) : clock;
  const name = model.session.title || model.session.label;
  const tests = model.result?.tests ?? [];
  const failedTests = tests.filter(test => test.status === "failed").length;
  return <section className="my-3 min-w-0 overflow-hidden rounded-xl border border-border bg-card" aria-label={`Worker ${name}`}>
    <header className="flex flex-wrap items-center gap-x-2.5 gap-y-1 px-3.5 py-2.5">
      {live
        ? <PulseDot size={7}/>
        : <CornerDownRight size={13} className="shrink-0 text-muted-foreground" aria-hidden="true"/>}
      <b className="min-w-0 truncate text-[12.5px] font-medium text-foreground">{name}</b>
      <span className={cn("shrink-0 text-[8.5px] font-semibold tracking-[0.07em]", PANEL_TONE_TEXT[model.status.tone])}>{model.status.label}</span>
      <span className="flex-1"/>
      {modelLabel && <em className="hidden flex-none rounded border border-border px-1.5 py-0.5 font-mono text-[10px] not-italic text-muted-foreground/70 sm:inline">{modelLabel}{effort ? ` · ${effort}` : ""}</em>}
      {model.retryCount > 0 && <span className="inline-flex shrink-0 items-center gap-0.5 font-mono text-[9px] text-muted-foreground"><RotateCcw size={9} aria-hidden="true"/>retry {model.retryCount}</span>}
      <span className="shrink-0 font-mono text-[9px] text-muted-foreground">{formatElapsed(model.startedAt, elapsedAt)}</span>
    </header>

    {objective && <p className="px-3.5 pb-2 text-[12px] leading-relaxed text-muted-foreground">{objective}</p>}

    {(model.progressSummary || model.waitingReason) && <div className="flex flex-wrap items-center gap-x-2.5 gap-y-1 border-t border-border px-3.5 py-2 text-[10.5px]">
      {model.progressSummary && <span className="min-w-0 truncate font-mono text-foreground/80">{model.progressSummary}</span>}
      {model.waitingReason && <span className="shrink-0 rounded-full border border-warning/30 bg-warning/10 px-2 py-0.5 text-[9px] font-medium text-warning">waiting: {model.waitingReason.replaceAll("_", " ")}{model.waitingSince ? ` · ${formatElapsed(model.waitingSince, clock)}` : ""}</span>}
    </div>}

    {live && model.feed.length > 0 && <ol className="m-0 list-none border-t border-border px-3.5 py-2 space-y-0.5">
      {model.feed.map((line, index) => <li key={line.id} className={cn("truncate font-mono text-[10.5px] leading-[1.6]", index === model.feed.length - 1 ? "text-muted-foreground" : "text-muted-foreground/60")}>{line.text}</li>)}
    </ol>}

    {model.result && <div className="flex flex-wrap items-center gap-x-3 gap-y-1 border-t border-border px-3.5 py-2 text-[10.5px] text-muted-foreground">
      {model.result.filesChanged.length > 0 && <span className="inline-flex items-center gap-1"><FileText size={11} aria-hidden="true"/>{model.result.filesChanged.length} file{model.result.filesChanged.length === 1 ? "" : "s"}</span>}
      {tests.length > 0 && <span className={cn("inline-flex items-center gap-1", failedTests ? "text-destructive" : "text-success")}>{failedTests ? <X size={11} aria-hidden="true"/> : <Check size={11} aria-hidden="true"/>}{failedTests ? `${failedTests} of ${tests.length} failing` : `${tests.length} test${tests.length === 1 ? "" : "s"} passing`}</span>}
      {model.result.summary && <span className="min-w-0 flex-1 truncate">{model.result.summary}</span>}
    </div>}

    {(onExpandWorker || onOpenSession) && <footer className="flex flex-wrap items-center gap-2 border-t border-border px-3.5 py-2">
      {onExpandWorker && <button type="button" onClick={() => onExpandWorker(model.session.id)} className="inline-flex items-center gap-1.5 rounded-full border border-border px-2.5 py-1 text-[11px] transition-colors hover:bg-accent"><Maximize2 size={11} aria-hidden="true"/> Expand</button>}
      {onOpenSession && <button type="button" onClick={() => onOpenSession(model.session.id)} className="inline-flex items-center gap-1.5 rounded-full border border-border px-2.5 py-1 text-[11px] transition-colors hover:bg-accent"><CornerDownRight size={11} aria-hidden="true"/> Open session</button>}
    </footer>}
  </section>;
}

/// Someone redirected a running worker. A quiet line, because the intervention
/// matters but is not a decision the reader has to make — and because the user
/// who typed it already knows; this is here so the *other* surfaces agree.
function SteerChip({ item, onOpenSession }: { item: ConversationItem; onOpenSession?: (sessionId: string) => void }) {
  const childSessionId = delegationChildSessionId(item);
  // Two separate facts. Whether the guidance reached the worker is the one the
  // person who typed it cares about; whether the orchestrator was told is a
  // quieter footnote. Reading one flag for both made a landed steer read as
  // failed whenever the parent's runtime happened to be down.
  const undelivered = item.data.steerDelivered === false;
  const queued = item.data.landed === "next_turn_boundary";
  const unnotified = item.data.orchestratorNotified === false;
  return <div className="my-3 flex min-w-0 items-center gap-[9px] px-2 -ml-2 text-[12.5px] text-muted-foreground">
    <Navigation size={12} className={cn("shrink-0", undelivered && "text-warning")} aria-hidden="true"/>
    <span className="min-w-0 flex-1 truncate">{item.title || "Worker steered"}{item.text && <span className="text-muted-foreground/70"> — {item.text}</span>}</span>
    {undelivered
      ? <span className="shrink-0 text-[9px] font-medium tracking-[0.06em] text-warning">NOT DELIVERED</span>
      : queued && <span className="shrink-0 text-[9px] font-medium tracking-[0.06em] text-muted-foreground/70">AT NEXT STEP</span>}
    {!undelivered && unnotified && <span className="shrink-0 text-[9px] tracking-[0.06em] text-muted-foreground/70">ORCHESTRATOR NOT TOLD</span>}
    {childSessionId && onOpenSession && <button type="button" onClick={() => onOpenSession(childSessionId)} className="shrink-0 rounded-full border border-border px-2 py-0.5 text-[10.5px] transition-colors hover:bg-accent">Open</button>}
  </div>;
}

/// A worker that ended without completing, said plainly.
///
/// The old card collapsed every outcome into "Subagent finished" and let the
/// orchestrator silently retry. Naming the classified cause is what lets the
/// person reading decide whether another attempt is worth anything.
function WorkerFailureRow({ title, summary, cause, failureClass, childSessionId, onOpenSession, onRetryWorker }: {
  title: string;
  summary: string;
  cause: string;
  failureClass: string;
  childSessionId?: string;
  onOpenSession?: (sessionId: string) => void;
  onRetryWorker?: (childSessionId: string) => Promise<void>;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const transport = failureClass === "protocol_invalid";
  return <div className="my-3 min-w-0 rounded-lg border border-border border-l-2 border-l-warning bg-card px-3 py-2 text-xs text-muted-foreground" role="alert">
    <div className="flex items-center gap-1.5 font-medium text-warning"><AlertTriangle size={13} className="shrink-0" aria-hidden="true" /> <span className="min-w-0">{title}</span></div>
    <p className="mt-1 text-foreground">{cause}</p>
    {transport && <p className="mt-1 text-muted-foreground/70">Bridge could not read this worker&apos;s result, so nothing below has been verified. Retrying would not change that on its own.</p>}
    {summary && <p className="mt-1.5 whitespace-pre-wrap break-words">{summary}</p>}
    <div className="mt-2 flex flex-wrap items-center gap-2">
      {childSessionId && onRetryWorker && <button
        type="button"
        disabled={busy}
        onClick={() => { setBusy(true); setError(undefined); void onRetryWorker(childSessionId).catch((cause: unknown) => setError(cause instanceof Error ? cause.message : String(cause))).finally(() => setBusy(false)); }}
        className="inline-flex items-center gap-1.5 rounded-full bg-primary px-2.5 py-1 text-[11px] font-medium text-primary-foreground transition-colors hover:bg-primary/90 disabled:opacity-50"
      ><RotateCcw size={12} aria-hidden="true" /> {busy ? "Retrying…" : "Retry this task"}</button>}
      {childSessionId && onOpenSession && <button type="button" onClick={() => onOpenSession(childSessionId)} className="inline-flex items-center gap-1.5 rounded-full border border-border px-2.5 py-1 text-[11px] transition-colors hover:bg-accent"><CornerDownRight size={12} aria-hidden="true" /> Open the worker</button>}
    </div>
    {error && <p className="mt-1.5 text-destructive">{error}</p>}
  </div>;
}

function titleAddsInfo(item: ConversationItem, isResult: boolean): boolean {
  const title = (item.title ?? "").trim();
  if (!title) return false;
  return isResult ? !/^worker result$/i.test(title) : true;
}

function stringList(value: unknown) { return Array.isArray(value) ? value.join("\n") : ""; }
function planSteps(data: Record<string, unknown>): Array<{ step: string; status: string }> {
  return Array.isArray(data.plan) ? data.plan.filter((v): v is { step: string; status: string } => !!v && typeof v === "object" && "step" in v && "status" in v) : [];
}
