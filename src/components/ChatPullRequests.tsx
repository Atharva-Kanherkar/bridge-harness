import { useCallback, useEffect, useId, useRef, useState, type CSSProperties, type FormEvent } from "react";
import {
  ArrowUpRight,
  ChevronDown,
  CircleCheck,
  CircleDashed,
  CircleDot,
  CircleMinus,
  CircleX,
  CloudOff,
  GitBranch,
  GitMerge,
  GitPullRequest,
  GitPullRequestClosed,
  GitPullRequestDraft,
  Link2,
  PanelRight,
  Plus,
  RotateCw,
} from "lucide-react";
import { bridgeApi } from "../api";
import { cn } from "@/lib/utils";
import type { SessionPullRequest } from "../protocol/generated/protocol";
import {
  announcement,
  chatPrKey,
  chatPrStatus,
  checkTone,
  eventTouches,
  freshness,
  orderChecks,
  refreshInterval,
  type ChatPrPhase,
  type ChatPrTone,
  type CheckTone,
} from "../chatPullRequests";

// The in-chat PR artifact. A PR the agent opened stays in its chat as a card
// that keeps moving after the turn ends: CI runs, fails, passes, the PR merges
// — and none of it appends a message to the transcript. The server poller
// does the GitHub work; this surface only re-reads it on the poller's events,
// on focus, and on a bounded cadence until every PR is terminal.

type ChatPrState = {
  prs: SessionPullRequest[];
  loaded: boolean;
  refreshing: boolean;
  /** A failed read of the whole list; per-row outages arrive as `stale`. */
  error?: string;
};

export function useChatPullRequests(sessionId: string | undefined, workspaceId: string | undefined) {
  const [state, setState] = useState<ChatPrState>({ prs: [], loaded: false, refreshing: false });
  const prsRef = useRef<SessionPullRequest[]>([]);
  const inflight = useRef(false);
  const queued = useRef(false);
  const sessionRef = useRef(sessionId);
  sessionRef.current = sessionId;

  // One read at a time per chat: a burst of poller events (checks_changed,
  // then ci_finished, then a focus) collapses to at most one trailing read.
  const load = useCallback(async (refresh: boolean) => {
    const target = sessionRef.current;
    if (!target) return;
    if (inflight.current) { queued.current = true; return; }
    inflight.current = true;
    if (refresh) setState(current => ({ ...current, refreshing: true }));
    try {
      const result = await bridgeApi.githubSessionPrs(target, refresh);
      if (sessionRef.current !== target) return;
      prsRef.current = result.pullRequests;
      setState({ prs: result.pullRequests, loaded: true, refreshing: false });
    } catch (value) {
      if (sessionRef.current !== target) return;
      // Keep the last known cards: an outage is labelled, never blanked.
      setState(current => ({ ...current, loaded: true, refreshing: false, error: value instanceof Error ? value.message : String(value) }));
    } finally {
      inflight.current = false;
      if (queued.current) { queued.current = false; void load(false); }
    }
  }, []);

  // Reopening a chat is a fresh read: whatever happened while it was closed
  // (a merge, a new push) shows up now rather than on the next poll.
  useEffect(() => {
    prsRef.current = [];
    setState({ prs: [], loaded: false, refreshing: false });
    if (sessionId) void load(true);
  }, [sessionId, load]);

  useEffect(() => {
    if (!sessionId) return;
    const unlisten: Array<Promise<() => void>> = [
      bridgeApi.onGithubSessionPrsChanged(payload => { if (payload.sessionId === sessionId) void load(false); }),
      bridgeApi.onGithubChecksChanged(payload => {
        if (payload.workspaceId === workspaceId && eventTouches(prsRef.current, payload.number)) void load(false);
      }),
      bridgeApi.onGithubCiFinished(payload => {
        if (payload.workspaceId === workspaceId && eventTouches(prsRef.current, payload.number)) void load(false);
      }),
    ];
    const onFocus = () => { if (document.visibilityState === "visible" && prsRef.current.length > 0) void load(false); };
    window.addEventListener("focus", onFocus);
    document.addEventListener("visibilitychange", onFocus);
    return () => {
      for (const pending of unlisten) void pending.then(stop => stop());
      window.removeEventListener("focus", onFocus);
      document.removeEventListener("visibilitychange", onFocus);
    };
  }, [sessionId, workspaceId, load]);

  const interval = refreshInterval(state.prs);
  useEffect(() => {
    if (interval === null) return;
    const timer = window.setInterval(() => { if (document.visibilityState !== "hidden") void load(false); }, interval);
    return () => window.clearInterval(timer);
  }, [interval, load]);

  const attach = useCallback(async (reference: string) => {
    if (!sessionRef.current) throw new Error("Open a chat first.");
    const result = await bridgeApi.githubAttachPr(sessionRef.current, reference);
    await load(false);
    return result;
  }, [load]);

  return { ...state, reload: load, attach };
}

