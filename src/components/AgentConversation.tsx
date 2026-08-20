import { memo, useEffect, useMemo, useRef, useState } from "react";
import { AlertTriangle, Brain, Check, ChevronDown, ChevronRight, Circle, CornerDownRight, FilePlus2, FileText, Gauge, GitFork, Globe, ListChecks, LoaderCircle, Pencil, RotateCcw, Search, SquareTerminal, Wrench, X } from "lucide-react";
import { projectSessionConversation, reduceConversation, type ConversationItem } from "../conversation";
import { pickGreeting } from "../greetings";
import type { AgentEvent, ApprovalDecision, CompletionSummary, ContinuationFidelity, Session, SessionEntry, WorkerRepositoryBinding } from "../types";
import { latestUsageSnapshot, type UsageSnapshot } from "../usage";
import { describeError } from "../errors";
import { looksLikeDiff } from "./highlight";
import { PatchView } from "./DiffView";
import { Markdown } from "./Markdown";
import { harnessLabel } from "../utils";

function providerLabel(harness?: string | null): string | undefined {
  return harness ? harnessLabel(harness) : undefined;
}

// Codex-style conversation: prose messages, live tool-call cards, clickable
// thinking, and consecutive tool work folded into activity groups that expand
// into per-action rows.

type Rendered =
  | { kind: "item"; item: ConversationItem }
  | { kind: "group"; key: string; items: ConversationItem[] }
  | { kind: "raw-group"; key: string; items: ConversationItem[] };

const GROUPABLE = new Set(["activity", "diff", "artifact"]);

function groupItems(items: ConversationItem[]): Rendered[] {
  const out: Rendered[] = [];
  const rawItems: ConversationItem[] = [];
  for (const item of items) {
    if (item.type === "raw") {
      rawItems.push(item);
      continue;
    }
    if (GROUPABLE.has(item.type) && item.data.staleBase !== true) {
      const last = out[out.length - 1];
      if (last?.kind === "group") { last.items.push(item); continue; }
      out.push({ kind: "group", key: `group-${item.key}`, items: [item] });
      continue;
    }
    out.push({ kind: "item", item });
  }
  if (rawItems.length) out.push({ kind: "raw-group", key: "raw-provider-events", items: rawItems });
  return out;
}

/* ── Shared surfaces ─────────────────────────────────────────────────────
   Chrome is achromatic and elevation is a lightness ladder, so an alert is a
   plain card wearing a colored tick rather than a tinted panel. A full wash is
   held back for genuine failures. */

/// The user's turn is the only bubble in the transcript; the agent answers
/// straight onto the canvas. That asymmetry is what carries the hierarchy.
const BUBBLE = "chat-message-enter ml-auto w-fit max-w-[85%] whitespace-pre-wrap break-words rounded-2xl border border-border bg-card px-3.5 py-2 text-[15px] leading-[1.7] tracking-[-0.006em] text-foreground";
/// Transcript-level notice: a quiet card that reads as a margin note.
const NOTICE = "mb-4 rounded-lg border border-border border-l-2 bg-card px-3 py-2 text-xs text-muted-foreground";
/// A decision the user has to make — approvals, adoptions, stale bases.
const PANEL = "my-4 min-w-0 overflow-hidden rounded-lg border border-border border-l-2 bg-card";
/// Verbatim text — paths, commands, diffstats — sits in an inset code well.
const WELL = "block rounded-md border border-border bg-code px-2.5 py-2 font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap break-words overflow-x-auto text-foreground";
const BTN_PRIMARY = "inline-flex items-center gap-1.5 rounded-full bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground transition-colors hover:bg-primary/90 disabled:opacity-50";
const BTN_SECONDARY = "inline-flex items-center gap-1.5 rounded-full border border-input px-3 py-1.5 text-xs font-medium text-foreground transition-colors hover:bg-accent disabled:opacity-50";

/* ── Tool-call presentation ─────────────────────────────────────────────── */

type ActionVerb = "edit" | "read" | "run" | "search" | "tool";

// The row is achromatic on purpose: the tool icon identifies the action, and
// color is left to the things that carry meaning — diffstats and failures.
interface ToolInfo {
  verb: ActionVerb;
  icon: React.ReactNode;
  doing: string;
  done: string;
  target?: string;
  detail?: string;
}

