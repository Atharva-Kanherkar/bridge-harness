import { memo, useEffect, useMemo, useRef, useState } from "react";
import { AlertTriangle, Check, ChevronDown, ChevronRight, Circle, CornerDownRight, FileText, Gauge, GitFork, Pencil, Search, SquareTerminal, Wrench, X } from "lucide-react";
import { projectSessionConversation, reduceConversation, type ConversationItem } from "../conversation";
import { pickGreeting } from "../greetings";
import type { AgentEvent, ContinuationFidelity, Session, SessionEntry } from "../types";
import { latestUsageSnapshot, type UsageSnapshot } from "../usage";
import { describeError } from "../errors";
import { Markdown } from "./Markdown";

function providerLabel(harness?: string | null): string | undefined {
  if (!harness) return undefined;
  if (harness === "claude") return "Claude";
  if (harness === "codex") return "Codex";
  return harness.charAt(0).toUpperCase() + harness.slice(1);
}

// Codex-style conversation: prose messages, quiet collapsible thinking, and
// consecutive tool work folded into activity groups ("Edited files, read
// files, ran commands") that expand into per-action rows.

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
    if (GROUPABLE.has(item.type)) {
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

type ActionVerb = "edit" | "read" | "run" | "search" | "tool";

function verbOf(item: ConversationItem): ActionVerb {
  const dataType = String(item.data.type ?? "");
  if (item.type === "diff" || dataType.includes("patch") || dataType.includes("fileChange")) return "edit";
  if (dataType === "readFile" || /^read /i.test(item.title ?? "")) return "read";
  if (dataType === "commandExecution" || item.data.command) return "run";
  if (dataType === "webSearch") return "search";
  return "tool";
}

const VERB_ICON: Record<ActionVerb, React.ReactNode> = {
  edit: <Pencil size={13}/>, read: <FileText size={13}/>, run: <SquareTerminal size={13}/>,
  search: <Search size={13}/>, tool: <Wrench size={13}/>,
};
const VERB_SUMMARY: Record<ActionVerb, string> = {
  edit: "edited files", read: "read files", run: "ran commands", search: "searched the web", tool: "used tools",
};

function summarize(items: ConversationItem[]): string {
  const seen: ActionVerb[] = [];
  for (const item of items) { const verb = verbOf(item); if (!seen.includes(verb)) seen.push(verb); }
  const parts = seen.map(verb => VERB_SUMMARY[verb]);
  const text = parts.join(", ");
  return text.charAt(0).toUpperCase() + text.slice(1);
}

function actionLabel(item: ConversationItem): { label: string; meta?: React.ReactNode } {
  const verb = verbOf(item);
  const additions = Number(item.data.additions ?? NaN);
  const deletions = Number(item.data.deletions ?? NaN);
  const durationMs = Number(item.data.durationMs ?? NaN);
  if (verb === "edit") {
    const file = item.title || String(item.data.path ?? "files");
    return { label: `Edited ${file}`, meta: Number.isFinite(additions) ? <span className="dstat"><b className="add">+{additions}</b> <b className="del">−{deletions}</b></span> : undefined };
  }
  if (verb === "run") {
    const command = String(item.data.command ?? item.title ?? "command");
    return { label: `Ran ${command}`, meta: Number.isFinite(durationMs) ? <span>{Math.round(durationMs / 1000) || 1}s</span> : undefined };
  }
  if (verb === "read") return { label: item.title || `Read ${String(item.data.path ?? "file")}` };
  if (verb === "search") return { label: item.title || "Searched the web" };
  return { label: item.title || "Used a tool" };
}

export const AgentConversation = memo(function AgentConversation({ session, events = [], forestEntries, activeLeafId, repositoryDivergence, continuationFidelity, onResolve, preview, working, pendingMessages = [] }: { session?: Session; events?: AgentEvent[]; forestEntries?: SessionEntry[]; activeLeafId?: string | null; repositoryDivergence?: "aligned" | "diverged" | "unknown"; continuationFidelity?: ContinuationFidelity; onResolve: (eventId: number, decision: string) => void; preview?: boolean; working?: boolean; pendingMessages?: string[] }) {
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
  if (!visibleItems.length && !working && !pendingMessages.length && repositoryDivergence !== "diverged" && continuationFidelity !== "projected_at_boundary" && continuationFidelity !== "projected_mid_turn") return <GreetingEmpty seed={session?.id ?? session?.workspaceId ?? undefined} />;
  const streaming = visibleItems.some(item => item.status === "streaming" || item.status === "inProgress");
  const existingUserTexts = new Set(visibleItems.filter(item => item.type === "message" && item.role === "user").map(item => item.text.trim()));
  const optimistic = pendingMessages.filter(text => !existingUserTexts.has(text.trim()));
  const errorContext = { provider: providerLabel(session?.harness), snapshot: latestUsageSnapshot(events) };
  const tailLength = visibleItems.length ? visibleItems[visibleItems.length - 1].text.length : 0;
  const scrollSignature = `${visibleItems.length}:${tailLength}:${optimistic.length}:${working ? 1 : 0}`;
  return <ScrollFollow signature={scrollSignature} className="absolute inset-0 overflow-y-auto overscroll-y-none scroll-smooth px-4 py-8 pb-24 sm:px-6 sm:py-10 scrollbar-thin scrollbar-thumb-white/10">
    <div className="mx-auto flex w-full max-w-2xl flex-col gap-6 sm:gap-8">
      {repositoryDivergence === "diverged" && <div role="alert" className="mb-4 rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning">This branch&apos;s context predates the current file state.</div>}
      {continuationFidelity === "projected_at_boundary" && <div role="status" className="mb-4 rounded-lg border border-border bg-foreground/[0.03] px-3 py-2 text-xs text-muted-foreground">Continuation restored from a phase-boundary projection; provider reasoning state was not transferred.</div>}
      {continuationFidelity === "projected_mid_turn" && <div role="alert" className="mb-4 rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning">Continuation fidelity degraded: context was projected mid-turn and provider reasoning state was lost.</div>}
      {preview && <div className="w-fit mx-auto mb-[22px] px-2.5 py-1 border border-dashed border-border rounded-full text-muted-foreground text-[10.5px] tracking-[0.04em]">Design preview — sample conversation</div>}
      {renderedItems.map(entry => entry.kind === "group"
        ? <ActivityGroup key={entry.key} items={entry.items}/>
        : entry.kind === "raw-group" ? <RawEventGroup key={entry.key} items={entry.items}/>
        : <ItemView key={entry.item.key} item={entry.item} onResolve={onResolve} errorContext={errorContext}/>)}
      {optimistic.map((text, index) => <div key={`pending-${index}`} className="chat-message-enter flex w-full justify-end"><div className="max-w-[min(100%,44rem)] rounded-2xl rounded-tr-sm bg-white/[0.04] px-5 py-3 text-[15px] leading-[1.7] text-neutral-100 ring-1 ring-white/[0.06] whitespace-pre-wrap">{text}</div></div>)}
      {working && !streaming && <div className="chat-message-enter flex justify-start pl-4"><div className="thinking-shimmer h-[2px] w-16 rounded-full" /></div>}
    </div>
  </ScrollFollow>;
});

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

function ThinkingIndicator() {
  return <div className="my-4 flex items-center gap-[9px]">
    <PulseDot size={7}/>
    <span className="text-muted-foreground text-[12.5px] bg-[linear-gradient(90deg,var(--color-muted-foreground)_0%,var(--color-foreground)_50%,var(--color-muted-foreground)_100%)] bg-[length:200%_100%] bg-clip-text text-transparent animate-[shimmer_2s_linear_infinite]">Thinking…</span>
  </div>;
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
      <h2 className="font-display text-lg font-medium tracking-tight text-white">{title}</h2>
      <p className="mt-2 text-sm leading-relaxed text-neutral-500">{copy}</p>
    </div>
  </div>;
}

function ItemView({ item, onResolve, errorContext }: { item: ConversationItem; onResolve: (eventId: number, decision: string) => void; errorContext?: { provider?: string; snapshot: UsageSnapshot | null } }) {
  if (item.type === "message") {
    if (item.role === "user") return <div className="chat-message-enter flex w-full justify-end"><div className="max-w-[min(100%,44rem)] rounded-2xl rounded-tr-sm bg-white/[0.04] px-5 py-3 text-[15px] leading-[1.7] text-neutral-100 ring-1 ring-white/[0.06] whitespace-pre-wrap">{item.text}</div></div>;
    return <div className="chat-message-enter flex w-full justify-start"><div className="relative max-w-[min(100%,44rem)] py-1 pl-1 text-neutral-300">{item.status === "streaming" && !item.text.trim() ? <div className="thinking-shimmer h-[2px] w-16 rounded-full" /> : <Markdown text={item.text} dim={item.status === "streaming"} />}</div></div>;
  }
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
  if (streaming) return <div className="my-4 flex items-center gap-[9px] before:content-[''] before:w-[7px] before:h-[7px] before:rounded-full before:bg-muted-foreground/50 before:animate-[thinking-pulse_1.6s_ease-in-out_infinite]"><span className="text-muted-foreground text-[12.5px] bg-[linear-gradient(90deg,var(--color-muted-foreground)_0%,var(--color-foreground)_50%,var(--color-muted-foreground)_100%)] bg-[length:200%_100%] bg-clip-text text-transparent animate-[shimmer_2s_linear_infinite]">{lastLine(text) || "Thinking…"}</span></div>;
  return <details className="my-[15px] group [&_summary::-webkit-details-marker]:hidden">
    <summary className="inline-flex items-center gap-2 text-muted-foreground text-[13px] py-1 cursor-pointer transition-colors hover:text-foreground"><ChevronRight size={12} className="text-muted-foreground/70 transition-transform group-open:rotate-90" aria-hidden="true" />Thought for a moment</summary>
    <div className="mt-2 pl-[15px] border-l-[1.5px] border-border text-muted-foreground text-[12.5px] leading-relaxed"><Markdown text={text}/></div>
  </details>;
}

function ActivityGroup({ items }: { items: ConversationItem[] }) {
  const live = items.some(item => item.status === "inProgress" || item.status === "streaming");
  const [open, setOpen] = useState(false);
  const expanded = open || live;
  return <div className="my-3">
    <button className="inline-flex items-center gap-[9px] text-muted-foreground text-[12px] py-1 text-left transition-colors hover:text-foreground group" onClick={() => setOpen(value => !value)}>
      {live ? <PulseDot size={7}/> : <Pencil size={13} className="text-muted-foreground/70" aria-hidden="true" />}
      <span>{summarize(items)}</span>
      <ChevronDown size={13} className={`text-muted-foreground/70 transition-transform opacity-70 ${expanded ? "rotate-180" : ""}`} aria-hidden="true" />
    </button>
    {expanded && <div className="mt-[5px] pl-[21px] border-l border-border/50 grid gap-[1px]">
      {items.map(item => <ActionRow key={item.key} item={item}/>)}
    </div>}
  </div>;
}

function ActionRow({ item }: { item: ConversationItem }) {
  const [open, setOpen] = useState(false);
  const output = String(item.data.aggregatedOutput ?? item.data.output ?? "");
  const live = item.status === "inProgress" || item.status === "streaming";
  const { label, meta } = actionLabel(item);
  return <div className="min-w-0">
    <button className="w-full flex items-center gap-[9px] min-h-[26px] py-1 pr-2 rounded-md text-left text-muted-foreground text-[12px] hover:text-foreground disabled:hover:text-muted-foreground transition-colors" disabled={!output} onClick={() => output && setOpen(value => !value)}>
      <span className="flex-none grid place-items-center text-muted-foreground/70">{live ? <PulseDot size={7}/> : VERB_ICON[verbOf(item)]}</span>
      <span className="min-w-0 overflow-hidden whitespace-nowrap text-ellipsis font-mono text-[11.5px]">{label}</span>
      {meta && <span className="flex-none text-muted-foreground/70 font-mono text-[10.5px]">{meta}</span>}
      {output && <ChevronRight size={12} className={`flex-none text-muted-foreground/70 transition-transform ${open ? "rotate-90" : ""}`} aria-hidden="true" />}
    </button>
    {open && output && <pre className="mt-[3px] mb-2 ml-[25px] p-2.5 max-h-[260px] overflow-auto border border-border rounded-xl bg-code text-muted-foreground font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap scrollbar-thin scrollbar-thumb-foreground/10">{output.slice(-4000)}</pre>}
  </div>;
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

function ApprovalCard({ item, onResolve }: { item: ConversationItem; onResolve: (eventId: number, decision: string) => void }) {
  const pending = item.status === "pending";
  const accepted = item.status === "accept" || item.status === "acceptForSession";
  return <div className="my-4 border border-warning/30 rounded-lg bg-warning/5 overflow-hidden">
    <header className="flex items-baseline gap-[9px] pt-3 px-[15px]"><b className="text-[13px] font-semibold text-foreground">{item.title || "Approval needed"}</b>{pending && <small className="text-warning text-[10.5px] tracking-[0.03em]">waiting for you</small>}</header>
    {item.text && <p className="mt-1.5 px-[15px] text-muted-foreground text-[12.5px] leading-relaxed">{item.text}</p>}
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

function DelegationRow({ item }: { item: ConversationItem }) {
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

function lastLine(text: string): string {
  const lines = text.trim().split("\n").filter(Boolean);
  return lines[lines.length - 1] ?? "";
}
function stringList(value: unknown) { return Array.isArray(value) ? value.join("\n") : ""; }
function planSteps(data: Record<string, unknown>): Array<{ step: string; status: string }> {
  return Array.isArray(data.plan) ? data.plan.filter((v): v is { step: string; status: string } => !!v && typeof v === "object" && "step" in v && "status" in v) : [];
}
