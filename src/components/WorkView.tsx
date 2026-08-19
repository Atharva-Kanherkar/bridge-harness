import { useCallback, useMemo, useRef, useState } from "react";
import { AlertCircle, CircleCheck, GitBranch, ListOrdered, RefreshCw, ShieldCheck, X } from "lucide-react";
import type { WorkBoard, WorkFact, WorkFactAction } from "../protocol/generated/protocol";
import { cn } from "@/lib/utils";
import {
  actionIsPrimary,
  actionLabel,
  bandFacts,
  detailIsDimmed,
  factAnnouncement,
  freshnessText,
  needsYouCount,
  RETRY_LABEL,
  SEVERITY_CAPTION,
  SEVERITY_LABEL,
  SOURCE_BY_KIND,
  SOURCE_LABEL,
  stalenessClause,
  type FactSource,
} from "./workFacts";

// The Work board. Prop-driven: it is handed a board and four callbacks and holds no
// data of its own, so every state below is reachable from a test without a backend.
//
// The board arrives ordered, so nothing here sorts. Severity is a 3px edge and one
// small word rather than a tinted panel — five blocking facts should not render as a
// wall of red — and freshness is carried three ways at once, because any single tell
// can be missed.

const SOURCE_ICON: Record<FactSource, typeof CircleCheck> = {
  check: CircleCheck,
  approval: ShieldCheck,
  queue: ListOrdered,
  branch: GitBranch,
};

/** The severity edge. A hairline, not a fill. */
const SEVERITY_EDGE: Record<WorkFact["severity"], string> = {
  blocking: "bg-destructive",
  attention: "bg-warning",
  info: "bg-info",
};

const SEVERITY_INK: Record<WorkFact["severity"], string> = {
  blocking: "text-destructive",
  attention: "text-warning",
  info: "text-info",
};

export type WorkActionOutcome = { ok: true } | { ok: false; reason: string };

export type WorkViewProps = {
  /** The board, or `undefined` while it is being read for the first time. */
  board?: WorkBoard;
  /** Why the board could not be read at all. Only set when there is nothing to show:
   * a failure that arrives while a board is on screen is a `refreshError`. */
  error?: string;
  /** A re-read that failed while a board was already rendered. Shown as a line rather
   * than replacing the board, because the numbers on screen are still the last thing
   * Bridge actually read. */
  refreshError?: string;
  /** Re-read the board. */
  onRefresh: () => void;
  /** Perform a fact's action. Resolving with `ok: false` attaches the reason to the
   * row rather than replacing it, because the fact is still true. */
  onAction: (action: WorkFactAction) => Promise<WorkActionOutcome>;
  /** Fixed clock, so freshness copy is deterministic in tests. */
  now?: Date;
};

function Chip({ children }: { children: React.ReactNode }) {
  return (
    <span className="inline-flex max-w-full items-center gap-1 truncate rounded border border-border px-1.5 py-px text-[10.5px] text-muted-foreground">
      {children}
    </span>
  );
}

/** The dot beside the freshness line. Hollow when nothing was measured, because an
 * empty reading should not look like a filled one. */
function FreshnessDot({ freshness }: { freshness: WorkFact["freshness"] }) {
  if (freshness === "unknown") {
    return <span aria-hidden="true" className="h-[5px] w-[5px] shrink-0 rounded-full ring-1 ring-muted-foreground" />;
  }
  return (
    <span
      aria-hidden="true"
      className={cn("h-[5px] w-[5px] shrink-0 rounded-full", freshness === "stale" ? "bg-warning" : "bg-success")}
    />
  );
}