// ── tone vocabulary ─────────────────────────────────────────────────────────

const TONE_TEXT: Record<ChatPrTone, string> = {
  merged: "text-info",
  closed: "text-muted-foreground",
  danger: "text-destructive",
  warning: "text-warning",
  success: "text-success",
  muted: "text-muted-foreground",
};

/** The soft wash behind the card's header — the only place a tone floods. */
const TONE_WASH: Record<ChatPrTone, string> = {
  merged: "from-info/[0.09]",
  closed: "from-foreground/[0.04]",
  danger: "from-destructive/[0.09]",
  warning: "from-warning/[0.07]",
  success: "from-success/[0.08]",
  muted: "from-foreground/[0.03]",
};

const TONE_DOT: Record<ChatPrTone, string> = {
  merged: "bg-info",
  closed: "bg-muted-foreground/60",
  danger: "bg-destructive",
  warning: "bg-warning",
  success: "bg-success",
  muted: "bg-muted-foreground/40",
};

const PHASE_ICON: Record<ChatPrPhase, typeof CircleCheck> = {
  merged: GitMerge,
  closed: GitPullRequestClosed,
  failing: CircleX,
  running: CircleDot,
  passing: CircleCheck,
  none: CircleDashed,
};

const CHECK_ICON: Record<CheckTone, { icon: typeof CircleCheck; className: string; label: string }> = {
  failed: { icon: CircleX, className: "text-destructive", label: "Failed" },
  running: { icon: CircleDot, className: "text-warning github-check-live", label: "Running" },
  queued: { icon: CircleDashed, className: "text-muted-foreground", label: "Queued" },
  passed: { icon: CircleCheck, className: "text-success", label: "Passed" },
  skipped: { icon: CircleMinus, className: "text-muted-foreground/70", label: "Skipped" },
};

const SEGMENT: Record<CheckTone, string> = {
  failed: "bg-destructive",
  running: "bg-warning/80 github-check-live",
  queued: "bg-foreground/10",
  passed: "bg-success/85",
  skipped: "bg-foreground/[0.18]",
};

function prIcon(pr: SessionPullRequest) {
  if (pr.state === "merged") return GitMerge;
  if (pr.state === "closed") return GitPullRequestClosed;
  return pr.isDraft ? GitPullRequestDraft : GitPullRequest;
}

// ── the ring ────────────────────────────────────────────────────────────────

/**
 * The card's glyph: the PR icon inside a ring whose arcs are the check set —
 * green passed, red failed, a breathing arc for what is still running. A
 * merged or closed PR draws one solid ring: its checks no longer matter.
 */
