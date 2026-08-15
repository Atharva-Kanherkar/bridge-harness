import { memo, useEffect, useMemo, useRef, useState } from "react";
import { AlertTriangle, Brain, Check, ChevronDown, ChevronRight, Circle, CornerDownRight, FilePlus2, FileText, Gauge, GitFork, Globe, ListChecks, LoaderCircle, Pencil, Search, SquareTerminal, Wrench, X } from "lucide-react";
import { projectSessionConversation, reduceConversation, type ConversationItem } from "../conversation";
import { pickGreeting } from "../greetings";
import type { AgentEvent, ApprovalDecision, CompletionSummary, ContinuationFidelity, Session, SessionEntry, WorkerRepositoryBinding } from "../types";
import { latestUsageSnapshot, type UsageSnapshot } from "../usage";
import { describeError } from "../errors";
import { highlightDiff, looksLikeDiff } from "./highlight";
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

/* ── Tool-call presentation ─────────────────────────────────────────────── */

type ActionVerb = "edit" | "read" | "run" | "search" | "tool";
type ToolTone = "indigo" | "sky" | "violet" | "teal" | "amber" | "neutral";

interface ToolInfo {
  verb: ActionVerb;
  icon: React.ReactNode;
  tone: ToolTone;
  doing: string;
  done: string;
  target?: string;
  detail?: string;
}

const TONE_STYLES: Record<ToolTone, string> = {
  indigo: "border-indigo-300/20 bg-indigo-400/[0.09] text-indigo-300",
  sky: "border-sky-300/20 bg-sky-400/[0.09] text-sky-300",
  violet: "border-violet-300/20 bg-violet-400/[0.09] text-violet-300",
  teal: "border-teal-300/20 bg-teal-400/[0.09] text-teal-300",
  amber: "border-amber-300/20 bg-amber-400/[0.09] text-amber-300",
  neutral: "border-white/[0.08] bg-white/[0.05] text-neutral-400",
};

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
      return { verb: "run", icon: <SquareTerminal size={13}/>, tone: "indigo", doing: "Running", done: "Ran", target: command ?? (title || "command"), detail: command };
    }
    if (key === "read") {
      const { target, detail } = fileTarget(input);
      return { verb: "read", icon: <FileText size={13}/>, tone: "sky", doing: "Reading", done: "Read", target: target ?? "file", detail };
    }
    if (key === "edit" || key === "multiedit" || key === "notebookedit") {
      const { target, detail } = fileTarget(input);
      return { verb: "edit", icon: <Pencil size={13}/>, tone: "violet", doing: "Editing", done: "Edited", target: target ?? "file", detail };
    }
    if (key === "write") {
      const { target, detail } = fileTarget(input);
      return { verb: "edit", icon: <FilePlus2 size={13}/>, tone: "violet", doing: "Writing", done: "Wrote", target: target ?? "file", detail };
    }
    if (key === "grep" || key === "glob") {
      const pattern = str(input.pattern);
      return { verb: "search", icon: <Search size={13}/>, tone: "teal", doing: "Searching", done: "Searched", target: pattern ? `“${pattern}”` : "files", detail: str(input.path) };
    }
    if (key === "websearch") {
      return { verb: "search", icon: <Globe size={13}/>, tone: "teal", doing: "Searching the web", done: "Searched the web", target: str(input.query) };
    }
    if (key === "webfetch") {
      return { verb: "search", icon: <Globe size={13}/>, tone: "teal", doing: "Fetching", done: "Fetched", target: str(input.url) };
    }
    if (key === "task") {
      return { verb: "tool", icon: <GitFork size={13}/>, tone: "amber", doing: "Delegating", done: "Delegated", target: str(input.description) };
    }
    if (key === "todowrite") {
      return { verb: "tool", icon: <ListChecks size={13}/>, tone: "amber", doing: "Updating tasks", done: "Updated tasks" };
    }
    if (key.startsWith("mcp__")) {
      const parts = name.replace(/^mcp__/, "").split("__");
      const server = parts[0] ?? name;
      const tool = parts.slice(1).join(" ").replaceAll("_", " ") || name;
      return { verb: "tool", icon: <Wrench size={13}/>, tone: "neutral", doing: `Using ${server}`, done: `Used ${server}`, target: tool };
    }
    return { verb: "tool", icon: <Wrench size={13}/>, tone: "neutral", doing: `Using ${name}`, done: `Used ${name}`, target: title || undefined };
  }

  // Codex-shaped items.
  if (item.type === "diff" || dataType.includes("patch") || dataType.includes("fileChange")) {
    const path = str(data.path) ?? (title || undefined);
    return { verb: "edit", icon: <Pencil size={13}/>, tone: "violet", doing: "Editing", done: "Edited", target: path ? baseName(path) : "files", detail: path };
  }
  if (dataType === "readFile" || /^read /i.test(title)) {
    const path = str(data.path) ?? title.replace(/^read /i, "");
    return { verb: "read", icon: <FileText size={13}/>, tone: "sky", doing: "Reading", done: "Read", target: path ? baseName(path) : "file", detail: path || undefined };
  }
  if (dataType === "commandExecution" || data.command) {
    const command = str(data.command) ?? (title || undefined);
    return { verb: "run", icon: <SquareTerminal size={13}/>, tone: "indigo", doing: "Running", done: "Ran", target: command ?? "command", detail: command };
  }
  if (dataType === "webSearch") {
    return { verb: "search", icon: <Globe size={13}/>, tone: "teal", doing: "Searching the web", done: "Searched the web", target: title || undefined };
  }
  return { verb: "tool", icon: <Wrench size={13}/>, tone: "neutral", doing: "Using a tool", done: "Used a tool", target: title || undefined };
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

