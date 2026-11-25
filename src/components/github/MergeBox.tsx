import { AnimatePresence, motion } from "framer-motion";
import {
  Ban,
  Check,
  ChevronDown,
  CircleCheck,
  CircleDashed,
  CircleX,
  Ellipsis,
  Eye,
  FileDiff,
  GitCommitHorizontal,
  GitMerge,
  GitPullRequestDraft,
  LoaderCircle,
  MessageSquare,
  RotateCcw,
  Sparkles,
  ThumbsUp,
  type LucideIcon,
} from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { cn } from "@/lib/utils";
import { Kbd } from "@/components/ui/kbd";
import { MenuItem, MenuPanel, useMenuPanel } from "@/components/ui/menu-panel";
import { modKey } from "@/lib/platform";
import { usePrefersReducedMotion } from "@/motion";
import type {
  GithubChecksResult,
  GithubMergeConfigResult,
  MergeStrategy,
  PullRequestSummary,
} from "@/protocol/generated/protocol";
import { MergeGraph } from "./MergeGraph";
import { RollupMeter } from "../GitHubPane";

// The merge box (issue #1028): one card that owns readiness, the strategy,
// the confirm step and the receipt. Merge expands the card into the confirm
// step in place — no dialog — the hairline sweeps while `gh` runs, the commits
// gather into main on success, and a refusal shakes once and keeps the form.

export type MergePhase = "idle" | "confirming" | "merging" | "merged" | "failed";

export type MergeState = {
  phase: MergePhase;
  strategy: MergeStrategy;
  subject: string;
  deleteBranch: boolean;
  /** The refusal reason, verbatim from `gh`. */
  message?: string;
  /** The landing choreography plays once, on a real `executed: true`. */
  celebrate: boolean;
  /** False when the PR was already merged on load: the strategy is unknown. */
  strategyKnown?: boolean;
};

export type MergeEvent =
  | { type: "reset"; summary?: PullRequestSummary }
  | { type: "config"; strategy: MergeStrategy }
  | { type: "open"; subject: string }
  | { type: "pick"; strategy: MergeStrategy; subject: string }
  | { type: "collapse" }
  | { type: "subject"; subject: string }
  | { type: "deleteBranch"; value: boolean }
  | { type: "merging" }
  | { type: "succeed" }
  | { type: "fail"; message: string };

/** Collapsed regions leave the tab order too, not just the pointer. */
const inertWhen = (shut: boolean) => (shut ? ({ inert: "" } as Record<string, string>) : {});

/** The default commit subject for a strategy, as `gh` would write it. */
export function defaultMergeSubject(summary: PullRequestSummary, strategy: MergeStrategy): string {
  return strategy === "merge"
    ? `Merge pull request #${summary.number} from ${summary.headBranch}`
    : `${summary.title} (#${summary.number})`;
}

export function mergeReducer(state: MergeState, event: MergeEvent): MergeState {
  switch (event.type) {
    case "reset":
      return initialMergeState(event.summary);
    case "config":
      if (state.phase !== "idle" || state.strategy === event.strategy) return state;
      // The repo default arrives late; a re-open recomputes the subject.
      return { ...state, strategy: event.strategy };
    case "open":
      return state.phase === "idle"
        ? { ...state, phase: "confirming", subject: event.subject, message: undefined }
        : state;
    case "pick":
      // A merge in flight or done is not reopened by a stale menu.
      if (state.phase === "merging" || state.phase === "merged") return state;
      return { ...state, phase: "confirming", strategy: event.strategy, subject: event.subject, message: undefined };
    case "collapse":
      return state.phase === "merging" ? state : { ...state, phase: "idle", message: undefined, celebrate: false };
    case "subject":
      return { ...state, subject: event.subject };
    case "deleteBranch":
      return { ...state, deleteBranch: event.value };
    case "merging":
      return { ...state, phase: "merging", message: undefined };
    case "succeed":
      return { ...state, phase: "merged", celebrate: true, message: undefined };
    case "fail":
      return { ...state, phase: "failed", message: event.message, celebrate: false };
  }
}

/** A detail that already reports merged renders the receipt, never a replay. */
export function initialMergeState(summary?: PullRequestSummary): MergeState {
  if (summary?.state === "merged") {
    return { phase: "merged", strategy: "squash", subject: "", deleteBranch: false, celebrate: false, strategyKnown: false };
  }
  return { phase: "idle", strategy: "squash", subject: "", deleteBranch: false, celebrate: false };
}

