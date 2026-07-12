import type { AgentEvent } from "./types";

export type ConversationItemType = "message" | "reasoning" | "activity" | "plan" | "approval" | "error" | "diff" | "artifact";
export interface ConversationItem {
  key: string; type: ConversationItemType; eventId: number; role?: string; status?: string;
  title?: string; text: string; data: Record<string, unknown>; sequence: number;
}

export function reduceConversation(events: AgentEvent[]): ConversationItem[] {
  const items = new Map<string, ConversationItem>();
  for (const event of [...events].sort((a, b) => a.sequence - b.sequence)) {
    if (event.kind === "provider.unknown" || event.kind.startsWith("session.") || event.kind.startsWith("turn.") || event.kind === "usage.updated") continue;
    const itemKey = event.itemId ?? `${event.kind}:${event.id}`;
    if (event.kind === "message.delta" || event.kind === "reasoning.delta") {
      const type = event.kind.startsWith("message") ? "message" : "reasoning";
      const existing = items.get(itemKey) ?? { key:itemKey, type, eventId:event.id, role:event.role ?? undefined, status:"streaming", text:"", data:{}, sequence:event.sequence };
      existing.text += event.text ?? ""; existing.status = "streaming"; existing.eventId = event.id; items.set(itemKey, existing); continue;
    }
    if (event.kind.endsWith(".output_delta") || event.kind === "diff.delta" || event.kind === "tool.progress") {
      const type: ConversationItemType = event.kind.startsWith("diff") ? "diff" : "activity";
      const existing = items.get(itemKey) ?? { key:itemKey, type, eventId:event.id, status:event.status ?? "inProgress", title:event.title ?? undefined, text:"", data:event.data, sequence:event.sequence };
      existing.text += event.text ?? ""; existing.status = event.status ?? existing.status; existing.eventId = event.id; existing.data = { ...existing.data, ...event.data }; items.set(itemKey, existing); continue;
    }
    if (event.kind === "plan.updated" || event.kind.startsWith("plan.")) {
      items.set("current-plan", { key:"current-plan", type:"plan", eventId:event.id, status:event.status ?? undefined, title:event.title ?? "Plan", text:event.text ?? "", data:event.data, sequence:event.sequence }); continue;
    }
    if (event.kind === "approval.requested") {
      items.set(`approval:${event.id}`, { key:`approval:${event.id}`, type:"approval", eventId:event.id, status:"pending", title:event.title ?? "Approval required", text:event.text ?? "", data:event.data, sequence:event.sequence }); continue;
    }
    if (event.kind === "approval.resolved") {
      const requestId = Number(event.data.requestEventId); const approval = items.get(`approval:${requestId}`); if (approval) approval.status = String(event.data.decision ?? event.status ?? "resolved"); continue;
    }
    const type: ConversationItemType = event.kind.startsWith("message.") ? "message" : event.kind.startsWith("reasoning.") ? "reasoning" : event.kind.startsWith("diff.") || event.kind.startsWith("file_change.") ? "diff" : event.kind.startsWith("artifact.") ? "artifact" : event.kind === "error" ? "error" : "activity";
    const existing = items.get(itemKey);
    const next: ConversationItem = existing ?? { key:itemKey, type, eventId:event.id, role:event.role ?? undefined, status:event.status ?? undefined, title:event.title ?? undefined, text:"", data:{}, sequence:event.sequence };
    next.eventId = event.id; next.status = event.status ?? next.status; next.title = event.title ?? next.title; next.role = event.role ?? next.role;
    if (event.text) next.text = event.text; next.data = event.data; items.set(itemKey,next);
  }
  return [...items.values()].sort((a,b)=>a.sequence-b.sequence);
}