function StatusRing({ pr, tone }: { pr: SessionPullRequest; tone: ChatPrTone }) {
  const Icon = prIcon(pr);
  const size = 44;
  const stroke = 3;
  const radius = (size - stroke) / 2;
  const circumference = 2 * Math.PI * radius;
  const { checks } = pr;
  const terminal = pr.state !== "open";
  const arcs: Array<{ value: number; className: string }> = terminal || checks.total === 0 ? [] : [
    { value: checks.passed, className: "stroke-success" },
    { value: checks.failed, className: "stroke-destructive" },
    { value: checks.inProgress, className: "stroke-warning github-check-live" },
    { value: checks.skipped + checks.cancelled, className: "stroke-foreground/25" },
  ].filter(arc => arc.value > 0);
  const gap = arcs.length > 1 ? 3 : 0;
  let offset = 0;
  return <span className="relative grid size-11 shrink-0 place-items-center">
    <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} className="absolute inset-0 -rotate-90" aria-hidden="true">
      <circle cx={size / 2} cy={size / 2} r={radius} fill="none" strokeWidth={stroke} className={cn("stroke-foreground/[0.08]", checks.total === 0 && !terminal && "[stroke-dasharray:3_4]")} />
      {terminal && <circle cx={size / 2} cy={size / 2} r={radius} fill="none" strokeWidth={stroke} className={pr.state === "merged" ? "stroke-info" : "stroke-muted-foreground/50"} />}
      {arcs.map((arc, index) => {
        const length = Math.max(0, (arc.value / checks.total) * circumference - gap);
        const element = <circle
          key={index}
          cx={size / 2}
          cy={size / 2}
          r={radius}
          fill="none"
          strokeWidth={stroke}
          strokeLinecap="round"
          strokeDasharray={`${length} ${circumference}`}
          strokeDashoffset={-offset}
          className={cn(arc.className, "transition-[stroke-dasharray,stroke-dashoffset] duration-700 ease-[cubic-bezier(0.22,1,0.36,1)]")}
        />;
        offset += (arc.value / checks.total) * circumference;
        return element;
      })}
    </svg>
    <span className={cn("grid size-8 place-items-center rounded-full bg-card", TONE_TEXT[tone])}>
      <Icon size={15} strokeWidth={2} aria-hidden="true" />
    </span>
  </span>;
}

/** One segment per check, in list order: a CI run you can read at a glance. */
function CheckStrip({ pr }: { pr: SessionPullRequest }) {
  const ordered = orderChecks(pr.checkDetails);
  if (ordered.length === 0) return null;
  return <div className="flex h-1.5 w-full gap-[3px]" aria-hidden="true">
    {ordered.map((check, index) => <span
      key={`${check.workflow}/${check.name}/${index}`}
      title={`${check.name} · ${CHECK_ICON[checkTone(check)].label.toLowerCase()}`}
      className={cn("min-w-1 flex-1 rounded-full transition-colors duration-500", SEGMENT[checkTone(check)])}
    />)}
  </div>;
}

function CountLegend({ pr }: { pr: SessionPullRequest }) {
  const { checks } = pr;
  const parts: Array<[number, string, string]> = [
    [checks.passed, "passed", "bg-success"],
    [checks.failed, "failed", "bg-destructive"],
    [checks.inProgress, "running", "bg-warning"],
    [checks.queued, "queued", "bg-foreground/20"],
    [checks.skipped + checks.cancelled, "skipped", "bg-foreground/25"],
  ];
  const shown = parts.filter(([count]) => count > 0);
  if (shown.length === 0) return null;
  return <ul className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] text-muted-foreground" aria-label="Check counts">
    {shown.map(([count, label, dot]) => <li key={label} className="inline-flex items-center gap-1.5">
      <span className={cn("size-1.5 rounded-full", dot)} aria-hidden="true" />
      <span className="tabular-nums text-foreground/80">{count}</span> {label}
    </li>)}
  </ul>;
}

// ── the card ────────────────────────────────────────────────────────────────

const ICON_BUTTON = "inline-grid size-7 place-items-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring active:scale-95";