const VERB_DONE: Record<ActionVerb, string> = {
  edit: "edited files", read: "read files", run: "ran commands", search: "searched the web", tool: "used tools",
};
const VERB_DOING: Record<ActionVerb, string> = {
  edit: "editing files", read: "reading files", run: "running commands", search: "searching the web", tool: "using tools",
};

function str(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value : undefined;
}

function baseName(path: string): string {
  const parts = path.replace(/[/\\]+$/, "").split(/[/\\]/);
  return parts[parts.length - 1] || path;
}

function fileTarget(input: Record<string, unknown>): { target?: string; detail?: string } {
  const path = str(input.file_path) ?? str(input.notebook_path) ?? str(input.path);
  return path ? { target: baseName(path), detail: path } : {};
}

/** Map a conversation item from either provider to a pretty tool card. */
function toolInfo(item: ConversationItem): ToolInfo {
  const data = item.data;
  const input = (data.input && typeof data.input === "object" ? data.input : {}) as Record<string, unknown>;
  const name = str(data.name);
  const dataType = String(data.type ?? "");
  const title = item.title ?? "";

  // Claude tool_use blocks: name + input.
  if (name) {
    const key = name.toLowerCase();
    if (key === "bash" || key === "shell") {
      const command = str(input.command);
      return { verb: "run", icon: <SquareTerminal size={12}/>, doing: "Running", done: "Ran", target: command ?? (title || "command"), detail: command };
    }
    if (key === "read") {
      const { target, detail } = fileTarget(input);
      return { verb: "read", icon: <FileText size={12}/>, doing: "Reading", done: "Read", target: target ?? "file", detail };
    }
    if (key === "edit" || key === "multiedit" || key === "notebookedit") {
      const { target, detail } = fileTarget(input);
      return { verb: "edit", icon: <Pencil size={12}/>, doing: "Editing", done: "Edited", target: target ?? "file", detail };
    }
    if (key === "write") {
      const { target, detail } = fileTarget(input);
      return { verb: "edit", icon: <FilePlus2 size={12}/>, doing: "Writing", done: "Wrote", target: target ?? "file", detail };
    }
    if (key === "grep" || key === "glob") {
      const pattern = str(input.pattern);
      return { verb: "search", icon: <Search size={12}/>, doing: "Searching", done: "Searched", target: pattern ? `“${pattern}”` : "files", detail: str(input.path) };
    }
    if (key === "websearch") {
      return { verb: "search", icon: <Globe size={12}/>, doing: "Searching the web", done: "Searched the web", target: str(input.query) };
    }
    if (key === "webfetch") {
      return { verb: "search", icon: <Globe size={12}/>, doing: "Fetching", done: "Fetched", target: str(input.url) };
    }
    if (key === "task") {
      return { verb: "tool", icon: <GitFork size={12}/>, doing: "Delegating", done: "Delegated", target: str(input.description) };
    }
    if (key === "todowrite") {
      return { verb: "tool", icon: <ListChecks size={12}/>, doing: "Updating tasks", done: "Updated tasks" };
    }
    if (key.startsWith("mcp__")) {
      const parts = name.replace(/^mcp__/, "").split("__");
      const server = parts[0] ?? name;
      const tool = parts.slice(1).join(" ").replaceAll("_", " ") || name;
      return { verb: "tool", icon: <Wrench size={12}/>, doing: `Using ${server}`, done: `Used ${server}`, target: tool };
    }
    return { verb: "tool", icon: <Wrench size={12}/>, doing: `Using ${name}`, done: `Used ${name}`, target: title || undefined };
  }

  // Codex-shaped items.
  if (item.type === "diff" || dataType.includes("patch") || dataType.includes("fileChange")) {
    const path = str(data.path) ?? (title || undefined);
    return { verb: "edit", icon: <Pencil size={12}/>, doing: "Editing", done: "Edited", target: path ? baseName(path) : "files", detail: path };
  }
  if (dataType === "readFile" || /^read /i.test(title)) {
    const path = str(data.path) ?? title.replace(/^read /i, "");
    return { verb: "read", icon: <FileText size={12}/>, doing: "Reading", done: "Read", target: path ? baseName(path) : "file", detail: path || undefined };
  }
  if (dataType === "commandExecution" || data.command) {
    const command = str(data.command) ?? (title || undefined);
    return { verb: "run", icon: <SquareTerminal size={12}/>, doing: "Running", done: "Ran", target: command ?? "command", detail: command };
  }
  if (dataType === "webSearch") {
    return { verb: "search", icon: <Globe size={12}/>, doing: "Searching the web", done: "Searched the web", target: title || undefined };
  }
  return { verb: "tool", icon: <Wrench size={12}/>, doing: "Using a tool", done: "Used a tool", target: title || undefined };
}

