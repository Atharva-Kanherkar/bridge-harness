import { useState } from "react";
import { AlertTriangle, Bot, Check, ChevronDown, ChevronRight, Circle, CornerDownRight, FileText, GitFork, LoaderCircle, Pencil, Search, SquareTerminal, Wrench, X } from "lucide-react";
import { projectSessionConversation, reduceConversation, type ConversationItem } from "../conversation";
import type { AgentEvent, Session, SessionEntry } from "../types";
import { Markdown } from "./Markdown";

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

export function AgentConversation({ session, events, forestEntries, activeLeafId, onResolve, preview }: { session?: Session; events: AgentEvent[]; forestEntries?: SessionEntry[]; activeLeafId?: string | null; onResolve: (eventId: number, decision: string) => void; preview?: boolean }) {
  const items = forestEntries?.length ? projectSessionConversation(forestEntries, activeLeafId ?? null) : reduceConversation(events);
  if (!session && !preview) return <Empty title="No agent yet" copy="Open a workspace and Bridge starts the orchestrator for you."/>;
  if (!items.length) return <Empty title="What should we build?" copy={`${session?.label ?? "The orchestrator"} is ready. Describe the work — Bridge routes it to the right harness and model.`}/>;
  return <div className="conversation-scroll">
    <div className="conversation-col">
      {preview && <div className="preview-chip">Design preview — sample conversation</div>}
      {groupItems(items).map(entry => entry.kind === "group"
        ? <ActivityGroup key={entry.key} items={entry.items}/>
        : entry.kind === "raw-group" ? <RawEventGroup key={entry.key} items={entry.items}/>
        : <ItemView key={entry.item.key} item={entry.item} onResolve={onResolve}/>)}
    </div>
  </div>;
}

function Empty({ title, copy }: { title: string; copy: string }) {
  return <div className="conversation-empty"><Bot size={22}/><h2>{title}</h2><p>{copy}</p></div>;
}

function ItemView({ item, onResolve }: { item: ConversationItem; onResolve: (eventId: number, decision: string) => void }) {
  if (item.type === "message") {
    if (item.role === "user") return <div className="user-turn"><div className="user-bubble">{item.text}</div></div>;
    return <div className="agent-prose">{item.status === "streaming" && !item.text.trim() ? <span className="thinking-line shimmer">Working…</span> : <Markdown text={item.text}/>}</div>;
  }
  if (item.type === "reasoning") return <Reasoning item={item}/>;
  if (item.type === "plan") return <PlanCard item={item}/>;
  if (item.type === "approval") return <ApprovalCard item={item} onResolve={onResolve}/>;
  if (item.type === "delegation") return <DelegationRow item={item}/>;
  if (item.type === "checkpoint" || item.type === "compaction" || item.type === "branch-summary") return <ForestCard item={item}/>;
  if (item.type === "raw") return <RawEvent item={item}/>;
  if (item.type === "error") return <div className="error-row"><AlertTriangle size={14}/><div><b>Agent error</b><p>{item.text || "The adapter reported an error."}</p></div></div>;
  return <ActivityGroup items={[item]}/>;
}

function ForestCard({ item }: { item: ConversationItem }) {
  const label = item.type === "checkpoint" ? "Checkpoint" : item.type === "compaction" ? "Context" : "Branch";
  return <div className={`forest-card ${item.type}`}>
    <header><GitFork size={13}/><b>{item.title || label}</b><small>{item.status || "durable"}</small></header>
    {item.text && <p>{item.text}</p>}
    {item.data.reason ? <code>{String(item.data.reason)}</code> : null}
  </div>;
}

function RawEvent({ item }: { item: ConversationItem }) {
  return <details className="raw-event">
    <summary><SquareTerminal size={12}/>{item.title || "Raw provider event"}<small>inspect</small></summary>
    <pre>{JSON.stringify(item.data, null, 2)}</pre>
  </details>;
}

function RawEventGroup({ items }: { items: ConversationItem[] }) {
  return <details className="raw-event raw-group">
    <summary><SquareTerminal size={12}/>{items.length} raw provider event{items.length === 1 ? "" : "s"}<small>inspect</small></summary>
    <div>{items.map(item => <RawEvent key={item.key} item={item}/>)}</div>
  </details>;
}