function FactRow({
  fact,
  now,
  onAction,
}: {
  fact: WorkFact;
  now: Date;
  onAction: (action: WorkFactAction) => Promise<WorkActionOutcome>;
}) {
  const [failure, setFailure] = useState<string>();
  const [busy, setBusy] = useState(false);
  // `disabled={busy}` is state, so it is not in effect until the next render — two
  // clicks in the same tick would both get through and fire two fast-forwards. The
  // ref closes that window synchronously.
  const running = useRef(false);
  const source = SOURCE_BY_KIND[fact.kind];
  const Icon = SOURCE_ICON[source];
  const stale = stalenessClause(fact, now);

  const run = useCallback(async () => {
    if (running.current) return;
    running.current = true;
    setBusy(true);
    try {
      const outcome = await onAction(fact.action);
      // A failure attaches to the row. The fact is still true — only the attempt
      // failed — so removing or replacing the row would be a lie about the state.
      setFailure(outcome.ok ? undefined : outcome.reason);
    } catch (error) {
      // The handler is supposed to return an outcome rather than throw, but a button
      // stuck disabled forever is the worst way to find out that it did.
      setFailure(error instanceof Error ? error.message : String(error));
    } finally {
      running.current = false;
      setBusy(false);
    }
  }, [fact.action, onAction]);

  return (
    <li className="flex flex-wrap gap-3 rounded-xl border border-border bg-card pr-3 sm:flex-nowrap sm:py-2.5">
      <span aria-hidden="true" className={cn("w-[3px] shrink-0 self-stretch rounded-r-sm", SEVERITY_EDGE[fact.severity])} />
      <span aria-hidden="true" className="mt-2.5 flex size-6.5 shrink-0 items-center justify-center rounded-md bg-muted sm:mt-0.5">
        <Icon size={14} strokeWidth={1.7} className="text-muted-foreground" />
      </span>
      <div className="min-w-0 flex-1 pt-2.5 sm:pt-0">
        <div className="flex flex-wrap items-center gap-2">
          <span className={cn("text-[10px] font-semibold uppercase tracking-wide", SEVERITY_INK[fact.severity])}>
            {SEVERITY_LABEL[fact.severity]}
          </span>
          <span className="text-[10.5px] font-medium text-muted-foreground">{SOURCE_LABEL[source]}</span>
        </div>
        {/* The accessible name says severity, source and freshness in words, so the
            row reads the same with no colour at all. */}
        <p className="mt-0.5 text-[12.5px] font-medium leading-snug [overflow-wrap:anywhere]">
          <span className="sr-only">{factAnnouncement(fact, now)}</span>
          <span aria-hidden="true">{fact.title}</span>
        </p>
        {fact.detail && (
          <p className={cn("mt-1 text-[11.5px] leading-relaxed text-muted-foreground [overflow-wrap:anywhere]", detailIsDimmed(fact) && "opacity-70")}>
            {fact.detail}
          </p>
        )}
        {stale && <p className="mt-1 text-[11.5px] leading-relaxed text-muted-foreground">{stale}</p>}
        {failure && (
          <div id={`${fact.dedupeKey}-failure`} className="mt-2 flex gap-2 rounded-lg border border-border border-l-[3px] border-l-destructive px-2.5 py-2">
            <AlertCircle size={13} strokeWidth={1.8} className="mt-px shrink-0 text-destructive" aria-hidden="true" />
            <p className="text-[11.5px] leading-relaxed text-muted-foreground">{failure}</p>
          </div>
        )}
        <div className="mt-1.5 flex flex-wrap items-center gap-2">
          {fact.target.kind === "workspace" && <Chip>{fact.target.workspaceId}</Chip>}
          {fact.target.kind === "completionAttempt" && <Chip>attempt {fact.target.attemptId}</Chip>}
          {fact.target.kind === "workerQueueItem" && <Chip>queued {fact.target.queueId}</Chip>}
          <span className={cn("inline-flex items-center gap-1.5 text-[10.5px]", fact.freshness === "stale" ? "text-warning" : "text-muted-foreground")}>
            <FreshnessDot freshness={fact.freshness} />
            {freshnessText(fact, now)}
          </span>
        </div>
      </div>
      {/* Below sm the action gets its own row under a hairline, because at 420px a
          button beside a wrapping title leaves neither enough room. */}
      <div className="mt-2.5 flex w-full shrink-0 justify-end border-t border-border pb-2.5 pt-2.5 sm:mt-0 sm:w-auto sm:border-0 sm:pb-0 sm:pt-0.5">
        <button
          type="button"
          onClick={() => void run()}
          disabled={busy}
          aria-describedby={failure ? `${fact.dedupeKey}-failure` : undefined}
          className={cn(
            "h-7 shrink-0 rounded-md px-2.5 text-[11.5px] font-medium transition-colors disabled:opacity-60",
            actionIsPrimary(fact) && !failure
              ? "bg-primary text-primary-foreground hover:opacity-90"
              : "border border-border text-foreground hover:bg-accent",
          )}
        >
          {failure ? RETRY_LABEL : actionLabel(fact.action)}
        </button>
      </div>
    </li>
  );
}

/** Rows in the shape of the answer. A local SQLite read is fast enough that a
 * spinner would flash, and three placeholders read as "nearly there". */
function LoadingRows() {
  return (
    <ul aria-label="Loading work" className="flex flex-col gap-1.5">
      {[
        { key: "first", title: "w-1/2", detail: "w-3/4" },
        { key: "second", title: "w-2/5", detail: "w-3/5" },
        { key: "third", title: "w-7/12", detail: "w-2/3" },
      ].map((widths, index) => (
        <li key={widths.key} className="flex gap-3 rounded-xl border border-border bg-card p-2.5">
          <span className="w-[3px] shrink-0 self-stretch rounded-r-sm bg-muted" />
          <span className="size-6.5 shrink-0 rounded-md bg-muted" />
          <div className="flex-1 space-y-2 pt-1">
            <span className={cn("block h-2 rounded-full bg-muted", widths.title)} />
            <span className={cn("block h-1.5 rounded-full bg-muted", widths.detail)} />
          </div>
          <span className="sr-only">Reading item {index + 1}</span>
        </li>
      ))}
    </ul>
  );
}

function Panel({ title, body, action }: { title: string; body: string; action: React.ReactNode }) {
  return (
    <div className="rounded-xl border border-border bg-card px-5 py-6 text-center">
      <h3 className="text-[13px] font-semibold">{title}</h3>
      <p className="mx-auto mt-1 max-w-[44ch] text-[12px] leading-relaxed text-muted-foreground">{body}</p>
      <div className="mt-3 flex justify-center gap-2">{action}</div>
    </div>
  );
}

