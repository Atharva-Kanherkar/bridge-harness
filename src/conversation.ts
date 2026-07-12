import type { AgentEvent } from "./types";

export type ConversationItemType = "message" | "reasoning" | "activity" | "plan" | "approval" | "error" | "diff" | "artifact" | "delegation";
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
      if (!event.text) continue;
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
    if (event.kind === "delegation.spawned" || event.kind === "delegation.result") {
      items.set(itemKey, { key:itemKey, type:"delegation", eventId:event.id, role:"system", status:event.status ?? undefined, title:event.title ?? undefined, text:event.text ?? "", data:event.data, sequence:event.sequence }); continue;
    }
    if (event.kind === "approval.requested") {
      items.set(`approval:${event.id}`, { key:`approval:${event.id}`, type:"approval", eventId:event.id, status:"pending", title:event.title ?? "Approval required", text:event.text ?? "", data:event.data, sequence:event.sequence }); continue;
    }
    if (event.kind === "approval.resolved") {
      const requestId = Number(event.data.requestEventId); const approval = items.get(`approval:${requestId}`); if (approval) approval.status = String(event.data.decision ?? event.status ?? "resolved"); continue;
    }
    const type: ConversationItemType = event.kind.startsWith("message.") ? "message" : event.kind.startsWith("reasoning.") ? "reasoning" : event.kind.startsWith("diff.") || event.kind.startsWith("file_change.") ? "diff" : event.kind.startsWith("artifact.") ? "artifact" : event.kind === "error" ? "error" : "activity";
    if (type === "reasoning" && !(event.text || stringList(event.data.summary))) continue;
    if (type === "message" && !event.text && !items.has(itemKey)) continue;
    const existing = items.get(itemKey);
    const next: ConversationItem = existing ?? { key:itemKey, type, eventId:event.id, role:event.role ?? undefined, status:event.status ?? undefined, title:event.title ?? undefined, text:"", data:{}, sequence:event.sequence };
    next.eventId = event.id; next.status = event.status ?? next.status; next.title = event.title ?? next.title; next.role = event.role ?? next.role;
    if (event.text) next.text = event.text; next.data = event.data; items.set(itemKey,next);
  }
  return [...items.values()]
    .map(item => item.type === "message" ? { ...item, text: stripWorkerResultBlocks(item.text) } : item)
    .filter(item => item.type !== "reasoning" || item.text.trim().length > 0)
    .filter(item => item.type !== "message" || item.text.trim().length > 0)
    .sort((a,b)=>a.sequence-b.sequence);
}

export function stripWorkerResultBlocks(text: string): string {
  const lines = text.split("\n");
  const kept: string[] = [];
  let index = 0;
  while (index < lines.length) {
    const trimmed = lines[index].trimStart();
    const tag = trimmed.startsWith("```") ? trimmed.replace(/^`+/, "").trim().toLowerCase() : "";
    if (tag.includes("bridge") && tag.includes("worker") && tag.includes("result")) {
      const closing = lines.findIndex((line, candidate) => candidate > index && line.trimStart().startsWith("```"));
      if (closing >= 0) {
        index = closing + 1;
        continue;
      }
    }
    kept.push(lines[index]);
    index += 1;
  }
  return kept.join("\n").trim();
}

function stringList(value:unknown){return Array.isArray(value)?value.join("\n"):"";}