function Reasoning({ item }: { item: ConversationItem }) {
  const streaming = item.status === "streaming";
  const text = item.text || stringList(item.data.summary);
  if (streaming) return <div className="thinking-live"><span className="thinking-line shimmer">{lastLine(text) || "Thinking…"}</span></div>;
  return <details className="thinking">
    <summary><ChevronRight size={12} className="chev"/>Thought for a moment</summary>
    <div className="thinking-body"><Markdown text={text}/></div>
  </details>;
}

function ActivityGroup({ items }: { items: ConversationItem[] }) {
  const live = items.some(item => item.status === "inProgress" || item.status === "streaming");
  const [open, setOpen] = useState(false);
  const expanded = open || live;
  return <div className={`activity-group ${expanded ? "open" : ""}`}>
    <button className="activity-summary" onClick={() => setOpen(value => !value)}>
      {live ? <LoaderCircle size={13} className="spin"/> : <Pencil size={13}/>}
      <span>{summarize(items)}</span>
      <ChevronDown size={13} className="chev"/>
    </button>
    {expanded && <div className="activity-list">
      {items.map(item => <ActionRow key={item.key} item={item}/>)}
    </div>}
  </div>;
}

function ActionRow({ item }: { item: ConversationItem }) {
  const [open, setOpen] = useState(false);
  const output = String(item.data.aggregatedOutput ?? item.data.output ?? "");
  const live = item.status === "inProgress" || item.status === "streaming";
  const { label, meta } = actionLabel(item);
  return <div className="action-line">
    <button className="action-head" onClick={() => output && setOpen(value => !value)} data-expandable={!!output}>
      <span className="action-icon">{live ? <LoaderCircle size={12} className="spin"/> : VERB_ICON[verbOf(item)]}</span>
      <span className="action-label">{label}</span>
      {meta && <span className="action-meta">{meta}</span>}
      {output && <ChevronRight size={12} className={`chev ${open ? "down" : ""}`}/>}
    </button>
    {open && output && <pre className="action-output">{output.slice(-4000)}</pre>}
  </div>;
}

function PlanCard({ item }: { item: ConversationItem }) {
  return <div className="plan-card">
    <header><FileText size={13}/><b>{item.title || "Plan"}</b></header>
    {planSteps(item.data).map((step, index) => <div className={`plan-step ${step.status}`} key={`${step.step}-${index}`}>
      {step.status === "completed" ? <Check size={12}/> : step.status === "inProgress" ? <LoaderCircle className="spin" size={12}/> : <Circle size={8}/>}
      <span>{step.step}</span>
    </div>)}
  </div>;
}

function ApprovalCard({ item, onResolve }: { item: ConversationItem; onResolve: (eventId: number, decision: string) => void }) {
  return <div className="approval">
    <header><b>{item.title || "Approval needed"}</b><small>waiting for you</small></header>
    {item.text && <p>{item.text}</p>}
    {item.data.command ? <code>{String(item.data.command)}</code> : null}
    {item.data.cwd ? <small className="approval-cwd">{String(item.data.cwd)}</small> : null}
    {item.status === "pending"
      ? <div className="approval-actions">
          <button onClick={() => onResolve(item.eventId, "decline")}><X size={12}/> Decline</button>
          <button onClick={() => onResolve(item.eventId, "acceptForSession")}>Allow for session</button>
          <button className="approve" onClick={() => onResolve(item.eventId, "accept")}><Check size={12}/> Allow once</button>
        </div>
      : <div className="approval-resolved"><Check size={12}/> {item.status}</div>}
  </div>;
}

function DelegationRow({ item }: { item: ConversationItem }) {
  const isResult = "delivered" in item.data;
  const model = String(item.data.modelLabel ?? item.data.model ?? "");
  const effort = item.data.effort ? String(item.data.effort) : "";
  const [open, setOpen] = useState(false);
  return <div className={`delegation ${isResult ? "result" : ""}`}>
    <button className="delegation-head" onClick={() => item.text && setOpen(value => !value)}>
      {isResult ? <CornerDownRight size={13}/> : <GitFork size={13}/>}
      <span>{isResult ? "Subagent finished" : "Delegated"}{titleAddsInfo(item, isResult) && <b> · {item.title}</b>}</span>
      {model && <em>{model}{effort ? ` · ${effort}` : ""}</em>}
      {item.text && <ChevronRight size={12} className={`chev ${open ? "down" : ""}`}/>}
    </button>
    {open && item.text && <div className="delegation-body"><Markdown text={item.text}/></div>}
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