export const MERGE_BUTTON_LABEL: Record<MergeStrategy, string> = {
  merge: "Create a merge commit",
  squash: "Squash and merge",
  rebase: "Rebase and merge",
};

export type OverflowItem = {
  label: string;
  icon: LucideIcon;
  destructive?: boolean;
  onSelect: () => void;
};

const ROW_TONE: Record<"ok" | "wait" | "bad" | "warn", string> = {
  ok: "bg-success/12 text-success",
  wait: "bg-muted text-muted-foreground",
  bad: "bg-destructive/12 text-destructive",
  warn: "bg-warning/12 text-warning",
};

function ReadinessRow({ tone, icon: Icon, title, note, trailing, live }: {
  tone: "ok" | "wait" | "bad" | "warn";
  icon: LucideIcon;
  title: ReactNode;
  note?: ReactNode;
  trailing?: ReactNode;
  live?: boolean;
}) {
  return (
    <div className="flex items-center gap-2.5 border-b border-border px-3.5 py-2.5 last:border-b-0">
      <span className={cn("grid size-[22px] shrink-0 place-items-center rounded-full", ROW_TONE[tone])}>
        <Icon size={12} strokeWidth={1.9} aria-hidden="true" className={cn(live && "github-check-live")} />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block text-meta leading-snug text-foreground">{title}</span>
        {note && <span className="mt-0.5 block truncate text-caption text-muted-foreground">{note}</span>}
      </span>
      {trailing}
    </div>
  );
}

/** The mockup's mini commit-graph glyphs for the three strategies. */
function StrategyGlyph({ kind }: { kind: MergeStrategy }) {
  return (
    <svg viewBox="0 0 34 20" className="mt-0.5 h-5 w-[34px] shrink-0" aria-hidden="true">
      {kind === "merge" && <>
        <path d="M2 15h30M6 5h14c4 0 4 10 8 10" fill="none" stroke="currentColor" strokeWidth="1.6" className="text-muted-foreground" />
        <circle cx="28" cy="15" r="2.6" className="fill-foreground" />
      </>}
      {kind === "squash" && <>
        <path d="M2 15h30" fill="none" stroke="currentColor" strokeWidth="1.6" className="text-muted-foreground" />
        <circle cx="22" cy="15" r="2.6" className="fill-foreground" />
        <circle cx="10" cy="5" r="2" className="fill-muted-foreground" />
        <circle cx="16" cy="5" r="2" className="fill-muted-foreground" />
      </>}
      {kind === "rebase" && <>
        <path d="M2 15h30" fill="none" stroke="currentColor" strokeWidth="1.6" className="text-muted-foreground" />
        <circle cx="18" cy="15" r="2.4" className="fill-foreground" />
        <circle cx="26" cy="15" r="2.4" className="fill-foreground" />
      </>}
    </svg>
  );
}

function menuNote(strategy: MergeStrategy, commits: number, baseBranch: string, isDefault: boolean): string {
  const n = `${commits} commit${commits === 1 ? "" : "s"}`;
  switch (strategy) {
    case "merge": return `Keeps all ${n} and adds a merge commit.`;
    case "squash": return `Combines ${n} into one on ${baseBranch}.${isDefault ? " Repo default." : ""}`;
    case "rebase": return `Replays ${n} onto ${baseBranch}, no merge commit.`;
  }
}

const ENTER_EASE: [number, number, number, number] = [0.22, 1, 0.36, 1];

export type MergeBoxProps = {
  summary: PullRequestSummary;
  checks: GithubChecksResult["checks"];
  commitCount: number;
  baseBranch: string;
  headBranch: string;
  prUrl: string;
  config?: GithubMergeConfigResult;
  state: MergeState;
  dispatch: (event: MergeEvent) => void;
  busy: boolean;
  reviewBusy: boolean;
  /** Whether the review picker is open. */
  reviewOpen?: boolean;
  /** Reviews of this PR still running on Bridge (queued, preparing, reviewing, posting). */
  reviewing?: number;
  onToggleReview: () => void;
  onApprove: () => void;
  overflow: OverflowItem[];
  onConfirm: () => void;
  /** The receipt's Restore branch (a confirmed write; slice 5). */
  onRestore: () => void;
};