function DiffPatch({ patch }: { patch: string }) {
  const html = useMemo(() => highlightDiff(patch.slice(-8000)), [patch]);
  return <div className="diff-view max-h-[320px] overflow-auto p-3" dangerouslySetInnerHTML={{ __html: html }} />;
}

function ActionRow({ item }: { item: ConversationItem }) {
  const [open, setOpen] = useState(false);
  const live = item.status === "inProgress" || item.status === "streaming";
  const failed = item.status === "failed";
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
        className="group/row flex w-full items-center gap-2.5 rounded-xl px-1.5 py-1.5 text-left transition-colors hover:bg-white/[0.035] disabled:cursor-default"
        disabled={!output}
        onClick={() => output && setOpen(value => !value)}
      >
        <span className={`flex h-6 w-6 shrink-0 items-center justify-center rounded-lg border ${TONE_STYLES[info.tone]}`}>
          {info.icon}
        </span>
        <span className="min-w-0 flex-1">
          <span className={`block truncate text-[12px] font-medium ${live ? "text-neutral-100" : "text-neutral-300"}`}>{label}</span>
          {detail && <span className="mt-px block truncate font-mono text-[10.5px] text-neutral-600">{detail}</span>}
        </span>
        {info.verb === "edit" && Number.isFinite(additions) && (
          <span className="shrink-0 font-mono text-[10.5px]"><b className="font-medium text-emerald-400/90">+{additions}</b> <b className="font-medium text-red-400/90">−{deletions}</b></span>
        )}
        {Number.isFinite(durationMs) && !live && <span className="shrink-0 font-mono text-[10.5px] text-neutral-600">{Math.max(1, Math.round(durationMs / 1000))}s</span>}
        {live && <LoaderCircle size={12} className="shrink-0 animate-spin text-indigo-300/80" aria-hidden="true"/>}
        {!live && failed && <X size={12} className="shrink-0 text-rose-400" aria-hidden="true"/>}
        {output && <ChevronRight size={12} className={`shrink-0 text-neutral-600 transition-transform ${open ? "rotate-90" : ""}`} aria-hidden="true"/>}
      </button>
      {open && output && (
        <div className="mb-2 ml-[42px] mt-0.5 overflow-hidden rounded-xl border border-white/[0.06] bg-black/30">
          {looksLikeDiff(output)
            ? <DiffPatch patch={output}/>
            : <pre className="max-h-[320px] overflow-auto whitespace-pre-wrap p-3 font-mono text-[11.5px] leading-relaxed text-neutral-400">{output.slice(-6000)}</pre>}
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
    <div className="my-2.5">
      <button
        type="button"
        className="group inline-flex items-center gap-2 rounded-lg px-1 py-1 text-left text-[12px] text-neutral-500 transition-colors hover:text-neutral-300"
        onClick={() => setOpen(value => !value)}
      >
        {live ? <PulseDot size={7}/> : <Check size={12} className="text-neutral-600" aria-hidden="true"/>}
        <span className={live ? "text-neutral-300" : undefined}>{summarize(items, live)}</span>
        <ChevronDown size={13} className={`text-neutral-600 transition-transform ${expanded ? "rotate-180" : ""}`} aria-hidden="true"/>
      </button>
      {expanded && <div className="mt-1 grid gap-0.5">{items.map(item => <ActionRow key={item.key} item={item}/>)}</div>}
    </div>
  );
}

/* ── Conversation ───────────────────────────────────────────────────────── */

export const AgentConversation = memo(function AgentConversation({ session, events = [], forestEntries, activeLeafId, repositoryDivergence, completion, continuationFidelity, onResolve, onWaiveCompletion, onRefreshBase, pendingAdoptions = [], onResolveAdoption, preview, working, pendingMessages = [] }: { session?: Session; events?: AgentEvent[]; forestEntries?: SessionEntry[]; activeLeafId?: string | null; repositoryDivergence?: string; completion?: CompletionSummary | null; continuationFidelity?: ContinuationFidelity; onResolve: (eventId: number, decision: ApprovalDecision) => void; onWaiveCompletion?: (attemptId: string, checkIds: string[], reason: string) => Promise<void>; onRefreshBase?: () => Promise<void>; pendingAdoptions?: WorkerRepositoryBinding[]; onResolveAdoption?: (childSessionId: string, decision: "adopt" | "discard") => Promise<void>; preview?: boolean; working?: boolean; pendingMessages?: string[] }) {
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
  return <ScrollFollow signature={scrollSignature} className="absolute inset-0 overflow-y-auto overscroll-y-none scroll-smooth px-4 py-8 pb-24 sm:px-6 sm:py-10">
    <div className="mx-auto flex w-full max-w-2xl flex-col gap-6 sm:gap-8">
      {pendingAdoptions.map(binding => <AdoptionCard key={binding.sessionId} binding={binding} onResolve={onResolveAdoption}/>)}
      {completion && <VerificationCard summary={completion} onWaive={onWaiveCompletion}/>}
      {repositoryDivergence === "diverged" && <div role="alert" className="mb-4 rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning">This branch&apos;s context predates the current file state.</div>}
      {continuationFidelity === "projected_at_boundary" && <div role="status" className="mb-4 rounded-lg border border-border bg-foreground/[0.03] px-3 py-2 text-xs text-muted-foreground">Continuation restored from a phase-boundary projection; provider reasoning state was not transferred.</div>}
      {continuationFidelity === "projected_mid_turn" && <div role="alert" className="mb-4 rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning">Continuation fidelity degraded: context was projected mid-turn and provider reasoning state was lost.</div>}
      {preview && <div className="w-fit mx-auto mb-[22px] px-2.5 py-1 border border-dashed border-border rounded-full text-muted-foreground text-[10.5px] tracking-[0.04em]">Design preview — sample conversation</div>}
      {renderedItems.map(entry => entry.kind === "group"
        ? <ActivityGroup key={entry.key} items={entry.items}/>
        : entry.kind === "raw-group" ? <RawEventGroup key={entry.key} items={entry.items}/>
        : <ItemView key={entry.item.key} item={entry.item} onResolve={onResolve} onRefreshBase={onRefreshBase} errorContext={errorContext}/>)}
      {optimistic.map((text, index) => <div key={`pending-${index}`} className="chat-message-enter flex w-full justify-end"><div className="max-w-[min(100%,44rem)] rounded-[1.35rem] rounded-tr-md border border-white/[0.07] bg-white/[0.055] px-5 py-3 text-[15px] leading-[1.7] tracking-[-0.006em] text-neutral-100 shadow-[0_10px_30px_-14px_rgba(0,0,0,0.5),inset_0_1px_0_rgba(255,255,255,0.06)] backdrop-blur-xl whitespace-pre-wrap">{text}</div></div>)}
      {working && !streaming && <div className="chat-message-enter flex justify-start pl-4"><div className="thinking-shimmer h-[2px] w-16 rounded-full" /></div>}
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
  return <div role="alert" className="my-4 overflow-hidden rounded-lg border border-warning/30 bg-warning/5">
    <header className="flex items-baseline gap-[9px] px-[15px] pt-3">
      <b className="text-[13px] font-semibold text-foreground">Worker changes are not in your workspace yet</b>
      {settling && <small className="text-warning text-[10.5px] tracking-[0.03em]">settling…</small>}
    </header>
    <p className="mt-1.5 px-[15px] text-[12.5px] leading-relaxed text-muted-foreground">
      This worker wrote in its own worktree. Adopting merges those changes into your checkout; discarding throws them away. Until you choose, this session stays unfinished.
    </p>
    {binding.changedPaths.length > 0 && <code className="mt-2 mx-[15px] block max-h-40 overflow-y-auto rounded-md border border-border bg-background p-[9px_11px] font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap text-code-foreground">{binding.changedPaths.join("\n")}</code>}
    <small className="mt-1 block px-[15px] font-mono text-[10.5px] text-muted-foreground/70">
      {binding.diffstat ?? "no diffstat"} · {binding.worktreeBranch}{binding.dirty ? " · uncommitted" : ""}
    </small>
    <small className="mt-0.5 block px-[15px] font-mono text-[10.5px] text-muted-foreground/60">{binding.worktreePath}</small>
    {error && <p className="mt-1.5 px-[15px] text-[12px] leading-relaxed text-destructive-foreground">{error}</p>}
    <div className="flex items-center justify-end gap-[7px] p-[12px_13px]">
      <button disabled={!!busy || settling || !onResolve} className="inline-flex items-center gap-1.5 rounded-md border border-border bg-transparent px-3 py-1.5 text-xs font-medium text-muted-foreground transition-colors hover:bg-accent disabled:opacity-50" onClick={() => act("discard")}>
        <X size={12} aria-hidden="true" /> {busy === "discard" ? "Discarding…" : "Discard"}
      </button>
      <button disabled={!!busy || settling || !onResolve} className="inline-flex items-center gap-1.5 rounded-md bg-foreground px-3 py-1.5 text-xs font-medium text-background transition-colors hover:bg-foreground/90 disabled:opacity-50" onClick={() => act("adopt")}>
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
  const tone = summary.verdict === "verified" ? "border-success/30 bg-success/10 text-success" : summary.verdict === "waived" ? "border-warning/30 bg-warning/10 text-warning" : summary.verdict === "changes_requested" || summary.verdict === "failed" ? "border-destructive/30 bg-destructive/10 text-destructive" : "border-info/30 bg-info/10 text-info";
  const title = summary.verdict === "verified" ? "Verified" : summary.verdict === "waived" ? "Verified with waiver" : summary.verdict === "changes_requested" ? "Changes requested" : summary.verdict === "superseded" ? "Evidence superseded" : summary.verdict === "failed" ? "Verification failed" : "Verifying";
  const statusIcon = (status: string) => status === "passed" ? <Check size={12} className="text-success" aria-hidden="true"/> : status === "failed" ? <X size={12} className="text-destructive" aria-hidden="true"/> : status === "skipped" || status === "blocked" || status === "stale" ? <AlertTriangle size={12} className="text-warning" aria-hidden="true"/> : <Circle size={10} className="text-muted-foreground" aria-hidden="true"/>;
  return <section aria-label="Completion verification" className={`overflow-hidden rounded-xl border ${tone}`}>
    <div className="flex items-start gap-3 px-4 py-3">
      <div className="min-w-0 flex-1"><div className="flex items-center gap-2"><strong className="font-display text-sm text-foreground">{title}</strong><span className="text-[11px]">{summary.passedRequired} of {summary.totalRequired} required checks passed</span></div><p className="mt-1 font-mono text-[10.5px] text-muted-foreground">revision {summary.repository.head.slice(0, 12)} · {summary.repository.dirtyDigest === "clean" ? "clean" : `tree ${summary.repository.dirtyDigest.slice(0, 8)}`}</p></div>
      {!summary.markdownCommitted && <span className="rounded-full border border-border px-2 py-0.5 text-[10px] text-muted-foreground">private contract</span>}
    </div>
    <details className="border-t border-current/10">
      <summary className="cursor-pointer px-4 py-2 text-xs text-foreground marker:text-muted-foreground">Proof and checks</summary>
      <div className="space-y-1 border-t border-current/10 px-4 py-3">
        {summary.checks.map(check => <div key={check.checkId} className="flex items-start gap-2 text-xs text-muted-foreground">{statusIcon(check.status)}<div className="min-w-0 flex-1"><div className="flex flex-wrap gap-x-2"><span className="text-foreground">{check.command || check.checkId}</span><span>{check.kind.replace("_", " ")}</span>{check.verifierFamily && <span>· {check.verifierFamily}</span>}</div>{check.detail && <p className="mt-0.5 truncate font-mono text-[10.5px]">{check.detail}</p>}</div><span className="text-[10px] uppercase tracking-wide">{check.status}</span></div>)}
        {summary.waiverReason && <p className="mt-2 rounded-md border border-warning/20 bg-warning/5 px-2 py-1.5 text-xs text-warning">Waiver: {summary.waiverReason}</p>}
        {onWaive && unresolved.length > 0 && !["verified", "waived", "superseded"].includes(summary.verdict) && <div className="pt-2">
          {!waiverOpen ? <button type="button" onClick={() => setWaiverOpen(true)} className="rounded-md border border-warning/30 px-2.5 py-1.5 text-xs font-medium text-warning transition-colors hover:bg-warning/10">Waive unresolved checks</button> : <form onSubmit={event => { event.preventDefault(); const reason = waiverReason.trim(); if (!reason) { setWaiverError("Explain why these checks can be waived."); return; } setWaiving(true); setWaiverError(undefined); void onWaive(summary.attemptId, unresolved.map(check => check.checkId), reason).then(() => { setWaiverOpen(false); setWaiverReason(""); }).catch(error => setWaiverError(error instanceof Error ? error.message : String(error))).finally(() => setWaiving(false)); }} className="space-y-2 rounded-lg border border-warning/20 bg-warning/5 p-2.5">
            <p className="text-xs text-warning">This records human-approved risk for: {unresolved.map(check => check.command || check.checkId).join(", ")}. It remains distinct from Verified.</p>
            <textarea autoFocus value={waiverReason} onChange={event => setWaiverReason(event.target.value)} rows={2} placeholder="Reason for waiver" aria-label="Waiver reason" className="w-full resize-none rounded-md border border-border bg-background px-2.5 py-2 text-xs text-foreground outline-none placeholder:text-muted-foreground focus:border-warning/50"/>
            {waiverError && <p role="alert" className="text-xs text-destructive">{waiverError}</p>}
            <div className="flex gap-2"><button type="submit" disabled={waiving} className="rounded-md bg-warning px-2.5 py-1.5 text-xs font-semibold text-warning-foreground disabled:opacity-50">{waiving ? "Recording…" : `Waive ${unresolved.length} check${unresolved.length === 1 ? "" : "s"}`}</button><button type="button" disabled={waiving} onClick={() => { setWaiverOpen(false); setWaiverError(undefined); }} className="rounded-md border border-border px-2.5 py-1.5 text-xs text-muted-foreground disabled:opacity-50">Cancel</button></div>
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
  return <div className="absolute inset-0 flex flex-col items-center justify-center px-6 text-center animate-page-enter">
    <div className="flex max-w-[440px] flex-col items-center">
      <h2 className="font-display text-[22px] font-medium tracking-[-0.02em] text-white">{title}</h2>
      <p className="mt-2.5 max-w-[380px] text-[13.5px] leading-relaxed tracking-[-0.004em] text-neutral-500">{copy}</p>
    </div>
  </div>;
}

function ItemView({ item, onResolve, onRefreshBase, errorContext }: { item: ConversationItem; onResolve: (eventId: number, decision: ApprovalDecision) => void; onRefreshBase?: () => Promise<void>; errorContext?: { provider?: string; snapshot: UsageSnapshot | null } }) {
  if (item.type === "message") {
    if (item.role === "user") return <div className="chat-message-enter flex w-full justify-end"><div className="max-w-[min(100%,44rem)] rounded-[1.35rem] rounded-tr-md border border-white/[0.07] bg-white/[0.055] px-5 py-3 text-[15px] leading-[1.7] tracking-[-0.006em] text-neutral-100 shadow-[0_10px_30px_-14px_rgba(0,0,0,0.5),inset_0_1px_0_rgba(255,255,255,0.06)] backdrop-blur-xl whitespace-pre-wrap">{item.text}</div></div>;
    return <div className="chat-message-enter flex w-full justify-start"><div className="relative max-w-[min(100%,44rem)] py-1 pl-1 text-neutral-300">{item.status === "streaming" && !item.text.trim() ? <div className="thinking-shimmer h-[2px] w-16 rounded-full" /> : <Markdown text={item.text} dim={item.status === "streaming"} />}</div></div>;
  }
  if (item.data.staleBase === true) return <StaleBaseCard item={item} onRefresh={onRefreshBase}/>;
  if (item.type === "reasoning") return <Reasoning item={item}/>;
  if (item.type === "plan") return <PlanCard item={item}/>;
  if (item.type === "approval") return <ApprovalCard item={item} onResolve={onResolve}/>;
  if (item.type === "delegation") return <DelegationRow item={item}/>;
  if (item.type === "checkpoint" || item.type === "compaction" || item.type === "branch-summary") return <ForestCard item={item}/>;
  if (item.type === "raw") return <RawEvent item={item}/>;
  if (item.type === "error") {
    const described = describeError(item.text, errorContext);
    const isUsage = described.kind === "usage-limit";
    return <div className={`my-4 flex gap-2.5 p-3 rounded-lg border ${isUsage ? "border-warning/30 bg-warning/5 text-warning" : "border-destructive/30 bg-destructive/5 text-destructive-foreground"}`}>
      {isUsage ? <Gauge size={14} aria-hidden="true" /> : <AlertTriangle size={14} aria-hidden="true" />}
      <div><b className="text-[12px]">{described.title}</b><p className={`mt-1 text-[12px] leading-relaxed ${isUsage ? "text-warning/90" : "text-destructive-foreground/85"}`}>{described.message}</p></div>
    </div>;
  }
  return <ActivityGroup items={[item]}/>;
}

function ForestCard({ item }: { item: ConversationItem }) {
  const label = item.type === "checkpoint" ? "Checkpoint" : item.type === "compaction" ? "Context" : "Branch";
  return <div className={`my-2 px-3 py-2.5 border border-border rounded-lg bg-card ${item.type}`}>
    <header className="flex gap-2 items-center"><GitFork size={13} aria-hidden="true" /><b>{item.title || label}</b><small className="ml-auto text-muted-foreground">{item.status || "durable"}</small></header>
    {item.text && <p className="mt-2 text-muted-foreground text-[12px]">{item.text}</p>}
    {item.data.reason ? <code className="inline-block mt-2 text-[10px]">{String(item.data.reason)}</code> : null}
  </div>;
}

function RawEvent({ item }: { item: ConversationItem }) {
  return <details className="my-2 p-2 border border-border rounded-lg bg-card group [&_summary::-webkit-details-marker]:hidden">
    <summary className="flex items-center gap-[7px] cursor-pointer text-[11px] text-muted-foreground hover:text-foreground transition-colors"><SquareTerminal size={12} aria-hidden="true" /><span className="flex-1">{item.title || "Raw provider event"}</span><small className="text-muted-foreground group-open:hidden">inspect</small></summary>
    <pre className="max-h-[220px] overflow-auto mt-1.5 p-2 rounded-md bg-background text-[10px] whitespace-pre-wrap">{JSON.stringify(item.data, null, 2)}</pre>
  </details>;
}

function RawEventGroup({ items }: { items: ConversationItem[] }) {
  return <details className="my-2 p-2 border border-border rounded-lg bg-card group [&_summary::-webkit-details-marker]:hidden">
    <summary className="flex items-center gap-[7px] cursor-pointer text-[11px] text-muted-foreground hover:text-foreground transition-colors"><SquareTerminal size={12} aria-hidden="true" /><span className="flex-1">{items.length} raw provider event{items.length === 1 ? "" : "s"}</span><small className="text-muted-foreground group-open:hidden">inspect</small></summary>
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
      <div className="chat-message-enter my-3 flex items-start gap-3 rounded-2xl border border-white/[0.05] bg-white/[0.02] px-4 py-3">
        <Brain size={14} className="mt-0.5 shrink-0 text-violet-300/80 animate-[thinking-pulse_1.6s_ease-in-out_infinite]" aria-hidden="true"/>
        <div className="min-w-0 flex-1">
          <span className="text-[12px] font-medium bg-[linear-gradient(90deg,var(--color-muted-foreground)_0%,var(--color-foreground)_50%,var(--color-muted-foreground)_100%)] bg-[length:200%_100%] bg-clip-text text-transparent animate-[shimmer_2s_linear_infinite]">Thinking…</span>
          {recent.length > 0 && <div className="mt-1.5 space-y-0.5">
            {recent.map((line, index) => <p key={index} className={`truncate text-[12px] leading-relaxed ${index === recent.length - 1 ? "text-neutral-400" : "text-neutral-600"}`}>{line}</p>)}
          </div>}
        </div>
      </div>
    );
  }
  return (
    <details className="group my-3 rounded-2xl border border-white/[0.05] bg-white/[0.02] [&_summary::-webkit-details-marker]:hidden">
      <summary className="flex cursor-pointer items-center gap-2.5 px-4 py-2.5 text-[12px] text-neutral-500 transition-colors hover:text-neutral-300">
        <Brain size={13} className="shrink-0 text-violet-300/70" aria-hidden="true"/>
        <span className="font-medium">Thought for a moment</span>
        <ChevronRight size={12} className="ml-auto text-neutral-600 transition-transform group-open:rotate-90" aria-hidden="true"/>
      </summary>
      <div className="border-t border-white/[0.045] px-4 py-3 text-neutral-400">
        <Markdown text={text}/>
      </div>
    </details>
  );
}

function PlanCard({ item }: { item: ConversationItem }) {
  return <div className="my-[14px] border border-border rounded-lg bg-card overflow-hidden">
    <header className="flex items-center gap-2 p-[10px_13px] border-b border-border text-muted-foreground"><FileText size={13} aria-hidden="true" /><b className="text-[12px] font-medium text-foreground">{item.title || "Plan"}</b></header>
    {planSteps(item.data).map((step, index) => <div className={`min-h-[30px] flex items-center gap-[9px] py-[2px] px-[13px] text-[12.5px] ${step.status === "completed" ? "text-muted-foreground/60 line-through decoration-border" : step.status === "inProgress" ? "text-foreground" : "text-muted-foreground"}`} key={`${step.step}-${index}`}>
      {step.status === "completed" ? <Check size={12} className="flex-none text-muted-foreground/60" aria-hidden="true" /> : step.status === "inProgress" ? <PulseDot size={8}/> : <Circle size={8} className="flex-none text-muted-foreground/60" aria-hidden="true" />}
      <span>{step.step}</span>
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
  return <div className="my-4 border border-warning/30 rounded-lg bg-warning/5 overflow-hidden">
    <header className="flex items-baseline gap-[9px] pt-3 px-[15px]"><b className="text-[13px] font-semibold text-foreground">{item.title || "Approval needed"}</b>{pending && <small className="text-warning text-[10.5px] tracking-[0.03em]">waiting for you</small>}</header>
    {item.data.objective ? <p className="mt-1.5 px-[15px] text-muted-foreground text-[12.5px] leading-relaxed">{String(item.data.objective)}</p> : null}
    {scope.length > 0 && <div className="mt-2 px-[15px]">
      <small className="block text-muted-foreground/70 text-[10.5px] tracking-[0.03em] uppercase">Write scope</small>
      <code className="mt-1 block p-[9px_11px] border border-border rounded-md bg-background text-code-foreground font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap">{scope.join("\n")}</code>
    </div>}
    {remediation
      ? <p className="mt-2 px-[15px] text-muted-foreground text-[12.5px] leading-relaxed">{reason ? <em className="not-italic font-mono text-[11px] text-warning/90">{reason}</em> : null}{reason ? " — " : ""}{remediation}</p>
      : item.text && <p className="mt-1.5 px-[15px] text-muted-foreground text-[12.5px] leading-relaxed">{item.text}</p>}
    {item.data.command ? <code className="block mt-2.5 mx-[15px] p-[9px_11px] border border-border rounded-md bg-background text-code-foreground font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap">{String(item.data.command)}</code> : null}
    {item.data.cwd ? <small className="block pt-1.5 px-[15px] text-muted-foreground/70 font-mono text-[10.5px]">{String(item.data.cwd)}</small> : null}
    {pending
      ? <div className="flex justify-end gap-[7px] p-[12px_13px]">
          <button className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium bg-transparent border border-border text-muted-foreground hover:bg-accent transition-colors" onClick={() => onResolve(item.eventId, "decline")}><X size={12} aria-hidden="true" /> Decline</button>
          {item.data.approvalType !== "delegation_path_scope" && <button className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium bg-transparent border border-border text-foreground hover:bg-accent transition-colors" onClick={() => onResolve(item.eventId, "acceptForSession")}>Allow for session</button>}
          <button className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium bg-foreground text-background hover:bg-foreground/90 transition-colors" onClick={() => onResolve(item.eventId, "accept")}><Check size={12} aria-hidden="true" /> Allow once</button>
        </div>
      : <div className="p-[10px_15px_12px] flex items-center gap-1.5 text-muted-foreground text-[11.5px]">{accepted ? <Check size={12} aria-hidden="true" /> : <X size={12} aria-hidden="true" />} {item.status}</div>}
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
  return <div role="alert" className="my-4 overflow-hidden rounded-lg border border-warning/30 bg-warning/5">
    <header className="flex items-baseline gap-[9px] px-[15px] pt-3">
      <b className="text-[13px] font-semibold text-foreground">{item.title || `Workspace is ${behind} commits behind ${baseRef}`}</b>
    </header>
    <p className="mt-1.5 px-[15px] text-[12.5px] leading-relaxed text-muted-foreground">{item.text}</p>
    <small className="mt-1 block px-[15px] font-mono text-[10.5px] text-muted-foreground/70">{behind} behind · {ahead} ahead · {baseRef}{divergence.dirty === true ? " · uncommitted changes" : ""}</small>
    {error && <p className="mt-1.5 px-[15px] text-[12px] leading-relaxed text-destructive-foreground">{error}</p>}
    {refreshed
      ? <div className="flex items-center gap-1.5 p-[10px_15px_12px] text-[11.5px] text-muted-foreground"><Check size={12} aria-hidden="true" /> Workspace refreshed onto {baseRef}</div>
      : blocker
        // Refresh is a strict fast-forward, and this card already has the
        // fields that decide whether one is possible. Offering the button
        // anyway meant the common case — a worktree with one commit on it —
        // presented an action whose only outcome was an error dialog.
        ? <div className="p-[10px_15px_12px] text-[11.5px] leading-relaxed text-muted-foreground">{blocker}</div>
        : <div className="flex items-center justify-end gap-[7px] p-[12px_13px]">
            <span className="mr-auto px-2 text-[11.5px] text-muted-foreground/70">Or continue on the current revision.</span>
            <button disabled={busy || !onRefresh} className="inline-flex items-center gap-1.5 rounded-md border border-border bg-foreground px-3 py-1.5 text-xs font-medium text-background transition-colors hover:bg-foreground/90 disabled:opacity-50" onClick={refresh}>{busy ? "Refreshing…" : "Refresh workspace"}</button>
          </div>}
  </div>;
}

function DelegationRow({ item }: { item: ConversationItem }) {
  // A background worker's own approval card renders on the worker's conversation,
  // which is normally not the selected one. This mirrored row is what makes the
  // block visible where the user is actually working.
  if ("childBlocked" in item.data) {
    const blocked = item.data.childBlocked === true;
    const paths = Array.isArray(item.data.ownedPaths) ? item.data.ownedPaths.map(String) : [];
    if (!blocked) {
      return <div className="my-3 flex items-center gap-[9px] px-2 -ml-2 text-muted-foreground text-[12.5px]">
        <Check size={13} aria-hidden="true" />
        <span>{item.title || "Worker approval resolved"}</span>
      </div>;
    }
    return <div className="my-3 rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning" role="alert">
      <div className="flex items-center gap-1.5 font-medium"><AlertTriangle size={13} aria-hidden="true" /> {item.title || "A worker needs your approval"}</div>
      {item.data.objective ? <p className="mt-1 text-warning/80">{String(item.data.objective)}</p> : null}
      {item.text && <p className="mt-1 text-warning/80">{item.text}</p>}
      {item.data.command ? <code className="mt-1.5 block rounded-md border border-warning/25 bg-background/40 px-2 py-1.5 font-mono text-[11px] leading-relaxed whitespace-pre-wrap text-warning/90">{String(item.data.command)}</code> : null}
      {item.data.cwd ? <small className="mt-1 block font-mono text-[10.5px] text-warning/60">{String(item.data.cwd)}</small> : null}
      {paths.length > 0 && <small className="mt-1 block font-mono text-[10.5px] text-warning/60">write scope: {paths.join(", ")}</small>}
      <p className="mt-1 text-warning/70">Open the worker&apos;s conversation to allow or decline. The worker is idle until you do.</p>
    </div>;
  }
  const isRejected = "willRetry" in item.data;
  if (isRejected) {
    const reason = String(item.data.reason ?? item.text ?? "");
    const willRetry = item.data.willRetry === true;
    const launchFailed = item.data.launchFailed === true;
    return <div className="my-3 rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning" role="alert">
      <div className="flex items-center gap-1.5 font-medium"><AlertTriangle size={13} aria-hidden="true" /> {launchFailed ? "Worker failed to start" : "Delegation rejected — no worker started"}</div>
      {reason && <p className="mt-1 font-mono text-[11px] leading-relaxed text-warning/90">{reason}</p>}
      <p className="mt-1 text-warning/70">{launchFailed ? (item.data.orchestratorNotified === true ? "The orchestrator was notified and will not wait for this worker." : "The orchestrator could not be notified; retry after fixing the launch failure.") : willRetry ? "Asked the orchestrator to correct and re-emit the request." : "Automatic correction limit reached; the orchestrator will not retry on its own."}</p>
    </div>;
  }
  const isResult = "delivered" in item.data;
  const model = String(item.data.modelLabel ?? item.data.model ?? "");
  const effort = item.data.effort ? String(item.data.effort) : "";
  const [open, setOpen] = useState(false);
  return <div className="my-3">
    <button className="w-full flex items-center gap-[9px] min-h-[30px] p-[4px_8px] -ml-2 rounded-md text-left text-muted-foreground text-[12.5px] hover:bg-accent transition-colors" onClick={() => item.text && setOpen(value => !value)}>
      {isResult ? <CornerDownRight size={13} aria-hidden="true" /> : <GitFork size={13} aria-hidden="true" />}
      <span className="min-w-0 overflow-hidden whitespace-nowrap text-ellipsis">{isResult ? "Subagent finished" : "Delegated"}{titleAddsInfo(item, isResult) && <b className="text-muted-foreground font-medium"> · {item.title}</b>}</span>
      {model && <em className="flex-none font-mono text-[10px] text-muted-foreground/70 not-italic border border-border rounded px-1.5 py-0.5">{model}{effort ? ` · ${effort}` : ""}</em>}
      {item.text && <ChevronRight size={12} className={`transition-transform ${open ? "rotate-90" : ""}`} aria-hidden="true" />}
    </button>
    {open && item.text && <div className="my-1 ml-[5px] pl-[15px] border-l border-border text-muted-foreground text-[12.5px] leading-relaxed"><Markdown text={item.text}/></div>}
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