function PullRequestCard({ pr, now, onOpenPane, onRetry, retrying }: {
  pr: SessionPullRequest;
  now: number;
  onOpenPane?: (number: number, tab: "conversation" | "checks") => void;
  onRetry: () => void;
  retrying: boolean;
}) {
  const status = chatPrStatus(pr);
  const [open, setOpen] = useState(false);
  const listId = useId();
  const PhaseIcon = PHASE_ICON[status.phase];
  const ordered = orderChecks(pr.checkDetails);
  const failures = ordered.filter(check => checkTone(check) === "failed");

  return <article
    aria-label={`Pull request #${pr.number}: ${pr.title}. ${status.headline}.`}
    data-pr-phase={status.phase}
    className="animate-home-rise relative overflow-hidden rounded-2xl border border-border-card bg-card shadow-[0_1px_2px_rgb(0_0_0/0.04),0_12px_32px_-16px_rgb(0_0_0/0.18)]"
  >
    <div aria-hidden="true" className={cn("pointer-events-none absolute inset-x-0 top-0 h-28 bg-linear-to-b to-transparent transition-colors duration-700", TONE_WASH[status.tone])} />
    <div className="relative px-4 pb-3.5 pt-4 sm:px-5">
      <div className="flex items-start gap-3.5">
        <StatusRing pr={pr} tone={status.tone} />
        <div className="min-w-0 flex-1">
          <p className="flex items-center gap-1.5 text-[10.5px] font-medium uppercase tracking-[0.12em] text-muted-foreground">
            <span>Pull request</span>
            <span aria-hidden="true" className="text-muted-foreground/50">·</span>
            <span className="normal-case tracking-normal">{pr.attribution === "manual" ? "attached" : "opened in this chat"}</span>
          </p>
          <h3 className="mt-0.5 line-clamp-2 font-display text-[15px] font-semibold leading-snug tracking-[-0.01em] text-foreground">
            <span className="tabular-nums text-muted-foreground">#{pr.number}</span> {pr.title}
          </h3>
          <p className="mt-1.5 flex min-w-0 flex-wrap items-center gap-1.5 text-[11px] text-muted-foreground">
            <span className="inline-flex min-w-0 max-w-full items-center gap-1 rounded-md border border-border bg-background/60 px-1.5 py-0.5 font-mono text-[10.5px] text-foreground/80">
              <GitBranch size={10.5} className="shrink-0" aria-hidden="true" />
              <span className="truncate">{pr.headBranch}</span>
            </span>
            {pr.headSha && <span className="font-mono text-[10.5px] text-muted-foreground/80" title={pr.headSha}>{pr.headSha.slice(0, 7)}</span>}
            {pr.isDraft && pr.state === "open" && <span className="rounded-full border border-border px-1.5 py-px text-[10px] font-medium text-muted-foreground">Draft</span>}
          </p>
        </div>
        <div className="-mr-1.5 -mt-1 flex shrink-0 items-center">
          {onOpenPane && <button type="button" className={ICON_BUTTON} onClick={() => onOpenPane(pr.number, "conversation")} aria-label={`Open #${pr.number} in the GitHub pane`} title="Open in GitHub pane">
            <PanelRight size={14} aria-hidden="true" />
          </button>}
          <a className={ICON_BUTTON} href={pr.url} target="_blank" rel="noreferrer" aria-label={`Open #${pr.number} on GitHub`} title="Open on GitHub">
            <ArrowUpRight size={14} aria-hidden="true" />
          </a>
        </div>
      </div>

      <div className="mt-3.5 flex items-center gap-2">
        <PhaseIcon size={14} aria-hidden="true" className={cn("shrink-0", TONE_TEXT[status.tone], status.live && "github-check-live")} />
        <p className={cn("min-w-0 flex-1 truncate text-[13px] font-medium", status.tone === "muted" || status.tone === "closed" ? "text-foreground/80" : TONE_TEXT[status.tone])}>
          {status.headline}
        </p>
        <span className="shrink-0 text-[11px] tabular-nums text-muted-foreground" title={pr.fetchedAt ?? undefined}>
          {pr.stale ? "last known · " : "updated "}{freshness(pr.fetchedAt, now)}
        </span>
      </div>

      {pr.state === "open" && pr.checkDetails.length > 0 && <div className="mt-2.5 grid gap-2">
        <CheckStrip pr={pr} />
        <CountLegend pr={pr} />
      </div>}

      {pr.stale && <div role="status" className="mt-3 flex items-center gap-2 rounded-xl border border-border bg-background/70 px-3 py-2 text-[11.5px] text-muted-foreground">
        <CloudOff size={13} className="shrink-0" aria-hidden="true" />
        <span className="min-w-0 flex-1">GitHub didn’t answer, so this is the last state Bridge saw{pr.error ? <span className="block truncate text-[10.5px] text-muted-foreground/80" title={pr.error}>{pr.error}</span> : null}</span>
        <button type="button" onClick={onRetry} disabled={retrying} className="inline-flex min-h-7 shrink-0 items-center gap-1 rounded-lg border border-border bg-card px-2 text-[11.5px] font-medium text-foreground transition-colors hover:bg-accent disabled:opacity-60">
          <RotateCw size={11.5} className={cn(retrying && "animate-spin")} aria-hidden="true" /> Retry
        </button>
      </div>}

      {pr.checkDetails.length > 0 && <div className="mt-3 border-t border-border/80 pt-2">
        <button
          type="button"
          aria-expanded={open}
          aria-controls={listId}
          onClick={() => setOpen(value => !value)}
          className="-mx-2 flex w-[calc(100%+1rem)] items-center gap-2 rounded-lg px-2 py-1.5 text-left text-[12px] text-muted-foreground transition-colors hover:bg-accent/70 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <ChevronDown size={13} aria-hidden="true" className={cn("shrink-0 transition-transform duration-200", open ? "rotate-0" : "-rotate-90")} />
          <span className="flex-1">{open ? "Hide checks" : `Show ${pr.checkDetails.length} check${pr.checkDetails.length === 1 ? "" : "s"}`}</span>
          {!open && failures.length > 0 && <span className="truncate text-[11px] text-destructive">{failures.map(check => check.name).slice(0, 2).join(", ")}{failures.length > 2 ? ` +${failures.length - 2}` : ""}</span>}
        </button>
        <div id={listId} hidden={!open}>
          {open && <ul className="mt-1 grid gap-px">
            {ordered.map((check, index) => {
              const tone = checkTone(check);
              const { icon: Icon, className, label } = CHECK_ICON[tone];
              return <li
                key={`${check.workflow}/${check.name}/${index}`}
                className="github-row-enter group/check flex min-h-8 items-center gap-2.5 rounded-lg px-2 py-1 text-[12px] hover:bg-accent/60"
                style={{ "--github-row-delay": `${Math.min(index, 10) * 18}ms` } as CSSProperties}
              >
                <Icon size={13} className={cn("shrink-0", className)} aria-label={label} />
                <span className="min-w-0 flex-1 truncate text-foreground">{check.name}</span>
                <span className="hidden shrink-0 truncate text-[11px] text-muted-foreground sm:inline">{check.workflow}</span>
                {check.logUrl
                  ? <a href={check.logUrl} target="_blank" rel="noreferrer" className={cn("inline-flex shrink-0 items-center gap-0.5 rounded-md px-1.5 py-0.5 text-[11px] transition-colors hover:bg-accent hover:text-foreground", tone === "failed" ? "font-medium text-destructive" : "text-muted-foreground opacity-0 group-hover/check:opacity-100 focus-visible:opacity-100")} aria-label={`Logs for ${check.name}`}>
                      Logs <ArrowUpRight size={11} aria-hidden="true" />
                    </a>
                  : <span className="w-11 shrink-0" aria-hidden="true" />}
              </li>;
            })}
          </ul>}
          {open && onOpenPane && <button type="button" onClick={() => onOpenPane(pr.number, "checks")} className="mt-1 inline-flex min-h-7 items-center gap-1 rounded-lg px-2 text-[11.5px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">
            <PanelRight size={11.5} aria-hidden="true" /> Open checks in the GitHub pane
          </button>}
        </div>
      </div>}
    </div>
  </article>;
}

