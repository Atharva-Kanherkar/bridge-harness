import { useEffect, useRef, useState } from "react";
import { AgentConversation } from "../components/AgentConversation";
import { TasksPane, type AgentFocus } from "../components/TasksPane";
import { bridgeApi } from "../api";
import { asWireKind } from "../transcript/wire";
import type { AgentEvent, Session, SessionEntry, SessionForestSnapshot } from "../types";

// Development-only component fixture: real conversation/pane components,
// deterministic provider-shaped data, and controls with local effects only.
const startedAt = new Date(Date.now() - 75000).toISOString();
const parent: Session = { id: "fixture-chat", harness: "codex", label: "Direct chat", title: "Make harness subagents observable", kind: "direct", status: "working", metricSource: "estimated", restorationMode: "fresh", continuationFidelity: "native", startedAt };
const children: Session[] = ["Trace event attribution", "Review lifecycle transitions", "Check subagent controls"].map((title, index) => ({ ...parent, id: `fixture-child-${index + 1}`, parentSessionId: parent.id, providerSessionId: `provider-child-${index + 1}`, kind: "harness_subagent", title, label: title, status: index === 1 ? "waiting" : "working" }));
function event(id: number, sessionId: string, kind: string, rest: Partial<AgentEvent> = {}): AgentEvent {
  return { id, sessionId, sequence: id, protocolVersion: 1, kind: asWireKind(kind), itemId: null, role: null, status: null, title: null, text: null, data: {}, providerMeta: { adapter: "codex" }, createdAt: new Date().toISOString(), ...rest };
}
function entries(child: Session, approved: boolean): SessionEntry[] {
  const list = [
    event(1, child.id, "message.completed", { itemId: "reply", role: "assistant", text: child.id.endsWith("2") ? "The child turn must never settle the parent. I’m checking the router’s state transitions." : "Child events now have their own session. I’m checking the remaining event paths." }),
    event(2, child.id, "command.completed", { itemId: "read", title: "Read event router", status: "completed", data: { command: "rg subagent live_turn.rs", aggregatedOutput: "Child frames route before parent turn handling." } }),
  ];
  if (child.id.endsWith("2")) {
    list.push(event(3, child.id, "permission.requested", { itemId: "approval", title: "Run this command?", status: "pending", data: { requestId: "fixture-permission", interactionKind: "permission", command: "cargo test harness_subagents", actions: [{ id: "accept", decision: "accept", label: "Allow once" }, { id: "decline", decision: "decline", label: "Decline" }] } }));
    if (approved) list.push(event(4, child.id, "permission.resolved", { status: "accepted", data: { requestEventId: 3, decision: "accept" } }));
  }
  return list.map((item, index) => ({ id: `${child.id}-entry-${index}`, sessionId: child.id, parentEntryId: index ? `${child.id}-entry-${index - 1}` : null, sequence: item.sequence, semanticSchemaVersion: 2, kind: item.kind === asWireKind("message.completed") ? "assistant.message" : String(item.kind), payload: { itemId: item.itemId, role: item.role, title: item.title, status: item.status, text: item.text, data: { ...item.data, childSessionId: child.id, subagent: { sessionId: child.providerSessionId, agent: "codex", title: child.title } } }, contextVisibility: "eligible", createdAt: startedAt }));
}
export function HarnessSubagentsPreview() {
  const [sessions, setSessions] = useState([parent, ...children]);
  const [focus, setFocus] = useState<AgentFocus>();
  const [pinned, setPinned] = useState<Set<string>>(new Set());
  const current = useRef(sessions); current.current = sessions;
  const approved = useRef(false);
  useEffect(() => {
    const originalForest = bridgeApi.sessionForest;
    const originalDigest = bridgeApi.sessionForestDigest;
    const originalApproval = bridgeApi.resolveApproval;
    bridgeApi.sessionForest = async id => {
      const child = current.current.find(item => item.id === id)!;
      const rows = entries(child, approved.current);
      return { sessionId: id, entries: rows, head: { activeEntryId: rows.at(-1)?.id }, leaves: [], workerRuntimes: [], workerLeases: [], workerQueue: [], reasons: [], usage: [] } as unknown as SessionForestSnapshot;
    };
    bridgeApi.sessionForestDigest = async id => `${id}:${approved.current}`;
    bridgeApi.resolveApproval = async () => {
      approved.current = true;
      setSessions(items => items.map(item => item.id === "fixture-child-2" ? { ...item, status: "working" } : item));
      return { disposition: "resolved", interactionKind: "permission", status: "accepted", resolvedBy: "human", decision: "accept" };
    };
    return () => { bridgeApi.sessionForest = originalForest; bridgeApi.sessionForestDigest = originalDigest; bridgeApi.resolveApproval = originalApproval; };
  }, []);
  const rootEvents = [
    event(1, parent.id, "message.completed", { itemId: "user-ask", role: "user", text: "Check attribution, lifecycle and Stop independently using three subagents." }),
    event(2, parent.id, "message.completed", { itemId: "parent-reply", role: "assistant", text: "I started three subagents. Their progress and approvals are available in Agents while I continue reviewing the integration." }),
    ...sessions.slice(1).flatMap((child, index) => {
      const data = { childSessionId: child.id, subagent: { sessionId: child.providerSessionId, agent: "codex", title: child.title } };
      return [event(3 + index * 2, parent.id, "subagent.started", { data, status: "working" }), ...(child.status === "stopped" ? [event(4 + index * 2, parent.id, "subagent.completed", { data, status: "interrupted" })] : [])];
    }),
  ];
  const live = sessions.slice(1).map((child, index) => event(2, child.id, "tool.progress", { title: ["Reading the event router", "Waiting for permission", "Checking turn/interrupt"][index] }));
  const show = (sessionId: string) => setFocus({ sessionId, nonce: Date.now() });
  return <div className="flex h-screen flex-col bg-background text-foreground">
    <header className="flex h-12 shrink-0 items-center gap-3 border-b border-border px-5 text-[13px]"><b className="font-display text-base">Bridge</b><span className="text-muted-foreground">Harness subagents · component fixture</span><span className="ml-auto flex gap-3"><button onClick={() => document.documentElement.classList.remove("dark")}>Light</button><button onClick={() => document.documentElement.classList.add("dark")}>Dark</button></span></header>
    <div className="grid min-h-0 flex-1 grid-cols-[minmax(0,1fr)_420px]">
      <main className="flex min-h-0 flex-col"><div className="border-b border-border px-6 py-3"><h1 className="font-display text-lg">Make harness subagents observable</h1><p className="mt-1 text-xs text-muted-foreground">Direct · Codex · Parent working</p></div><div className="relative min-h-0 flex-1"><AgentConversation session={parent} events={rootEvents} onResolve={() => undefined} onOpenAgent={show} working /></div></main>
      <aside aria-label="Agents pane" className="flex min-h-0 flex-col border-l border-border bg-card"><div className="border-b border-border px-3 py-3 text-[13px] font-medium">Agents</div><div className="min-h-0 flex-1"><TasksPane chatSessionId={parent.id} sessions={sessions} liveEvents={live} pinned={pinned} focus={focus} onTogglePin={id => setPinned(old => { const next = new Set(old); if (next.has(id)) next.delete(id); else next.add(id); return next; })} onOpenSession={show} onStopWorker={async id => setSessions(items => items.map(item => item.id === id ? { ...item, status: "stopped", endedAt: new Date().toISOString() } : item))} /></div></aside>
    </div>
  </div>;
}