export function MergeBox({ summary, checks, commitCount, baseBranch, headBranch, prUrl, config, state, dispatch, busy, reviewBusy, reviewOpen = false, reviewing = 0, onToggleReview, onApprove, overflow, onConfirm, onRestore }: MergeBoxProps) {
  const reduced = usePrefersReducedMotion();
  const [menuOpen, setMenuOpen] = useState(false);
  const more = useMenuPanel<HTMLButtonElement>({ width: 208, height: 12 + overflow.length * 28 });
  const [shaking, setShaking] = useState(false);
  const boxRef = useRef<HTMLDivElement>(null);

  const open = state.phase === "idle";
  const merged = state.phase === "merged";
  const failed = state.phase === "failed";
  const merging = state.phase === "merging";
  const showConfirm = state.phase === "confirming" || merging || failed;
  // Reduced motion never plays: the receipt renders its settled frame.
  const celebrate = merged && state.celebrate && !reduced;

  // The refusal shakes the button once; it re-arms for the next "Try again".
  useEffect(() => {
    if (failed && !reduced) setShaking(true);
  }, [failed, reduced]);

  // Menus close on any press outside or on Escape (the confirm's own Escape
  // is handled at the detail level).
  useEffect(() => {
    if (!menuOpen) return;
    const close = () => { setMenuOpen(false); };
    const onPointerDown = (event: PointerEvent) => {
      if (boxRef.current?.contains(event.target as Node)) return;
      close();
    };
    const onKey = (event: KeyboardEvent) => { if (event.key === "Escape") close(); };
    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [menuOpen]);

  const rollup = summary.checks;
  const names = [...new Set(checks.map(check => check.name))];
  const checkNames = names.slice(0, 3).join(", ");
  const strategies = config
    ? (["merge", "squash", "rebase"] as const).filter(name => config.strategies[name])
    : [];

  const lands = state.strategy === "squash"
    ? <>Squashes <b className="font-medium text-foreground">{commitCount} commit{commitCount === 1 ? "" : "s"}</b> into one commit on <b className="font-medium text-foreground">{baseBranch}</b></>
    : state.strategy === "rebase"
      ? <>Replays <b className="font-medium text-foreground">{commitCount} commit{commitCount === 1 ? "" : "s"}</b> onto <b className="font-medium text-foreground">{baseBranch}</b>, no merge commit</>
      : <>Merges <b className="font-medium text-foreground">{commitCount} commit{commitCount === 1 ? "" : "s"}</b> into <b className="font-medium text-foreground">{baseBranch}</b> with a merge commit</>;

  const receiptLine = state.strategyKnown === false ? `Merged into ${baseBranch}` : `${state.strategy === "squash"
    ? `Squashed ${commitCount} commit${commitCount === 1 ? "" : "s"} into ${baseBranch}`
    : state.strategy === "rebase"
      ? `Replayed ${commitCount} commit${commitCount === 1 ? "" : "s"} onto ${baseBranch}`
      : `Merged ${commitCount} commit${commitCount === 1 ? "" : "s"} into ${baseBranch}`}${state.deleteBranch ? " · branch deleted" : ""}`;

  return (
    <div ref={boxRef} className="relative mt-4" aria-label="Merge pull request">
      <div className={cn(
        "relative rounded-xl border bg-card transition-colors duration-300",
        failed ? "border-destructive/55" : merged ? "border-merged/40" : "border-border-card",
      )}>
        {/* Readiness */}
        <div className={cn("u-collapse", !open && "shut")} aria-hidden={!open} {...inertWhen(!open)}>
          <div>
            {rollup.total > 0 && (() => {
              const verdict = rollup.failed > 0 ? "failing" : rollup.queued + rollup.inProgress > 0 ? "running" : rollup.passed > 0 ? "passing" : "none";
              return <ReadinessRow
                tone={verdict === "passing" ? "ok" : verdict === "failing" ? "bad" : "wait"}
                icon={verdict === "passing" ? CircleCheck : verdict === "failing" ? CircleX : CircleDashed}
                live={verdict === "running"}
                title={verdict === "passing" ? "All checks passed" : verdict === "failing" ? "Checks failing" : "Checks running"}
                note={checkNames ? `${checkNames} · ${rollup.passed} of ${rollup.total}` : `${rollup.passed} of ${rollup.total}`}
                trailing={<RollupMeter checks={rollup} className="w-20 shrink-0" />}
              />;
            })()}
            {summary.isDraft && <ReadinessRow
              tone="warn"
              icon={GitPullRequestDraft}
              title="Draft pull request"
              note="Mark it ready for review before merging"
            />}
            {summary.reviewDecision !== "none" && <ReadinessRow
              tone={summary.reviewDecision === "approved" ? "ok" : summary.reviewDecision === "changesRequested" ? "warn" : "wait"}
              icon={summary.reviewDecision === "approved" ? ThumbsUp : summary.reviewDecision === "changesRequested" ? MessageSquare : Eye}
              title={summary.reviewDecision === "approved" ? "Approved" : summary.reviewDecision === "changesRequested" ? "Changes requested" : "Review required"}
              note={summary.reviewDecision === "approved"
                ? "An approving review is in."
                : summary.reviewDecision === "changesRequested"
                  ? "A review blocks the merge until it is dismissed."
                  : "An approving review is needed before merge."}
            />}
            {summary.mergeability === "mergeable" && <ReadinessRow
              tone="ok"
              icon={GitCommitHorizontal}
              title="No conflicts with main"
              note={config ? `${MERGE_BUTTON_LABEL[config.defaultStrategy]} is the repository default` : undefined}
            />}
            {summary.mergeability === "conflicting" && <ReadinessRow
              tone="bad"
              icon={Ban}
              title="Conflicts with main"
              note={`GitHub reports its merge state as ${summary.mergeStateStatus.toLowerCase()}`}
            />}
            {summary.mergeability === "unknown" && <ReadinessRow
              tone="wait"
              icon={CircleDashed}
              title="Mergeability unknown"
            />}
          </div>
        </div>

        {/* Actions */}
        <div className={cn("u-collapse", !open && "shut")} aria-hidden={!open} {...inertWhen(!open)}>
          {/* Wraps in a narrow dock rather than pushing the pane sideways. */}
          <div className="flex flex-wrap items-center gap-1.5 px-3.5 py-3">
            <span className="inline-flex shrink-0 overflow-hidden rounded-lg bg-primary text-primary-foreground">
              <button
                type="button"
                data-merge-button
                onClick={() => dispatch({ type: "open", subject: defaultMergeSubject(summary, state.strategy) })}
                disabled={busy || !config}
                className="inline-flex h-[30px] items-center gap-1.5 px-3 text-meta font-medium transition-colors hover:bg-primary-foreground/10 disabled:opacity-50"
              >
                <GitMerge size={12} aria-hidden="true" />
                {config ? MERGE_BUTTON_LABEL[state.strategy] : "Merge"}
              </button>
              <button
                type="button"
                onClick={() => setMenuOpen(value => !value)}
                disabled={busy || !config}
                aria-haspopup="menu"
                aria-expanded={menuOpen}
                aria-label="Choose a merge strategy"
                className="inline-flex h-[30px] items-center border-l border-primary-foreground/25 px-2 transition-colors hover:bg-primary-foreground/10 disabled:opacity-50"
              >
                <ChevronDown size={12} aria-hidden="true" />
              </button>
            </span>
            {/* Approve, Review and More wrap as one group: never one alone. */}
            <span className="ml-auto flex shrink-0 items-center gap-1.5">
            <button type="button" onClick={onApprove} disabled={busy} className="inline-flex min-h-7 items-center gap-1.5 rounded-md px-2 text-meta font-medium text-foreground transition-colors hover:bg-accent disabled:opacity-50">
              <ThumbsUp size={12} aria-hidden="true" /> Approve
            </button>
            <button type="button" onClick={onToggleReview} disabled={busy || reviewBusy} aria-expanded={reviewOpen} aria-busy={reviewBusy || reviewing > 0} className={cn("inline-flex min-h-7 items-center gap-1.5 rounded-md px-2 text-meta font-medium text-foreground transition-colors hover:bg-accent disabled:opacity-50", reviewOpen && "bg-accent")}>
              {reviewBusy || reviewing > 0
                ? <LoaderCircle size={12} aria-hidden="true" className="animate-spin motion-reduce:animate-none" />
                : <Sparkles size={12} aria-hidden="true" />}
              {reviewBusy ? "Starting review…" : reviewing > 0 ? "Reviewing" : "Review"}
            </button>
            {overflow.length > 0 && <>
              <button
                ref={more.triggerRef}
                type="button"
                onClick={more.toggle}
                disabled={busy}
                aria-haspopup="menu"
                aria-expanded={more.open}
                aria-label="More pull request actions"
                title="More pull request actions"
                className="inline-grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-50"
              >
                <Ellipsis size={14} aria-hidden="true" />
              </button>
              {/* Portalled: the actions sit in a collapsing row that clips anything
                  drawn outside it, which hid this menu entirely. */}
              <MenuPanel controller={more} label="More pull request actions">
                {overflow.map(item => <MenuItem
                  key={item.label}
                  label={item.label}
                  destructive={item.destructive}
                  leading={<item.icon size={12} aria-hidden="true" className="shrink-0 opacity-70" />}
                  onClick={() => { more.close(); item.onSelect(); }}
                />)}
              </MenuPanel>
            </>}
            </span>
          </div>
        </div>

        {/* Confirm step */}
        <div className={cn("u-collapse", !showConfirm && "shut")} aria-hidden={!showConfirm} {...inertWhen(!showConfirm)}>
          <div className="px-3.5 pb-3.5 pt-2">
            <div className="flex items-center gap-2 rounded-lg bg-muted px-2.5 py-2 text-caption text-muted-foreground">
              <FileDiff size={13} className="shrink-0" aria-hidden="true" />
              <span>{lands}</span>
            </div>
            {state.strategy !== "rebase" && (
              <label className="mt-2.5 block">
                <span className="mb-1 block text-caption text-muted-foreground">Commit message</span>
                <input
                  value={state.subject}
                  onChange={event => dispatch({ type: "subject", subject: event.target.value })}
                  disabled={merging}
                  aria-label="Commit message"
                  className="h-8 w-full rounded-lg border border-input bg-background px-2.5 text-meta text-foreground outline-none focus-visible:border-ring disabled:opacity-45"
                />
              </label>
            )}
            <label className="mt-2.5 flex items-center gap-2 text-meta text-foreground">
              <input
                type="checkbox"
                checked={state.deleteBranch}
                onChange={event => dispatch({ type: "deleteBranch", value: event.target.checked })}
                disabled={merging}
                aria-label={`Delete ${headBranch} after merge`}
                className="size-[15px] shrink-0"
              />
              Delete <span className="rounded-sm bg-muted px-1.5 py-px font-mono text-caption text-foreground/80">{headBranch}</span> after merge
            </label>
            {failed && state.message && (
              <div role="alert" className="mt-2.5 flex items-start gap-2 rounded-lg bg-destructive/10 px-2.5 py-2 text-caption leading-relaxed text-destructive">
                <CircleX size={13} className="mt-px shrink-0" aria-hidden="true" />
                <span><b className="font-semibold">GitHub refused the merge.</b> {state.message}</span>
              </div>
            )}
            <div className="mt-3 flex items-center gap-2">
              <button
                type="button"
                onClick={() => dispatch({ type: "collapse" })}
                disabled={merging}
                className="inline-flex min-h-7 items-center rounded-md px-2 text-meta font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-50"
              >Cancel</button>
              <span className="flex items-center gap-1 text-micro text-muted-foreground">
                <Kbd className="border border-border bg-transparent">{modKey("↵")}</Kbd> to merge
              </span>
              <span className="flex-1" />
              <button
                type="button"
                data-merge-go
                onClick={onConfirm}
                disabled={merging || busy}
                onAnimationEnd={() => setShaking(false)}
                className={cn(
                  "relative inline-flex h-[30px] items-center overflow-hidden rounded-lg bg-primary px-3 text-meta font-medium text-primary-foreground transition-colors hover:bg-primary/90 disabled:opacity-50",
                  shaking && "merge-shake",
                )}
              >
                <span className={cn(
                  "inline-flex items-center gap-1.5 transition-[opacity,transform] duration-200 ease-[cubic-bezier(0.22,1,0.36,1)]",
                  merging && "pointer-events-none -translate-y-2.5 opacity-0",
                )}>
                  <GitMerge size={12} aria-hidden="true" />
                  {failed ? "Try again" : `Confirm ${MERGE_BUTTON_LABEL[state.strategy]}`}
                </span>
                <span className={cn(
                  "absolute inset-0 flex items-center justify-center gap-1.5 transition-[opacity,transform] duration-200 ease-[cubic-bezier(0.22,1,0.36,1)]",
                  merging ? "translate-y-0 opacity-100" : "pointer-events-none translate-y-2.5 opacity-0",
                )}>
                  Merging
                  <span className="flex gap-0.5" aria-hidden="true">
                    <span className="merge-dot" /><span className="merge-dot" /><span className="merge-dot" />
                  </span>
                </span>
              </button>
            </div>
          </div>
        </div>

        {/* Receipt */}
        <div className={cn("u-collapse", !merged && "shut")} aria-hidden={!merged} {...inertWhen(!merged)}>
          <div className={cn("px-3.5 pb-3.5 pt-3", celebrate ? "merge-graph-play" : "merge-graph-final")}>
            <MergeGraph strategy={state.strategy} commits={commitCount} baseBranch={baseBranch} headBranch={headBranch} celebrate={celebrate} />
            <div className={cn("mt-1.5 flex items-start gap-2.5", celebrate && "merge-rise d1")}>
              <span className="grid size-6 shrink-0 place-items-center rounded-full bg-merged/15 text-merged">
                <svg viewBox="0 0 24 24" className="size-3.5" aria-hidden="true">
                  <path className="badge-check" d="M20 6 9 17l-5-5" fill="none" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round" />
                </svg>
              </span>
              <span className="min-w-0">
                <span className="block text-ui font-semibold text-foreground">Merged into {baseBranch}</span>
                <span className="mt-0.5 block text-caption text-muted-foreground">{receiptLine}{celebrate ? " · just now" : ""}</span>
              </span>
            </div>
            <div className={cn("mt-3 flex items-center gap-1.5", celebrate && "merge-rise d2")}>
              <a
                href={prUrl}
                data-system-browser
                aria-label="Open the revert flow on GitHub"
                title="Revert on GitHub"
                className="inline-flex min-h-7 items-center gap-1.5 rounded-md border border-border-card bg-card px-2.5 text-meta font-medium text-foreground transition-colors hover:bg-accent"
              ><RotateCcw size={12} aria-hidden="true" /> Revert</a>
              <a
                href={`${prUrl}/commits`}
                data-system-browser
                aria-label="View the commit on GitHub"
                title="View commit on GitHub"
                className="inline-flex min-h-7 items-center gap-1.5 rounded-md border border-border-card bg-card px-2.5 text-meta font-medium text-foreground transition-colors hover:bg-accent"
              ><Eye size={12} aria-hidden="true" /> View commit</a>
              <button
                type="button"
                onClick={onRestore}
                disabled={busy}
                className="inline-flex min-h-7 items-center rounded-md px-2 text-meta font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-50"
              >Restore branch</button>
            </div>
          </div>
        </div>

        {/* The indeterminate hairline while the gh call is in flight */}
        <div className={cn("merge-hairline", merging && "merge-hairline-live")} aria-hidden="true" />
      </div>

      {/* Strategy menu, anchored above the split button */}
      <AnimatePresence>
        {menuOpen && (
          <motion.div
            role="menu"
            aria-label="Merge strategy"
            initial={{ opacity: 0, y: 6, scale: 0.97 }}
            animate={{ opacity: 1, y: 0, scale: 1, transition: { duration: 0.2, ease: ENTER_EASE } }}
            exit={{ opacity: 0, y: 6, scale: 0.97, transition: { duration: 0.2, ease: "easeIn" } }}
            style={{ transformOrigin: "20% 100%" }}
            className="absolute bottom-[calc(100%+6px)] left-3.5 z-20 w-[300px] rounded-xl border border-border-card bg-popover p-1.5 shadow-[0_16px_40px_-18px_rgb(0_0_0/0.5),0_2px_8px_-4px_rgb(0_0_0/0.22)]"
          >
            {strategies.map(strategy => (
              <button
                key={strategy}
                type="button"
                role="menuitemradio"
                aria-checked={state.strategy === strategy}
                onClick={() => { setMenuOpen(false); dispatch({ type: "pick", strategy, subject: defaultMergeSubject(summary, strategy) }); }}
                className="flex w-full items-start gap-2.5 rounded-lg px-2.5 py-2 text-left transition-colors hover:bg-accent"
              >
                <StrategyGlyph kind={strategy} />
                <span className="min-w-0 flex-1">
                  <span className="block text-meta font-medium text-foreground">{MERGE_BUTTON_LABEL[strategy]}</span>
                  <span className="mt-0.5 block text-caption leading-[1.4] text-muted-foreground">
                    {menuNote(strategy, commitCount, baseBranch, config?.defaultStrategy === strategy)}
                  </span>
                </span>
                {state.strategy === strategy && <Check size={13} aria-hidden="true" className="ml-auto mt-0.5 shrink-0 text-foreground" />}
              </button>
            ))}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