function summarize(items: ConversationItem[], live: boolean): string {
  const seen: ActionVerb[] = [];
  for (const item of items) {
    const verb = toolInfo(item).verb;
    if (!seen.includes(verb)) seen.push(verb);
  }
  const table = live ? VERB_DOING : VERB_DONE;
  const text = seen.map(verb => table[verb]).join(", ");
  const sentence = text.charAt(0).toUpperCase() + text.slice(1);
  return live ? `${sentence}…` : sentence;
}

/** The expandable payload behind a tool row: explicit output, else the item text. */
function toolOutput(item: ConversationItem): string {
  const direct = str(item.data.aggregatedOutput) ?? str(item.data.output);
  if (direct) return direct;
  const text = item.text ?? "";
  if (!text.trim()) return "";
  if (item.title && text.trim() === item.title.trim()) return "";
  return text;
}

function DiffPatch({ patch, path }: { patch: string; path?: string }) {
  return <PatchView patch={patch.slice(-8000)} path={path ?? ""} className="max-h-[320px] px-1" />;
}

/// One tool call, one collapsed monospace row: what ran on the left, what it
/// cost on the right. Expanding reveals the raw output or patch underneath.
function ActionRow({ item }: { item: ConversationItem }) {
  const [open, setOpen] = useState(false);
  const live = item.status === "inProgress" || item.status === "streaming";
  const failed = item.status === "failed";
  const succeeded = !live && !failed && item.status === "completed";
  const info = toolInfo(item);
  const output = toolOutput(item);
  const additions = Number(item.data.additions ?? NaN);
  const deletions = Number(item.data.deletions ?? NaN);
  const durationMs = Number(item.data.durationMs ?? NaN);
  const label = `${live ? info.doing : info.done}${info.target ? ` ${info.target}` : ""}`;
  const detail = info.detail && info.detail !== info.target ? info.detail : undefined;
  return (
    <div className="min-w-0">
      <button
        type="button"
        className="group/row flex w-full min-w-0 items-center gap-2 rounded-lg border border-border px-3 py-1.5 text-left font-mono text-[11px] text-muted-foreground transition-colors hover:bg-accent disabled:cursor-default disabled:hover:bg-transparent"
        disabled={!output}
        onClick={() => output && setOpen(value => !value)}
      >
        <span className="shrink-0 text-muted-foreground/70" aria-hidden="true">{info.icon}</span>
        <span className={`truncate ${live ? "text-foreground" : ""}`}>{label}</span>
        {/* flex-1 from a zero basis, so the path gives up room before the label does. */}
        {detail && <span className="hidden min-w-0 flex-1 truncate text-muted-foreground/70 sm:block">{detail}</span>}
        <span className="ml-auto flex shrink-0 items-center gap-2">
          {info.verb === "edit" && Number.isFinite(additions) && (
            <span><b className="font-medium text-success">+{additions}</b> <b className="font-medium text-destructive">−{deletions}</b></span>
          )}
          {Number.isFinite(durationMs) && !live && <span className="text-muted-foreground/70">{Math.max(1, Math.round(durationMs / 1000))}s</span>}
          {live && <LoaderCircle size={12} className="animate-spin text-muted-foreground" aria-hidden="true"/>}
          {succeeded && <Check size={12} className="text-success" aria-hidden="true"/>}
          {!live && failed && <X size={12} className="text-destructive" aria-hidden="true"/>}
          {output && <ChevronRight size={12} className={`text-muted-foreground/70 transition-transform ${open ? "rotate-90" : ""}`} aria-hidden="true"/>}
        </span>
      </button>
      {open && output && (
        <div className="mb-2 mt-0.5 overflow-hidden rounded-lg border border-border bg-code">
          {looksLikeDiff(output)
            ? <DiffPatch patch={output} path={info.verb === "edit" || info.verb === "read" ? info.detail : undefined}/>
            : <pre className="max-h-[320px] overflow-auto whitespace-pre-wrap break-words p-3 font-mono text-[11.5px] leading-relaxed text-muted-foreground">{output.slice(-6000)}</pre>}
        </div>
      )}
    </div>
  );
}