// ── attach ──────────────────────────────────────────────────────────────────

export function AttachPullRequest({ onAttach, variant = "inline" }: { onAttach: (reference: string) => Promise<unknown>; variant?: "inline" | "empty" }) {
  const [open, setOpen] = useState(false);
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const inputId = useId();
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (!value.trim()) return;
    setBusy(true); setError(undefined);
    try {
      await onAttach(value.trim());
      setValue(""); setOpen(false);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally { setBusy(false); }
  };
  if (!open) {
    return <button type="button" onClick={() => setOpen(true)} className={cn("inline-flex min-h-7 items-center gap-1.5 rounded-lg px-2 text-[11.5px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground", variant === "empty" && "border border-dashed border-border")}>
      <Plus size={12} aria-hidden="true" /> Attach a pull request
    </button>;
  }
  return <form onSubmit={submit} className="animate-page-mount flex flex-col gap-1.5 rounded-xl border border-border bg-card p-2 shadow-sm">
    <label htmlFor={inputId} className="sr-only">Pull request URL or number</label>
    <div className="flex items-center gap-1.5">
      <Link2 size={13} className="ml-1 shrink-0 text-muted-foreground" aria-hidden="true" />
      <input
        id={inputId}
        autoFocus
        value={value}
        onChange={event => setValue(event.target.value)}
        onKeyDown={event => { if (event.key === "Escape") { setOpen(false); setError(undefined); } }}
        placeholder="github.com/owner/repo/pull/123 or #123"
        className="min-w-0 flex-1 bg-transparent px-1 py-1 text-[12.5px] text-foreground outline-none placeholder:text-muted-foreground/70"
        aria-invalid={!!error}
        aria-describedby={error ? `${inputId}-error` : undefined}
        disabled={busy}
      />
      <button type="submit" disabled={busy || !value.trim()} className="inline-flex min-h-7 items-center rounded-lg bg-primary px-2.5 text-[11.5px] font-medium text-primary-foreground transition-opacity hover:opacity-90 disabled:opacity-50">
        {busy ? "Verifying…" : "Attach"}
      </button>
      <button type="button" onClick={() => { setOpen(false); setError(undefined); }} className="inline-flex min-h-7 items-center rounded-lg px-2 text-[11.5px] text-muted-foreground hover:bg-accent hover:text-foreground">Cancel</button>
    </div>
    {error && <p id={`${inputId}-error`} role="alert" className="px-1 text-[11px] text-destructive">{error}</p>}
  </form>;
}