function GhostButton({ children, onClick }: { children: React.ReactNode; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="inline-flex h-6.5 items-center gap-1.5 rounded-md border border-border px-2.5 text-[11.5px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
    >
      {children}
    </button>
  );
}

export function WorkView({ board, error, refreshError, onRefresh, onAction, now = new Date() }: WorkViewProps) {
  const [noticeDismissed, setNoticeDismissed] = useState(false);
  const bands = useMemo(() => bandFacts(board?.facts ?? []), [board]);
  // The same set the rail badge counts. Info is "worth knowing, not worth
  // interrupting for", so a board holding only info facts is not waiting on you —
  // and the two numbers must not disagree about that.
  const count = needsYouCount(board?.facts ?? []);
  const anyFacts = (board?.facts.length ?? 0) > 0;
  // Suggested work has no runner yet, so this is a statement about the product
  // rather than a call to action: the facts are complete without a model.
  const showNotice = !noticeDismissed && board?.suggestions.state === "not_configured";

  return (
    <section aria-label="Work" className="flex min-h-0 flex-1 flex-col overflow-y-auto">
      <header className="flex items-start gap-3 px-5 pb-3 pt-5">
        <div className="min-w-0">
          <h2 className="text-[17px] font-semibold tracking-tight">Needs you</h2>
          <p aria-live="polite" className="mt-0.5 text-[12px] leading-snug text-muted-foreground">
            {error
              ? "The board could not be read."
              : board === undefined
                ? "Reading what needs you."
                : count === 0
                  ? anyFacts
                    ? "Nothing is waiting on you. What is below is worth knowing, not urgent."
                    : "Nothing is waiting on you."
                  : `${count} ${count === 1 ? "thing needs" : "things need"} you. Nothing was started to build this list.`}
          </p>
        </div>
        <div className="ml-auto shrink-0">
          <GhostButton onClick={onRefresh}>
            <RefreshCw size={12} strokeWidth={1.8} aria-hidden="true" />
            Refresh
          </GhostButton>
        </div>
      </header>

      {showNotice && (
        <div className="mx-5 mb-2.5 flex items-center gap-2 rounded-lg border border-border bg-card px-2.5 py-2">
          <AlertCircle size={13} strokeWidth={1.6} className="shrink-0 text-muted-foreground" aria-hidden="true" />
          <p className="min-w-0 flex-1 text-[11.5px] leading-relaxed text-muted-foreground">
            Suggested work is off until a briefing model is set up. The facts below do not need one.
          </p>
          <button
            type="button"
            onClick={() => setNoticeDismissed(true)}
            aria-label="Dismiss the suggested work notice"
            className="shrink-0 rounded p-0.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          >
            <X size={12} strokeWidth={1.8} aria-hidden="true" />
          </button>
        </div>
      )}

      {refreshError && board !== undefined && (
        <div className="mx-5 mb-2.5 flex items-start gap-2 rounded-lg border border-border border-l-[3px] border-l-destructive px-2.5 py-2">
          <AlertCircle size={13} strokeWidth={1.8} className="mt-px shrink-0 text-destructive" aria-hidden="true" />
          <p className="min-w-0 flex-1 text-[11.5px] leading-relaxed text-muted-foreground">
            <span className="font-medium text-foreground">Could not re-read the board.</span> {refreshError} What is below is the last thing Bridge read.
          </p>
        </div>
      )}

      <div className="flex flex-col gap-1.5 px-5 pb-5">
        {board === undefined && error ? (
          <Panel
            title="Work could not be read"
            body="The local database did not answer. Nothing is wrong with your sessions — this screen only reads, so retrying is safe."
            action={<GhostButton onClick={onRefresh}>{RETRY_LABEL}</GhostButton>}
          />
        ) : board === undefined ? (
          <LoadingRows />
        ) : !anyFacts ? (
          <Panel
            title="Nothing needs you"
            body="No failed checks, no unanswered approvals, nothing parked, and every workspace is close to its base."
            action={<GhostButton onClick={onRefresh}>Refresh</GhostButton>}
          />
        ) : (
          bands.map(band => (
            <section key={band.severity} aria-labelledby={`work-band-${band.severity}`}>
              <h3
                id={`work-band-${band.severity}`}
                className="mb-1 mt-2.5 flex items-center gap-2 pl-0.5 text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground first:mt-0"
              >
                {SEVERITY_LABEL[band.severity]}
                <span className="font-normal normal-case tracking-normal opacity-65">— {SEVERITY_CAPTION[band.severity]}</span>
              </h3>
              <ul aria-label={`${SEVERITY_LABEL[band.severity]} work`} className="flex flex-col gap-1.5">
                {band.facts.map(fact => (
                  <FactRow key={fact.dedupeKey} fact={fact} now={now} onAction={onAction} />
                ))}
              </ul>
            </section>
          ))
        )}
      </div>
    </section>
  );
}

export default WorkView;