function ActivityGroup({ items }: { items: ConversationItem[] }) {
  const live = items.some(item => item.status === "inProgress" || item.status === "streaming");
  const [open, setOpen] = useState(false);
  const expanded = open || live;
  return (
    <div className="my-2.5 min-w-0">
      <button
        type="button"
        className="group inline-flex max-w-full items-center gap-2 rounded-lg px-1 py-1 text-left text-[12px] text-muted-foreground transition-colors hover:text-foreground"
        onClick={() => setOpen(value => !value)}
      >
        {live ? <PulseDot size={7}/> : <Check size={12} className="shrink-0 text-muted-foreground/70" aria-hidden="true"/>}
        <span className={`truncate ${live ? "text-foreground" : ""}`}>{summarize(items, live)}</span>
        <ChevronDown size={13} className={`shrink-0 text-muted-foreground/70 transition-transform ${expanded ? "rotate-180" : ""}`} aria-hidden="true"/>
      </button>
      {expanded && <div className="mt-1 grid min-w-0 gap-1">{items.map(item => <ActionRow key={item.key} item={item}/>)}</div>}
    </div>
  );
}

/* ── Conversation ───────────────────────────────────────────────────────── */

export const AgentConversation = memo(function AgentConversation({ session, events = [], forestEntries, activeLeafId, repositoryDivergence, completion, continuationFidelity, onResolve, onOpenSession, onWaiveCompletion, onRefreshBase, onRetryWorker, pendingAdoptions = [], onResolveAdoption, preview, working, pendingMessages = [], highlightEntryId }: { session?: Session; events?: AgentEvent[]; forestEntries?: SessionEntry[]; activeLeafId?: string | null; repositoryDivergence?: string; completion?: CompletionSummary | null; continuationFidelity?: ContinuationFidelity; onResolve: (eventId: number, decision: ApprovalDecision) => void; onOpenSession?: (sessionId: string) => void; onWaiveCompletion?: (attemptId: string, checkIds: string[], reason: string) => Promise<void>; onRefreshBase?: () => Promise<void>; onRetryWorker?: (childSessionId: string) => Promise<void>; pendingAdoptions?: WorkerRepositoryBinding[]; onResolveAdoption?: (childSessionId: string, decision: "adopt" | "discard") => Promise<void>; preview?: boolean; working?: boolean; pendingMessages?: string[]; highlightEntryId?: string | null }) {
  const visibleItems = useMemo(() => {
    const durableItems = forestEntries?.length ? projectSessionConversation(forestEntries, activeLeafId ?? null) : [];
    const nextLiveItems = reduceConversation(events);
    const items = [...durableItems];
    const durableIds = new Set(durableItems.map(item => item.eventId));
    for (const live of nextLiveItems) {
      if (!durableIds.has(live.eventId)) items.push(live);
    }
    return items.filter(item => item.type !== "raw");
  }, [activeLeafId, events, forestEntries]);
  const renderedItems = useMemo(() => groupItems(visibleItems), [visibleItems]);

  if (!session && !preview) return <Empty title="No chat yet" copy="Start a chat from the sidebar, or open a workspace agent."/>;
  if (!visibleItems.length && !working && !pendingMessages.length && !completion && !pendingAdoptions.length && repositoryDivergence !== "diverged" && continuationFidelity !== "projected_at_boundary" && continuationFidelity !== "projected_mid_turn") return <GreetingEmpty seed={session?.id ?? session?.workspaceId ?? undefined} />;
  const streaming = visibleItems.some(item => item.status === "streaming" || item.status === "inProgress");
  const existingUserTexts = new Set(visibleItems.filter(item => item.type === "message" && item.role === "user").map(item => item.text.trim()));
  const optimistic = pendingMessages.filter(text => !existingUserTexts.has(text.trim()));
  const errorContext = { provider: providerLabel(session?.harness), snapshot: latestUsageSnapshot(events) };
  const tailLength = visibleItems.length ? visibleItems[visibleItems.length - 1].text.length : 0;
  const scrollSignature = `${visibleItems.length}:${tailLength}:${optimistic.length}:${working ? 1 : 0}`;
  return <ScrollFollow signature={scrollSignature} className="absolute inset-0 overflow-y-auto overscroll-y-none scroll-smooth px-3 py-8 pb-24 sm:px-6 sm:py-10">
    <div className="mx-auto flex w-full min-w-0 max-w-3xl flex-col gap-6 sm:gap-8">
      {pendingAdoptions.map(binding => <AdoptionCard key={binding.sessionId} binding={binding} onResolve={onResolveAdoption}/>)}
      {completion && <VerificationCard summary={completion} onWaive={onWaiveCompletion}/>}
      {repositoryDivergence === "diverged" && <div role="alert" className={`${NOTICE} border-l-warning`}>This branch&apos;s context predates the current file state.</div>}
      {continuationFidelity === "projected_at_boundary" && <div role="status" className={`${NOTICE} border-l-info`}>Continuation restored from a phase-boundary projection; provider reasoning state was not transferred.</div>}
      {continuationFidelity === "projected_mid_turn" && <div role="alert" className={`${NOTICE} border-l-warning`}>Continuation fidelity degraded: context was projected mid-turn and provider reasoning state was lost.</div>}
      {preview && <div className="w-fit mx-auto mb-[22px] px-2.5 py-1 border border-dashed border-border rounded-full text-muted-foreground text-[10.5px] tracking-[0.04em]">Design preview — sample conversation</div>}
      {renderedItems.map(entry => entry.kind === "group"
        ? <ActivityGroup key={entry.key} items={entry.items}/>
        : entry.kind === "raw-group" ? <RawEventGroup key={entry.key} items={entry.items}/>
        : <div
            key={entry.item.key}
            id={entry.item.entryId ? `forest-entry-${entry.item.entryId}` : undefined}
            data-entry-id={entry.item.entryId}
            className={highlightEntryId && entry.item.entryId === highlightEntryId ? "rounded-xl bg-accent/60 ring-1 ring-ring/70" : undefined}
          >
            <ItemView item={entry.item} onResolve={onResolve} onOpenSession={onOpenSession} onRefreshBase={onRefreshBase} onRetryWorker={onRetryWorker} errorContext={errorContext}/>
          </div>)}
      {optimistic.map((text, index) => <div key={`pending-${index}`} className={BUBBLE}>{text}</div>)}
      {working && !streaming && <div className="chat-message-enter flex justify-start"><div className="thinking-shimmer h-[2px] w-16 rounded-full" /></div>}
    </div>
  </ScrollFollow>;
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
        {summary.checks.map(check => <div key={check.checkId} className="flex items-start gap-2 text-xs text-muted-foreground">{statusIcon(check.status)}<div className="min-w-0 flex-1"><div className="flex flex-wrap gap-x-2"><span className="text-foreground">{check.command || check.checkId}</span><span>{check.kind.replace("_", " ")}</span>{check.verifierFamily && <span>· {check.verifierFamily}</span>}</div>{check.detail && <p className="mt-0.5 truncate font-mono text-[10.5px]">{check.detail}</p>}</div><span className="shrink-0 text-[10px] uppercase tracking-wide">{check.status}</span></div>)}
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

function GreetingEmpty({ seed }: { seed?: string }) {
  // Stable per session so it doesn't reshuffle on every re-render, tinted by time of day.
  const greeting = useMemo(() => pickGreeting(seed), [seed]);
  return <Empty title={greeting.headline} copy={greeting.hint} />;
}

function Empty({ title, copy }: { title: string; copy: string }) {
  return <div className="absolute inset-0 flex flex-col items-center justify-center px-4 text-center animate-page-enter sm:px-6">
    <div className="flex w-full max-w-[440px] flex-col items-center">
      <h2 className="font-display text-[22px] font-medium tracking-[-0.02em] text-foreground">{title}</h2>
      <p className="mt-2.5 max-w-[380px] text-[13.5px] leading-relaxed tracking-[-0.004em] text-muted-foreground">{copy}</p>
    </div>
  </div>;
}

function ItemView({ item, onResolve, onOpenSession, onRefreshBase, onRetryWorker, errorContext }: { item: ConversationItem; onResolve: (eventId: number, decision: ApprovalDecision) => void; onOpenSession?: (sessionId: string) => void; onRefreshBase?: () => Promise<void>; onRetryWorker?: (childSessionId: string) => Promise<void>; errorContext?: { provider?: string; snapshot: UsageSnapshot | null } }) {
  if (item.type === "message") {
    if (item.role === "user") return <div className={BUBBLE}>{item.text}</div>;
    // No bubble, no card: the agent writes straight onto the canvas.
    return <div className="chat-message-enter w-full min-w-0 text-foreground">{item.status === "streaming" && !item.text.trim() ? <div className="thinking-shimmer h-[2px] w-16 rounded-full" /> : <Markdown text={item.text} dim={item.status === "streaming"} />}</div>;
  }
  if (item.data.staleBase === true) return <StaleBaseCard item={item} onRefresh={onRefreshBase}/>;
  if (item.type === "reasoning") return <Reasoning item={item}/>;
  if (item.type === "plan") return <PlanCard item={item}/>;
  if (item.type === "approval") return <ApprovalCard item={item} onResolve={onResolve}/>;
  if (item.type === "delegation") return <DelegationRow item={item} onOpenSession={onOpenSession} onRetryWorker={onRetryWorker}/>;
  if (item.type === "checkpoint" || item.type === "compaction" || item.type === "branch-summary") return <ForestCard item={item}/>;
  if (item.type === "raw") return <RawEvent item={item}/>;
  if (item.type === "error") {
    const described = describeError(item.text, errorContext);
    const isUsage = described.kind === "usage-limit";
    // A rate limit is a wait, not a failure — it gets the tick. A real error
    // is the one place a full wash is warranted.
    return <div className={`my-4 flex min-w-0 gap-2.5 rounded-lg border border-l-2 p-3 ${isUsage ? "border-border border-l-warning bg-card text-warning" : "border-destructive/30 border-l-destructive bg-destructive/5 text-destructive"}`}>
      {isUsage ? <Gauge size={14} className="mt-0.5 shrink-0" aria-hidden="true" /> : <AlertTriangle size={14} className="mt-0.5 shrink-0" aria-hidden="true" />}
      <div className="min-w-0"><b className="text-[12px]">{described.title}</b><p className="mt-1 text-[12px] leading-relaxed break-words text-muted-foreground">{described.message}</p></div>
    </div>;
  }
  return <ActivityGroup items={[item]}/>;
}

function ForestCard({ item }: { item: ConversationItem }) {
  const label = item.type === "checkpoint" ? "Checkpoint" : item.type === "compaction" ? "Context" : "Branch";
  return <div className={`my-2 min-w-0 px-3 py-2.5 border border-border rounded-lg bg-card ${item.type}`}>
    <header className="flex gap-2 items-center"><GitFork size={13} className="shrink-0" aria-hidden="true" /><b className="min-w-0 truncate text-foreground">{item.title || label}</b><small className="ml-auto shrink-0 text-muted-foreground">{item.status || "durable"}</small></header>
    {item.text && <p className="mt-2 text-muted-foreground text-[12px]">{item.text}</p>}
    {item.data.reason ? <code className="inline-block mt-2 font-mono text-[10px] break-all text-muted-foreground">{String(item.data.reason)}</code> : null}
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

function Reasoning({ item }: { item: ConversationItem }) {
  const streaming = item.status === "streaming";
  const text = item.text || stringList(item.data.summary);
  if (streaming) {
    const lines = text.split("\n").map(line => line.trim()).filter(Boolean);
    const recent = lines.slice(-3);
    return (
      <div className="chat-message-enter my-3 flex min-w-0 items-start gap-3 rounded-xl border border-border bg-card px-3.5 py-3 sm:px-4">
        <Brain size={14} className="mt-0.5 shrink-0 text-muted-foreground animate-[thinking-pulse_1.6s_ease-in-out_infinite]" aria-hidden="true"/>
        <div className="min-w-0 flex-1">
          <span className="text-[12px] font-medium bg-[linear-gradient(90deg,var(--color-muted-foreground)_0%,var(--color-foreground)_50%,var(--color-muted-foreground)_100%)] bg-[length:200%_100%] bg-clip-text text-transparent animate-[thinking-shimmer_2s_linear_infinite]">Thinking…</span>
          {recent.length > 0 && <div className="mt-1.5 space-y-0.5">
            {recent.map((line, index) => <p key={index} className={`truncate text-[12px] leading-relaxed ${index === recent.length - 1 ? "text-muted-foreground" : "text-muted-foreground/70"}`}>{line}</p>)}
          </div>}
        </div>
      </div>
    );
  }
  return (
    <details className="group my-3 min-w-0 rounded-xl border border-border bg-card [&_summary::-webkit-details-marker]:hidden">
      <summary className="flex cursor-pointer items-center gap-2.5 px-3.5 py-2.5 text-[12px] text-muted-foreground transition-colors hover:text-foreground sm:px-4">
        <Brain size={13} className="shrink-0 text-muted-foreground/70" aria-hidden="true"/>
        <span className="font-medium">Thought for a moment</span>
        <ChevronRight size={12} className="ml-auto shrink-0 text-muted-foreground/70 transition-transform group-open:rotate-90" aria-hidden="true"/>
      </summary>
      <div className="border-t border-border px-3.5 py-3 text-muted-foreground sm:px-4">
        <Markdown text={text}/>
      </div>
    </details>
  );
}

function PlanCard({ item }: { item: ConversationItem }) {
  return <div className="my-[14px] min-w-0 border border-border rounded-lg bg-card overflow-hidden">
    <header className="flex items-center gap-2 px-3.5 py-2.5 border-b border-border text-muted-foreground sm:px-4"><FileText size={13} className="shrink-0" aria-hidden="true" /><b className="text-[12px] font-medium text-foreground">{item.title || "Plan"}</b></header>
    {planSteps(item.data).map((step, index) => <div className={`min-h-[30px] flex items-center gap-[9px] py-[2px] px-3.5 text-[12.5px] sm:px-4 ${step.status === "completed" ? "text-muted-foreground/70 line-through decoration-border" : step.status === "inProgress" ? "text-foreground" : "text-muted-foreground"}`} key={`${step.step}-${index}`}>
      {step.status === "completed" ? <Check size={12} className="flex-none text-muted-foreground/70" aria-hidden="true" /> : step.status === "inProgress" ? <PulseDot size={8}/> : <Circle size={8} className="flex-none text-muted-foreground/70" aria-hidden="true" />}
      <span className="min-w-0">{step.step}</span>
    </div>)}
  </div>;
}

function ApprovalCard({ item, onResolve }: { item: ConversationItem; onResolve: (eventId: number, decision: ApprovalDecision) => void }) {
  const pending = item.status === "pending";
  const accepted = item.status === "accept" || item.status === "acceptForSession";
  const scope = Array.isArray(item.data.requestedOwnedPaths) ? item.data.requestedOwnedPaths.map(String) : [];
  // The machine-readable routing reason and its remediation are persisted on the
  // approval entry. Showing them is what turns "allow this?" into a decision the
  // user can actually make.
  const reason = typeof item.data.reason === "string" ? item.data.reason : "";
  const remediation = typeof item.data.remediation === "string" ? item.data.remediation : "";
  return <div className={`${PANEL} border-l-warning`}>
    <header className="flex flex-wrap items-baseline gap-x-2 gap-y-1 pt-3 px-3.5 sm:px-4"><b className="text-[13px] font-semibold text-foreground">{item.title || "Approval needed"}</b>{pending && <small className="text-warning text-[10.5px] tracking-[0.03em]">waiting for you</small>}</header>
    {item.data.objective ? <p className="mt-1.5 px-3.5 text-muted-foreground text-[12.5px] leading-relaxed sm:px-4">{String(item.data.objective)}</p> : null}
    {scope.length > 0 && <div className="mt-2 px-3.5 sm:px-4">
      <small className="block text-muted-foreground/70 text-[10.5px] tracking-[0.03em] uppercase">Write scope</small>
      <code className={`mt-1 ${WELL}`}>{scope.join("\n")}</code>
    </div>}
    {remediation
      ? <p className="mt-2 px-3.5 text-muted-foreground text-[12.5px] leading-relaxed sm:px-4">{reason ? <em className="not-italic font-mono text-[11px] break-all text-warning">{reason}</em> : null}{reason ? " — " : ""}{remediation}</p>
      : item.text && <p className="mt-1.5 px-3.5 text-muted-foreground text-[12.5px] leading-relaxed sm:px-4">{item.text}</p>}
    {item.data.command ? <code className={`mt-2.5 mx-3.5 sm:mx-4 ${WELL}`}>{String(item.data.command)}</code> : null}
    {item.data.cwd ? <small className="block pt-1.5 px-3.5 text-muted-foreground/70 font-mono text-[10.5px] break-all sm:px-4">{String(item.data.cwd)}</small> : null}
    {pending
      ? <div className="flex flex-wrap justify-end gap-[7px] px-3.5 py-3 sm:px-4">
          <button className={BTN_SECONDARY} onClick={() => onResolve(item.eventId, "decline")}><X size={12} aria-hidden="true" /> Decline</button>
          {item.data.approvalType !== "delegation_path_scope" && <button className={BTN_SECONDARY} onClick={() => onResolve(item.eventId, "acceptForSession")}>Allow for session</button>}
          <button className={BTN_PRIMARY} onClick={() => onResolve(item.eventId, "accept")}><Check size={12} aria-hidden="true" /> Allow once</button>
        </div>
      : <div className="flex items-center gap-1.5 px-3.5 pb-3 pt-2.5 text-muted-foreground text-[11.5px] sm:px-4">{accepted ? <Check size={12} aria-hidden="true" /> : <X size={12} aria-hidden="true" />} {item.status}</div>}
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

function DelegationRow({ item, onOpenSession, onRetryWorker }: { item: ConversationItem; onOpenSession?: (sessionId: string) => void; onRetryWorker?: (childSessionId: string) => Promise<void> }) {
  // A background worker's own approval card renders on the worker's conversation,
  // which is normally not the selected one. This mirrored row is what makes the
  // block visible where the user is actually working.
  if ("childBlocked" in item.data) {
    const blocked = item.data.childBlocked === true;
    const paths = Array.isArray(item.data.ownedPaths) ? item.data.ownedPaths.map(String) : [];
    // The child session id travels on the event, so the mirror can hand the user
    // straight to the worker's conversation where the real approval lives —
    // otherwise the block is a dead end and the card is effectively lost.
    const childSessionId = typeof item.data.childSessionId === "string" ? item.data.childSessionId : undefined;
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
  const isRejected = "willRetry" in item.data;
  if (isRejected) {
    const reason = String(item.data.reason ?? item.text ?? "");
    const willRetry = item.data.willRetry === true;
    const launchFailed = item.data.launchFailed === true;
    return <div className="my-3 min-w-0 rounded-lg border border-border border-l-2 border-l-warning bg-card px-3 py-2 text-xs text-muted-foreground" role="alert">
      <div className="flex items-center gap-1.5 font-medium text-warning"><AlertTriangle size={13} className="shrink-0" aria-hidden="true" /> <span className="min-w-0">{launchFailed ? "Worker failed to start" : "Delegation rejected — no worker started"}</span></div>
      {reason && <p className="mt-1 font-mono text-[11px] leading-relaxed break-words text-foreground">{reason}</p>}
      <p className="mt-1 text-muted-foreground/70">{launchFailed ? (item.data.orchestratorNotified === true ? "The orchestrator was notified and will not wait for this worker." : "The orchestrator could not be notified; retry after fixing the launch failure.") : willRetry ? "Asked the orchestrator to correct and re-emit the request." : "Automatic correction limit reached; the orchestrator will not retry on its own."}</p>
    </div>;
  }
  const isResult = "delivered" in item.data;
  const model = String(item.data.modelLabel ?? item.data.model ?? "");
  const effort = item.data.effort ? String(item.data.effort) : "";
  const [open, setOpen] = useState(false);
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