// ── the transcript section ──────────────────────────────────────────────────

export function ChatPullRequestCards({ prs, onOpenPane, onRetry, onAttach, refreshing, anchorRef }: {
  prs: SessionPullRequest[];
  onOpenPane?: (number: number, tab: "conversation" | "checks") => void;
  onRetry: () => void;
  onAttach: (reference: string) => Promise<unknown>;
  refreshing: boolean;
  anchorRef?: (element: HTMLElement | null) => void;
}) {
  const now = useNow(prs.length > 0 ? 15_000 : null);
  const spoken = useStatusAnnouncements(prs);
  if (prs.length === 0) return null;
  return <section ref={anchorRef} id="chat-pull-requests" aria-label="Pull requests from this chat" className="mt-2 mb-6 grid w-full scroll-mt-6 gap-2.5">
    {prs.map(pr => <PullRequestCard key={chatPrKey(pr)} pr={pr} now={now} onOpenPane={onOpenPane} onRetry={onRetry} retrying={refreshing} />)}
    <div className="flex justify-start">
      <AttachPullRequest onAttach={onAttach} />
    </div>
    <p aria-live="polite" className="sr-only">{spoken}</p>
  </section>;
}

// ── the strip above the composer ────────────────────────────────────────────

/** A single line that says how the chat's PR is doing while its card is off
 * screen. Clicking it scrolls the card into view. */
