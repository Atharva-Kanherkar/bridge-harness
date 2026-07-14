import type { AgentEvent, SessionEntry } from "./types";

export type ConversationItemType = "message" | "reasoning" | "activity" | "plan" | "approval" | "error" | "diff" | "artifact" | "delegation" | "checkpoint" | "compaction" | "branch-summary" | "raw";
export interface ConversationItem {
  key: string; type: ConversationItemType; eventId: number; role?: string; status?: string;
  title?: string; text: string; data: Record<string, unknown>; sequence: number; entryId?: string;
}

/** Select one root-to-leaf path without relying on input array order. */
export function selectActiveBranch(entries: SessionEntry[], activeLeafId: string | null): SessionEntry[] {
  if (!activeLeafId) return [];
  const leaf = entries.find((entry) => entry.id === activeLeafId);
  if (!leaf) return [];
  const byId = new Map(
    entries
      .filter((entry) => entry.sessionId === leaf.sessionId)
      .map((entry) => [entry.id, entry] as const),
  );
  const branch: SessionEntry[] = [];
  const visited = new Set<string>();
  let current: SessionEntry | undefined = byId.get(activeLeafId);
  while (current && !visited.has(current.id)) {
    branch.push(current);
    visited.add(current.id);
    current = current.parentEntryId ? byId.get(current.parentEntryId) : undefined;
  }
  return branch.reverse();
}

/** Project immutable forest entries into UI items with entry-derived, branch-stable keys. */
export function projectSessionConversation(entries: SessionEntry[], activeLeafId: string | null): ConversationItem[] {
  const items: ConversationItem[] = [];
  const approvalsBySequence = new Map<number, ConversationItem>();
  for (const entry of selectActiveBranch(entries, activeLeafId)) {
    if (entry.semanticSchemaVersion < 1 || entry.semanticSchemaVersion > 2) {
      throw new Error(`Unsupported semantic event schema version ${entry.semanticSchemaVersion} on entry ${entry.id}`);
    }
    if (entry.kind === "approval.resolved") {
      const nested = objectValue(entry.payload.data);
      const requestEventId = Number(entry.payload.requestEventId ?? nested.requestEventId);
      const request = approvalsBySequence.get(requestEventId);
      if (request) {
        request.status = stringValue(entry.payload.decision) ?? stringValue(nested.decision) ?? stringValue(entry.payload.status) ?? "resolved";
        request.data = { ...request.data, resolution: entry.payload };
        continue;
      }
    }
    const item = projectSessionEntry(entry);
    items.push(item);
    if (entry.kind === "approval.requested") approvalsBySequence.set(entry.sequence, item);
  }
  return items;
}

function projectSessionEntry(entry: SessionEntry): ConversationItem {
  const payload = entry.payload;
  const base = {
    key: `entry:${entry.id}`,
    entryId: entry.id,
    eventId: entry.sequence,
    sequence: entry.sequence,
    status: stringValue(payload.status),
    data: payload,
  };
  if (isRawProviderEntry(entry)) {
    return {
      ...base,
      type: "raw",
      title: stringValue(payload.title) ?? "Raw provider event",
      text: stringValue(payload.text) ?? "",
      data: { ...payload, collapsed: true, inspectable: true },
    };
  }
  switch (entry.kind) {
    case "user.message":
    case "assistant.message":
      return {
        ...base,
        type: "message",
        role: entry.kind === "user.message" ? "user" : stringValue(payload.role) ?? "assistant",
        text: stringValue(payload.text) ?? "",
      };
    case "checkpoint":
      return { ...base, type: "checkpoint", title: "Checkpoint", text: stringValue(payload.summary) ?? "" };
    case "compaction":
      return { ...base, type: "compaction", title: "Context compacted", text: stringValue(payload.summary) ?? "" };
    case "compaction.requested":
      return { ...base, type: "compaction", title: "Compaction requested", text: stringValue(payload.reason) ?? "" };
    case "compaction.failed":
      return { ...base, type: "compaction", status: "failed", title: "Compaction failed", text: stringValue(payload.reason) ?? "" };
    case "branch.summary":
      return { ...base, type: "branch-summary", title: "Branch summary", text: stringValue(payload.summary) ?? "" };
    case "error":
      return { ...base, type: "error", status: stringValue(payload.status) ?? "failed", title: stringValue(payload.title) ?? "Agent error", text: errorTextFromPayload(payload) };
    default:
      return {
        ...base,
        type: entry.kind === "approval.requested" || entry.kind === "approval.resolved" ? "approval" : entry.kind === "artifact.created" ? "artifact" : entry.kind.startsWith("delegation.") || entry.kind === "worker.result" ? "delegation" : "activity",
        role: stringValue(payload.role),
        title: stringValue(payload.title) ?? humanizeKind(entry.kind),
        text: stringValue(payload.text) ?? stringValue(payload.summary) ?? stringValue(payload.reason) ?? "",
      };
  }
}

/** Pull a human-readable error string from an error entry, tolerant of provider shapes. */
function errorTextFromPayload(payload: Record<string, unknown>): string {
  const direct = stringValue(payload.text);
  if (direct) return direct;
  const data = objectValue(payload.data);
  const error = objectValue(data.error);
  return stringValue(error.message) ?? stringValue(data.message) ?? stringValue(data.reason) ?? "";
}

function isRawProviderEntry(entry: SessionEntry): boolean {
  return entry.kind.startsWith("provider.") || entry.kind.startsWith("raw.") || entry.contextVisibility.toLowerCase().includes("raw");
}

function stringValue(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

function objectValue(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : {};
}

function humanizeKind(kind: string): string {
  return kind.replace(/[._-]+/g, " ").replace(/^\w/, (letter) => letter.toUpperCase());
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
