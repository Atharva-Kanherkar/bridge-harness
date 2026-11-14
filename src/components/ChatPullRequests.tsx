import { useCallback, useEffect, useId, useRef, useState, type FormEvent } from "react";
import { CircleCheck, CircleDashed, CircleDot, CircleMinus, CircleX, GitMerge, GitPullRequestArrow, GitPullRequestClosed, Link2, Plus } from "lucide-react";
import { bridgeApi } from "../api";
import { cn } from "@/lib/utils";
import type { SessionPullRequest } from "../protocol/generated/protocol";
import {
  announcement,
  chatPrKey,
  chatPrStatus,
  eventTouches,
  refreshInterval,
  type ChatPrPhase,
  type CheckTone,
} from "../chatPullRequests";

// A chat's pull requests, kept live after the turn ends: CI runs, fails,
// passes, the PR merges, and none of it appends a message to the transcript.
// The server poller does the GitHub work; this hook only re-reads it on the
// poller's events, on focus, and on a bounded cadence until every PR is
// terminal. The composer shelf and the Work pane draw what it returns.

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
  const queued = useRef<boolean | null>(null);
  const sessionRef = useRef(sessionId);
  sessionRef.current = sessionId;

  // One read at a time per chat: a burst of poller events (checks_changed,
  // then ci_finished, then a focus) collapses to at most one trailing read.
  const load = useCallback(async (refresh: boolean) => {
    const target = sessionRef.current;
    if (!target) return;
    if (inflight.current) { queued.current = (queued.current ?? false) || refresh; return; }
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
      if (queued.current !== null) { const refreshNext = queued.current; queued.current = null; void load(refreshNext); }
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
    const onFocus = () => { if (document.visibilityState === "visible") void load(true); };
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

/** Pull requests linked to chats, keyed by session id. */
export type SidebarPullRequests = Readonly<Record<string, SessionPullRequest[]>>;

const SIDEBAR_FOCUS_THROTTLE_MS = 60_000;
const SIDEBAR_OPEN_INTERVAL_MS = 120_000;

/**
 * Every chat's linked PRs for the chat list. The first read is snapshot-only
 * (no `gh`), so rows paint at once; a refresh then re-reads only open PRs.
 * Poller events and focus refresh again, and an interval keeps open PRs moving
 * while any remain. Merged and closed PRs are never re-read.
 */
export function useSidebarPullRequests(): SidebarPullRequests {
  const [byChat, setByChat] = useState<SidebarPullRequests>({});
  const numbers = useRef(new Set<number>());
  const inflight = useRef(false);
  const queued = useRef<boolean | null>(null);
  const lastRefresh = useRef(0);

  const load = useCallback(async (refresh: boolean) => {
    if (inflight.current) { queued.current = (queued.current ?? false) || refresh; return; }
    inflight.current = true;
    if (refresh) lastRefresh.current = Date.now();
    try {
      const result = await bridgeApi.githubSidebarPrs(refresh);
      const next: Record<string, SessionPullRequest[]> = {};
      numbers.current = new Set();
      for (const chat of result.chats) {
        next[chat.sessionId] = chat.pullRequests;
        for (const pr of chat.pullRequests) numbers.current.add(pr.number);
      }
      setByChat(next);
    } catch {
      // Keep the last known chips: the list is a hint, never a blocker.
    } finally {
      inflight.current = false;
      if (queued.current !== null) { const refreshNext = queued.current; queued.current = null; void load(refreshNext); }
    }
  }, []);

  useEffect(() => {
    void load(false).then(() => load(true));
    const touches = (payload: { number: number }) => { if (numbers.current.has(payload.number)) void load(true); };
    const unlisten: Array<Promise<() => void>> = [
      bridgeApi.onGithubSessionPrsChanged(() => { void load(false); }),
      bridgeApi.onGithubChecksChanged(touches),
      bridgeApi.onGithubCiFinished(touches),
    ];
    const onFocus = () => {
      if (document.visibilityState !== "visible") return;
      if (Date.now() - lastRefresh.current >= SIDEBAR_FOCUS_THROTTLE_MS) void load(true);
    };
    window.addEventListener("focus", onFocus);
    document.addEventListener("visibilitychange", onFocus);
    return () => {
      for (const pending of unlisten) void pending.then(stop => stop());
      window.removeEventListener("focus", onFocus);
      document.removeEventListener("visibilitychange", onFocus);
    };
  }, [load]);

  const anyOpen = Object.values(byChat).some(prs => prs.some(pr => pr.state === "open"));
  useEffect(() => {
    if (!anyOpen) return;
    const timer = window.setInterval(() => { if (document.visibilityState !== "hidden") void load(true); }, SIDEBAR_OPEN_INTERVAL_MS);
    return () => window.clearInterval(timer);
  }, [anyOpen, load]);

  return byChat;
}

// ── check vocabulary ────────────────────────────────────────────────────────

export const CHECK_ICON: Record<CheckTone, { icon: typeof CircleCheck; className: string; label: string }> = {
  failed: { icon: CircleX, className: "text-destructive", label: "Failed" },
  running: { icon: CircleDot, className: "text-warning github-check-live", label: "Running" },
  queued: { icon: CircleDashed, className: "text-muted-foreground", label: "Queued" },
  passed: { icon: CircleCheck, className: "text-success", label: "Passed" },
  skipped: { icon: CircleMinus, className: "text-muted-foreground/70", label: "Skipped" },
};

// ── the glyph ───────────────────────────────────────────────────────────────

/**
 * A PR at 16px, the same in the shelf, the Work pane and anywhere else a PR is
 * one row: a failing PR is a red circle-x, merged and closed are their GitHub
 * marks, and an open PR is a ring whose arcs are its checks (running breathes,
 * passed is green, skipped is a faint grey). No checks yet is a dashed track.
 */
export function PullRequestGlyph({ pr, size = 16, className }: { pr: SessionPullRequest; size?: number; className?: string }) {
  const status = chatPrStatus(pr);
  if (pr.state === "merged") return <GitMerge size={size - 2} strokeWidth={2} className={cn("shrink-0 text-info", className)} aria-hidden="true" />;
  if (pr.state === "closed") return <GitPullRequestClosed size={size - 2} strokeWidth={2} className={cn("shrink-0 text-muted-foreground", className)} aria-hidden="true" />;
  if (status.phase === "conflicts") return <GitPullRequestArrow size={size - 2} strokeWidth={2} className={cn("shrink-0 text-destructive", className)} aria-hidden="true" data-pr-glyph="conflicts" />;
  if (status.phase === "failing") return <CircleX size={size} strokeWidth={2} className={cn("shrink-0 text-destructive", className)} aria-hidden="true" />;
  const { checks } = pr;
  const stroke = 2.25;
  const radius = (size - stroke) / 2;
  const circumference = 2 * Math.PI * radius;
  const arcs = checks.total === 0 ? [] : [
    { value: checks.inProgress, className: "stroke-warning github-check-live" },
    { value: checks.passed, className: "stroke-success" },
    { value: checks.skipped + checks.cancelled, className: "stroke-foreground/25" },
  ].filter(arc => arc.value > 0);
  const gap = arcs.length > 1 ? 1.2 : 0;
  let offset = 0;
  return <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} className={cn("shrink-0 -rotate-90", className)} aria-hidden="true" data-pr-glyph={status.phase}>
    <circle cx={size / 2} cy={size / 2} r={radius} fill="none" strokeWidth={stroke} className={cn("stroke-foreground/[0.12]", checks.total === 0 && "[stroke-dasharray:2_2.5]")} />
    {arcs.map((arc, index) => {
      const length = Math.max(3, (arc.value / checks.total) * circumference - gap);
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
        className={cn(arc.className, "transition-[stroke-dasharray,stroke-dashoffset] duration-500")}
      />;
      offset += (arc.value / checks.total) * circumference;
      return element;
    })}
  </svg>;
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

// ── announcements ───────────────────────────────────────────────────────────

/** The live region's text changes only on a phase transition, so a poll that
 * lands the same state stays silent. */
export function useStatusAnnouncements(prs: SessionPullRequest[]) {
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