export function ChatPullRequestStrip({ prs, hidden, onJump }: { prs: SessionPullRequest[]; hidden: boolean; onJump: () => void }) {
  const lead = prs[0];
  if (!lead || hidden) return null;
  const status = chatPrStatus(lead);
  const others = prs.length - 1;
  const { checks } = lead;
  const settled = checks.total - checks.queued - checks.inProgress;
  return <div className="mx-auto mb-2 flex max-w-conversation justify-center px-4 sm:px-6">
    <button
      type="button"
      onClick={onJump}
      className="u-glass-soft animate-home-rise group/strip inline-flex h-[30px] max-w-full items-center gap-2 rounded-full pl-2.5 pr-3 text-xs text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      aria-label={`Pull request #${lead.number}: ${status.headline}${lead.stale ? ", last known state" : ""}. Jump to card.`}
    >
      <span className="relative grid size-2 shrink-0 place-items-center" aria-hidden="true">
        {status.live && <span className="absolute inset-0 scale-[2.4] opacity-30"><span className={cn("block size-full rounded-full github-check-live", TONE_DOT[status.tone])} /></span>}
        <span className={cn("size-2 rounded-full", TONE_DOT[status.tone])} />
      </span>
      <span className="shrink-0 tabular-nums font-medium text-foreground">#{lead.number}</span>
      <span className={cn("shrink-0", TONE_TEXT[status.tone] === "text-muted-foreground" ? "text-foreground/80" : TONE_TEXT[status.tone])}>{status.summary}</span>
      {lead.state === "open" && checks.total > 0 && status.phase !== "running" && <span className="shrink-0 tabular-nums text-muted-foreground/80">{settled}/{checks.total}</span>}
      <span className="hidden min-w-0 truncate sm:inline">{lead.title}</span>
      {lead.stale && <CloudOff size={11} className="shrink-0" aria-hidden="true" />}
      {others > 0 && <span className="shrink-0 rounded-full bg-foreground/[0.06] px-1.5 text-[10.5px] tabular-nums">+{others}</span>}
      <ChevronDown size={12} className="shrink-0 transition-transform group-hover/strip:translate-y-px" aria-hidden="true" />
    </button>
  </div>;
}

/** Whether the cards section is on screen, for hiding the strip beside it. */
export function useInView() {
  const [inView, setInView] = useState(false);
  const observer = useRef<IntersectionObserver | null>(null);
  const ref = useCallback((element: HTMLElement | null) => {
    observer.current?.disconnect();
    observer.current = null;
    if (!element || typeof IntersectionObserver === "undefined") { setInView(false); return; }
    observer.current = new IntersectionObserver(entries => {
      setInView(entries.some(entry => entry.isIntersecting));
    }, { threshold: 0.15 });
    observer.current.observe(element);
  }, []);
  // No unmount effect: React calls the ref with null on unmount, which
  // disconnects above — and a StrictMode effect replay would otherwise tear
  // the observer down without a ref call to rebuild it.
  return [ref, inView] as const;
}

export function jumpToChatPullRequests() {
  document.getElementById("chat-pull-requests")?.scrollIntoView({ behavior: "smooth", block: "center" });
}

// ── small hooks ─────────────────────────────────────────────────────────────

function useNow(interval: number | null) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    setNow(Date.now());
    if (interval === null) return;
    const timer = window.setInterval(() => setNow(Date.now()), interval);
    return () => window.clearInterval(timer);
  }, [interval]);
  return now;
}

/** The live region's text changes only on a phase transition, so a poll that
 * lands the same state stays silent. */
function useStatusAnnouncements(prs: SessionPullRequest[]) {
  const phases = useRef(new Map<string, ChatPrPhase>());
  const [spoken, setSpoken] = useState("");
  useEffect(() => {
    const lines: string[] = [];
    for (const pr of prs) {
      const key = chatPrKey(pr);
      const line = announcement(phases.current.get(key), pr);
      if (line) lines.push(line);
      phases.current.set(key, chatPrStatus(pr).phase);
    }
    if (lines.length > 0) setSpoken(lines.join(" "));
  }, [prs]);
  return spoken;
}
