import { stripBridgeFences } from "./humanize";
import type { AgentEvent, SessionEntry } from "./types";

export type ConversationItemType = "message" | "reasoning" | "activity" | "plan" | "approval" | "permission" | "question" | "error" | "diff" | "artifact" | "delegation" | "checkpoint" | "compaction" | "branch-summary" | "raw";
export interface ConversationItem {
  key: string; type: ConversationItemType; eventId: number; role?: string; status?: string;
  title?: string; text: string; data: Record<string, unknown>; sequence: number; entryId?: string;
  /**
   * Provider item id from the live event (`AgentEvent.itemId`) or the durable
   * payload. Live events do not copy this into `data`; durable ones do.
   */
  itemId?: string;
  /**
   * The identity this item shares with its counterpart on the other
   * projection (live vs durable), when it has one. `eventId` is not it: on
   * the live side it is the session-event table's own autoincrement id, and
   * on the durable side it is the forest entry's sequence — two unrelated
   * numbering spaces that happen to collide by coincidence, not by design.
   * Prefer this field over `eventId` for any merge that needs to recognize a
   * streamed item and its persisted twin as the same logical thing. See
   * `itemIdentity`.
   */
  identity?: string;
}

/**
 * Domain id keys, tried in priority order, that identify one logical item —
 * a tool call, an approval, a permission, a question — the same way whether
 * it is read off the live event stream or off a persisted forest entry. Both
 * sides carry these inside their own `data`/`payload` bag (the backend
 * writes the same JSON body to both the session-event row and, once it goes
 * durable, the forest entry's payload); the autoincrement ids each store
 * assigns around that body do not agree, and were never meant to.
 */
const IDENTITY_KEYS = ["itemId", "approvalId", "requestId", "questionId"] as const;

/**
 * The identity a conversation item shares with its counterpart on the other
 * projection, falling back to something still unique — but not
 * cross-projection-stable — when the item carries none of the known domain
 * ids. Both `reduceConversation` and `projectSessionConversation` populate
 * `item.identity` with this before returning, so a caller merging live and
 * durable lists never has to reach for `eventId`.
 */
export function itemIdentity(item: Pick<ConversationItem, "data" | "entryId" | "eventId" | "type" | "itemId">): string {
  if (typeof item.itemId === "string" && item.itemId) return item.itemId;
  for (const key of IDENTITY_KEYS) {
    const value = item.data[key];
    if (typeof value === "string" && value) return value;
  }
  // A durable-only card (checkpoint, compaction, branch summary) has no live
  // twin to line up with, so the entry id is unique enough. A live-only item
  // with none of the above falls back to its own event id.
  return item.entryId ? `entry:${item.entryId}` : `${item.type}:${item.eventId}`;
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

/** Lifecycle kinds whose started/completed entries describe one tool call. */
const LIFECYCLE_KINDS = new Set([
  "tool.started", "tool.completed",
  "command.started", "command.completed",
  "file_change.started", "file_change.completed",
  "reasoning.started", "reasoning.completed",
  "item.started", "item.completed",
  // A mirrored child approval is blocked-then-resolved under one item id; folding
  // keeps the durable projection from showing both halves as separate alerts.
  "delegation.blocked",
]);

/** Project immutable forest entries into UI items with entry-derived, branch-stable keys. */
export function projectSessionConversation(entries: SessionEntry[], activeLeafId: string | null): ConversationItem[] {
  const items: ConversationItem[] = [];
  const approvalsBySequence = new Map<number, ConversationItem>();
  const interactionsBySequence = new Map<string, ConversationItem>();
  const lifecycleByItemId = new Map<string, ConversationItem>();
  let compactionMaintenanceActive = false;
  for (const entry of selectActiveBranch(entries, activeLeafId)) {
    if (entry.semanticSchemaVersion < 1 || entry.semanticSchemaVersion > 2) {
      throw new Error(`Unsupported semantic event schema version ${entry.semanticSchemaVersion} on entry ${entry.id}`);
    }
    // Lifecycle plumbing the live reducer has always dropped. The projection
    // used to let it through, so `session.started` and `session.status`
    // replayed as collapsed "used tools" groups before and around the user's
    // messages, but only after a reload, which is exactly what made them read
    // as a glitch. Deliberately narrower than the reducer's filter:
    // `session.model_changed` is a bespoke row that reports what the switch
    // carried, and `provider.unknown` replays as a collapsed raw entry a
    // reviewer can inspect — both are wanted in replay, neither exists live.
    if (isLifecycleNoise(entry.kind)) continue;
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
    const interactionKind = interactionType(entry.kind);
    if (interactionKind && !entry.kind.endsWith(".requested")) {
      const nested = objectValue(entry.payload.data);
      const requestEventId = Number(entry.payload.requestEventId ?? nested.requestEventId);
      const request = interactionsBySequence.get(`${interactionKind}:${requestEventId}`);
      if (request) {
        request.status = stringValue(entry.payload.status) ?? stringValue(nested.status) ?? stringValue(nested.decision) ?? "resolved";
        request.data = { ...request.data, ...nested, resolution: entry.payload };
        continue;
      }
    }
    if (entry.kind === "compaction.requested") compactionMaintenanceActive = true;
    const item = projectSessionEntry(entry);
    const internalCompactionMessage = item.type === "message"
      && (compactionMaintenanceActive || isInternalCompactionEnvelope(item.text, item.data));
    if (entry.kind === "compaction" || entry.kind === "compaction.failed") {
      compactionMaintenanceActive = false;
    }
    if (internalCompactionMessage) continue;
    // Tool calls are stored as separate started/completed entries — fold them
    // into a single row so a stale "inProgress" ghost never lingers.
    const itemId = stringValue(entry.payload.itemId);
    if (itemId && LIFECYCLE_KINDS.has(entry.kind)) {
      const existing = lifecycleByItemId.get(itemId);
      if (existing) {
        existing.status = item.status ?? existing.status;
        existing.title = item.title ?? existing.title;
        if (item.text) existing.text = item.text;
        existing.data = { ...existing.data, ...item.data };
        existing.eventId = item.eventId;
        continue;
      }
      lifecycleByItemId.set(itemId, item);
    }
    items.push(item);
    if (entry.kind === "approval.requested") approvalsBySequence.set(entry.sequence, item);
    if (interactionKind && entry.kind.endsWith(".requested")) {
      interactionsBySequence.set(`${interactionKind}:${entry.sequence}`, item);
    }
  }
  return items
    .filter(item => item.type !== "reasoning" || item.text.trim().length > 0)
    .map(withIdentity);
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
        text: entry.kind === "assistant.message"
          ? stripBridgeFences(stringValue(payload.text) ?? "")
          : stringValue(payload.text) ?? "",
      };
    case "checkpoint":
      return { ...base, type: "checkpoint", title: "Checkpoint", text: stringValue(payload.summary) ?? "" };
    case "compaction":
      return { ...base, type: "compaction", title: "Context compacted", text: stringValue(payload.summary) ?? "" };
    case "compaction.requested":
      return { ...base, type: "compaction", title: "Compaction requested", text: compactionReasonLabel(stringValue(payload.reason)) };
    case "compaction.failed":
      return {
        ...base,
        type: "compaction",
        status: "failed",
        title: "Compaction failed",
        // New entries carry safe, classified copy. Keep the raw reason only as
        // a compatibility fallback for transcripts written by older builds.
        text: stringValue(payload.message) ?? stringValue(payload.reason) ?? "",
      };
    case "branch.summary":
      return { ...base, type: "branch-summary", title: "Branch summary", text: stringValue(payload.summary) ?? "" };
    case "error":
      return { ...base, type: "error", status: stringValue(payload.status) ?? "failed", title: stringValue(payload.title) ?? "Agent error", text: errorTextFromPayload(payload) };
    case "reasoning":
    case "reasoning.completed":
    case "reasoning.started":
    case "reasoning.delta":
      return {
        ...base,
        type: "reasoning",
        status: stringValue(payload.status) ?? "completed",
        title: stringValue(payload.title) ?? "Thought for a moment",
        text: reasoningDisplayText(stringValue(payload.text), payload),
      };
    default:
      return {
        ...base,
        // `file_change.*` is a diff, durably as well as live. The live reducer
        // has always typed it that way; the durable projection called it plain
        // activity, so a patch replayed from history lost the one label that
        // says "render me as a diff" and came back as a generic tool row.
        type: interactionType(entry.kind) ?? (entry.kind === "approval.requested" || entry.kind === "approval.resolved" ? "approval" : entry.kind === "artifact.created" ? "artifact" : entry.kind.startsWith("delegation.") || entry.kind === "worker.result" ? "delegation" : entry.kind.startsWith("file_change.") || entry.kind.startsWith("diff.") ? "diff" : entry.kind.startsWith("reasoning.") ? "reasoning" : "activity"),
        role: stringValue(payload.role),
        title: stringValue(payload.title) ?? humanizeKind(entry.kind),
        text: stringValue(payload.text) ?? stringValue(payload.summary) ?? stringValue(payload.reason) ?? "",
        // Flatten the stored wrapper: the inner event data (tool input, command,
        // output…) wins, so durable items render like live ones.
        data: { ...payload, ...objectValue(payload.data) },
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

function interactionType(kind: string): "permission" | "question" | undefined {
  if (kind.startsWith("permission.")) return "permission";
  if (kind.startsWith("question.")) return "question";
  return undefined;
}

function humanizeKind(kind: string): string {
  return kind.replace(/[._-]+/g, " ").replace(/^\w/, (letter) => letter.toUpperCase());
}

/**
 * A compaction reason, in the language of the transcript it appears in.
 *
 * These are wire values (`before_downgrade`), and a card that shows one to the
 * reader is showing plumbing. An unrecognised reason passes through unchanged:
 * the host owns this set and may add to it, and a raw value read once is better
 * than a wrong one read confidently.
 */
const COMPACTION_REASONS: Record<string, string> = {
  context_pressure: "Context was nearly full",
  response_reserve: "Reserving room for the reply",
  phase_boundary: "At a phase boundary",
  before_suspend: "Before suspending this session",
  before_downgrade: "Before switching models",
  before_shutdown: "Before shutting this session down",
  manual: "Asked for by hand",
};

export function compactionReasonLabel(reason?: string): string {
  if (!reason) return "";
  return COMPACTION_REASONS[reason] ?? reason;
}

/** Lifecycle plumbing with no conversational reading at all — filtered from
 *  the durable projection. The live reducer drops a superset (every
 *  `session.*`, plus `provider.unknown`, which replay renders as raw). */
export function isLifecycleNoise(kind: string): boolean {
  return (
    (kind.startsWith("session.") && kind !== "session.model_changed") ||
    kind.startsWith("turn.") ||
    kind === "usage.updated"
  );
}

export function reduceConversation(events: AgentEvent[]): ConversationItem[] {
  const items = new Map<string, ConversationItem>();
  const internalCompactionMessageKeys = new Set<string>();
  let compactionMaintenanceActive = false;
  let turnIndex = 0;
  for (const event of [...events].sort((a, b) => a.sequence - b.sequence)) {
    if (event.kind === "provider.unknown" || event.kind === "usage.updated") continue;
    if (event.kind.startsWith("turn.") || event.kind.startsWith("session.")) {
      if (event.kind === "turn.started") {
        turnIndex += 1;
      }
      if (event.kind === "turn.completed" || event.kind === "turn.failed" || event.kind === "session.idle") {
        for (const item of items.values()) {
          if (item.type === "reasoning" && item.status === "streaming") {
            item.status = "completed";
          }
        }
      }
      continue;
    }
    const fallbackReasoningKey = `reasoning:live:${turnIndex}`;
    const itemKey = event.itemId ?? (event.kind.startsWith("reasoning.") ? fallbackReasoningKey : `${event.kind}:${event.id}`);
    if (event.kind === "compaction.requested") compactionMaintenanceActive = true;
    if (event.kind.startsWith("message.")
      && (compactionMaintenanceActive || isInternalCompactionEnvelope(event.text ?? "", event.data))) {
      internalCompactionMessageKeys.add(itemKey);
    }
    if (event.kind === "compaction" || event.kind === "compaction.failed") {
      compactionMaintenanceActive = false;
    }
    if (event.kind === "message.delta" || event.kind === "reasoning.delta") {
      if (!event.text) continue;
      const type = event.kind.startsWith("message") ? "message" : "reasoning";
      const key = type === "reasoning" ? (event.itemId ?? fallbackReasoningKey) : itemKey;
      const existing = items.get(key) ?? { key, type, eventId:event.id, role:event.role ?? undefined, status:"streaming", text:"", data:{}, sequence:event.sequence };
      existing.text += event.text ?? ""; existing.status = "streaming"; existing.eventId = event.id; items.set(key, existing); continue;
    }
    if (event.kind.endsWith(".output_delta") || event.kind === "diff.delta" || event.kind === "tool.progress") {
      const type: ConversationItemType = event.kind.startsWith("diff") ? "diff" : "activity";
      const existing = items.get(itemKey) ?? { key:itemKey, type, eventId:event.id, status:event.status ?? "inProgress", title:event.title ?? undefined, text:"", data:event.data, sequence:event.sequence };
      existing.text += event.text ?? ""; existing.status = event.status ?? existing.status; existing.eventId = event.id; existing.data = { ...existing.data, ...event.data }; items.set(itemKey, existing); continue;
    }
    if (event.kind === "plan.updated" || event.kind.startsWith("plan.")) {
      const planKey = event.itemId ?? "current-plan";
      items.set(planKey, { key: planKey, type:"plan", eventId:event.id, status:event.status ?? undefined, title:event.title ?? "Plan", text:event.text ?? "", data:event.data, sequence:event.sequence }); continue;
    }
    // Every `delegation.*` frame is a delegation row. Listing them by name meant
    // a new one (a resumed warm worker, a steer) silently rendered as generic
    // tool activity; the durable projection has always matched on the prefix.
    if (event.kind.startsWith("delegation.")) {
      items.set(itemKey, { key:itemKey, type:"delegation", eventId:event.id, role:"system", status:event.status ?? undefined, title:event.title ?? undefined, text:event.text ?? "", data:event.data, sequence:event.sequence }); continue;
    }
    if (event.kind === "approval.requested") {
      items.set(`approval:${event.id}`, { key:`approval:${event.id}`, type:"approval", eventId:event.id, status:"pending", title:event.title ?? "Approval required", text:event.text ?? "", data:event.data, sequence:event.sequence }); continue;
    }
    if (event.kind === "approval.resolved") {
      const requestId = Number(event.data.requestEventId); const approval = items.get(`approval:${requestId}`); if (approval) approval.status = String(event.data.decision ?? event.status ?? "resolved"); continue;
    }
    const interactionKind = interactionType(event.kind);
    if (interactionKind && event.kind.endsWith(".requested")) {
      items.set(`${interactionKind}:${event.id}`, {
        key: `${interactionKind}:${event.id}`,
        type: interactionKind,
        eventId: event.id,
        status: event.status ?? "pending",
        title: event.title ?? (interactionKind === "permission" ? "Permission required" : "Question"),
        text: event.text ?? "",
        data: event.data,
        sequence: event.sequence,
      });
      continue;
    }
    if (interactionKind) {
      const requestId = Number(event.data.requestEventId);
      const interaction = items.get(`${interactionKind}:${requestId}`);
      if (interaction) {
        interaction.status = event.status ?? String(event.data.decision ?? "resolved");
        interaction.data = { ...interaction.data, ...event.data, resolution: event };
      }
      continue;
    }
    const type: ConversationItemType = event.kind.startsWith("message.") ? "message" : event.kind.startsWith("reasoning.") ? "reasoning" : event.kind.startsWith("diff.") || event.kind.startsWith("file_change.") ? "diff" : event.kind.startsWith("artifact.") ? "artifact" : event.kind === "error" ? "error" : "activity";
    if (type === "reasoning") {
      let target = items.get(itemKey);
      if (!target) {
        for (const candidate of [...items.values()].reverse()) {
          if (candidate.type === "reasoning" && candidate.status === "streaming") {
            target = candidate;
            break;
          }
        }
      }
      if (target) {
        target.status = event.status ?? "completed";
        if (event.text) target.text = event.text;
        else if (!target.text) {
          const display = reasoningDisplayText(null, event.data);
          if (display) target.text = display;
        }
        target.data = { ...target.data, ...event.data };
        target.eventId = event.id;
        if (event.itemId && target.key !== event.itemId) {
          items.delete(target.key);
          target.key = event.itemId;
          items.set(target.key, target);
        }
        continue;
      }
      if (!reasoningDisplayText(event.text, event.data)) continue;
    }
    if (type === "message" && !event.text && !items.has(itemKey)) continue;
    const existing = items.get(itemKey);
    const next: ConversationItem = existing ?? { key:itemKey, type, eventId:event.id, role:event.role ?? undefined, status:event.status ?? undefined, title:event.title ?? undefined, text:"", data:{}, sequence:event.sequence };
    next.eventId = event.id; next.status = event.status ?? next.status; next.title = event.title ?? next.title; next.role = event.role ?? next.role;
    if (event.text) next.text = event.text;
    else if (type === "reasoning" && !next.text) next.text = reasoningDisplayText(null, event.data);
    next.data = { ...next.data, ...event.data }; items.set(itemKey,next);
  }
  return [...items.values()]
    .map(item => item.type === "message" ? { ...item, text: stripBridgeFences(stripWorkerResultBlocks(item.text)) } : item)
    .filter(item => item.type !== "message" || !internalCompactionMessageKeys.has(item.key))
    .filter(item => item.type !== "reasoning" || item.text.trim().length > 0)
    .filter(item => item.type !== "message" || item.text.trim().length > 0)
    .sort((a,b)=>a.sequence-b.sequence)
    .map(item => withIdentity(stampLiveItemId(item)));
}

/** Live events keep `itemId` on the event, not in `data`. The reducer uses
 *  that value as `item.key` when the provider sent one; synthetic keys always
 *  contain a colon (`approval:7`, `command.started:9`, `reasoning:live:0`). */
function stampLiveItemId(item: ConversationItem): ConversationItem {
  if (item.itemId || item.key.includes(":")) return item;
  return { ...item, itemId: item.key };
}

/** Defense in depth for a backend-tagged maintenance frame. Content shape is
 * deliberately irrelevant: a user may legitimately ask for the checkpoint
 * schema, while a malformed maintenance reply or refusal is still internal. */
export function isInternalCompactionEnvelope(
  _text: string,
  data: Record<string, unknown> = {},
): boolean {
  return data.bridgeInternalOrigin === "compaction";
}

/**
 * The optimistic pending rows that have not yet come back as real user turns.
 *
 * Delivery is judged per row, in the row's **own** session: a pending message
 * for an aside must reconcile against the aside's slice of the live stream,
 * never against whichever session happens to be selected. The selected
 * session gets one extra source — its durable projection — because its forest
 * is the only one the app holds in memory; every other session's user turn
 * still arrives on the global live stream, which is enough.
 *
 * Returns the same array reference when nothing was delivered, so callers can
 * keep referential equality for render stability.
 */
export function undeliveredPending<T extends { sessionId: string; text: string }>(
  pending: readonly T[],
  liveEvents: AgentEvent[],
  selected: { sessionId?: string; durableUserTexts: ReadonlySet<string> },
): T[] {
  if (!pending.length) return pending as T[];
  const liveTexts = new Map<string, Set<string>>();
  const deliveredIn = (sessionId: string, text: string): boolean => {
    let texts = liveTexts.get(sessionId);
    if (!texts) {
      texts = new Set(
        reduceConversation(liveEvents.filter(event => event.sessionId === sessionId))
          .filter(item => item.type === "message" && item.role === "user")
          .map(item => item.text.trim()),
      );
      liveTexts.set(sessionId, texts);
    }
    if (texts.has(text)) return true;
    return sessionId === selected.sessionId && selected.durableUserTexts.has(text);
  };
  const next = pending.filter(item => !deliveredIn(item.sessionId, item.text.trim()));
  return next.length === pending.length ? (pending as T[]) : next;
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

function withIdentity(item: ConversationItem): ConversationItem {
  const itemId = item.itemId ?? stringValue(item.data.itemId);
  const next = itemId && item.itemId !== itemId ? { ...item, itemId } : item;
  return { ...next, identity: itemIdentity(next) };
}

/** Resolved reasoning text: the streamed `text`, or Codex's summary-only payload. */
export function reasoningDisplayText(text?: string | null, data: Record<string, unknown> = {}): string {
  if (text) return text;
  const summary = data.summary;
  if (typeof summary === "string") return summary;
  return stringList(summary);
}

/** A worker-result payload stamped onto assistant prose, if that is all the text is. */
export function workerResultSummary(text: string): string | undefined {
  const prefix = "[worker result]";
  if (!text.startsWith(prefix)) return undefined;
  const rest = text.slice(prefix.length).trim();
  return rest || undefined;
}

/* ── Tool-call display data ──────────────────────────────────────────────
   One tool call arrives in as many shapes as there are providers: a Claude
   `tool_use` block with a `name` and an `input`, a Codex ACP item with a `type`,
   an OpenCode part with a nested `state`. The renderer used to read those apart
   inline, one `data.foo ?? data.bar` at a time, which meant every new field the
   transcript wanted to show — an exit code, a patch — widened an ad-hoc bag of
   reads spread across a component. Naming the shape once means the summary row,
   the diff, and the terminal block cannot disagree about what a call was. */

export type ToolVerb = "edit" | "read" | "run" | "search" | "tool";

/** Which icon the row wears. A key, not a component: this module stays
 *  React-free so it can be tested and reused as plain data. */
export type ToolGlyph = "pencil" | "file-plus" | "file" | "terminal" | "search" | "globe" | "fork" | "list" | "wrench";

export type ToolStatus = "running" | "completed" | "failed" | "idle";

export interface ToolCallDisplay {
  verb: ToolVerb;
  glyph: ToolGlyph;
  /** Present tense, while it runs: "Editing". */
  doing: string;
  /** Past tense, once it has: "Edited". */
  done: string;
  /** What was acted on — a basename, a search pattern, a command. */
  target?: string;
  /** The full path, when `target` is only its basename. */
  path?: string;
  /** The command as typed, for the terminal block. */
  command?: string;
  additions?: number;
  deletions?: number;
  durationMs?: number;
  /** Present only where the provider actually reports one. */
  exitCode?: number;
  /** A unified diff this call carries, to render inline. */
  patch?: string;
  /** Everything else it produced. */
  output?: string;
  status: ToolStatus;
}

/**
 * Whether a string really is a unified diff.
 *
 * Deliberately stricter than `looksLikeDiff` in `components/highlight.ts`, and
 * deliberately not a call to it: that predicate is loose on purpose, because it
 * decides how to render output nobody has classified, and it lives in a module
 * that pulls in the whole syntax-highlighting stack. Claiming `patch` is a
 * stronger statement — the transcript will show this inline, by default — so it
 * wants a hunk header or a git header, nothing inferred.
 */
function carriesPatch(text: string | undefined): boolean {
  if (!text) return false;
  const sample = text.slice(0, 4000);
  return /^@@+ /m.test(sample) || /^diff --git /m.test(sample);
}

function numberValue(value: unknown): number | undefined {
  if (typeof value === "number") return Number.isFinite(value) ? value : undefined;
  if (typeof value === "string" && value.trim()) {
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : undefined;
  }
  return undefined;
}

function text(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value : undefined;
}

function basename(path: string): string {
  const parts = path.replace(/[/\\]+$/, "").split(/[/\\]/);
  return parts[parts.length - 1] || path;
}

/** Every place a provider has been seen to put an exit code. */
function readExitCode(data: Record<string, unknown>): number | undefined {
  const state = objectValue(data.state);
  const metadata = { ...objectValue(data.metadata), ...objectValue(state.metadata) };
  for (const candidate of [data.exitCode, data.exit_code, state.exitCode, state.exit_code, metadata.exitCode, metadata.exit_code, metadata.exit]) {
    const parsed = numberValue(candidate);
    if (parsed !== undefined) return Math.trunc(parsed);
  }
  return undefined;
}

/** The diff a file change carries, wherever the provider chose to hang it. */
function readPatch(item: ConversationItem, data: Record<string, unknown>, output: string | undefined): string | undefined {
  const state = objectValue(data.state);
  const metadata = { ...objectValue(data.metadata), ...objectValue(state.metadata) };
  for (const candidate of [data.patch, data.diff, data.unifiedDiff, state.diff, state.patch, metadata.diff, metadata.patch]) {
    const value = text(candidate);
    if (value) return value;
  }
  // ACP file changes arrive as a list of per-file changes, each with its own
  // diff. Joined in order so a multi-file edit reads as one patch.
  if (Array.isArray(data.changes)) {
    const joined = data.changes
      .map(change => {
        const entry = objectValue(change);
        return text(entry.diff) ?? text(entry.patch) ?? text(entry.unifiedDiff);
      })
      .filter((value): value is string => !!value)
      .join("\n");
    if (joined) return joined;
  }
  // Some providers only ever put the diff in the body. Taken last, and only
  // when it is unmistakably a diff.
  if (carriesPatch(item.text)) return item.text;
  if (carriesPatch(output)) return output;
  return undefined;
}

function readPath(data: Record<string, unknown>): string | undefined {
  const input = objectValue(data.input);
  const state = objectValue(data.state);
  const stateInput = objectValue(state.input);
  const direct = text(input.file_path) ?? text(input.notebook_path) ?? text(input.path)
    ?? text(data.path) ?? text(stateInput.filePath) ?? text(stateInput.file_path) ?? text(stateInput.path);
  if (direct) return direct;
  if (Array.isArray(data.changes)) {
    const first = objectValue(data.changes[0]);
    return text(first.path);
  }
  return undefined;
}

function readStatus(status: string | undefined): ToolStatus {
  if (status === "inProgress" || status === "streaming" || status === "running") return "running";
  if (status === "failed" || status === "error") return "failed";
  if (status === "completed") return "completed";
  return "idle";
}

/** The output behind a tool row: explicit output, else the item's own body. */
function readOutput(item: ConversationItem, data: Record<string, unknown>): string | undefined {
  const state = objectValue(data.state);
  const direct = text(data.aggregatedOutput) ?? text(data.output) ?? text(state.output);
  if (direct) return direct;
  const body = item.text ?? "";
  if (!body.trim()) return undefined;
  // A title echoed back as the body is not output, it is the title again.
  if (item.title && body.trim() === item.title.trim()) return undefined;
  return body;
}

/** Read one conversation item as the tool call it describes. */
export function toolCallDisplay(item: ConversationItem): ToolCallDisplay {
  const data = item.data;
  const path = readPath(data);
  const output = readOutput(item, data);
  const common = {
    path,
    output,
    additions: numberValue(data.additions),
    deletions: numberValue(data.deletions),
    durationMs: numberValue(data.durationMs),
    exitCode: readExitCode(data),
    status: readStatus(item.status),
  };
  const named = namedToolFacet(item, data);
  const command = named.command ?? (named.verb === "run" ? text(data.command) : undefined);
  return {
    ...common,
    ...named,
    path: named.path ?? path,
    command,
    // Only edits show a diff inline; a read whose body happens to be a diff is
    // still just output.
    patch: named.verb === "edit" ? readPatch(item, data, output) : undefined,
  };
}

export function parseCommandTokens(command: string): string[] {
  const trimmed = command.trim();
  const stripped = trimmed.replace(/^([A-Za-z_][A-Za-z0-9_]*=[^\s]+\s+)+/, "");
  const tokens: string[] = [];
  const regex = /[^\s"']+|"([^"]*)"|'([^']*)'/g;
  let match: RegExpExecArray | null;
  while ((match = regex.exec(stripped)) !== null) {
    tokens.push(match[1] ?? match[2] ?? match[0]);
  }
  return tokens;
}

function matchesAny(bin: string, names: string[]): boolean {
  const base = bin.split("/").pop() || bin;
  return names.includes(base);
}

function isFlag(arg: string): boolean {
  return /^--?[a-zA-Z0-9]/.test(arg);
}

function classifySingleCommand(cmd: string): {
  verb: ToolVerb;
  glyph: ToolGlyph;
  doing: string;
  done: string;
  target?: string;
  path?: string;
} | null {
  let clean = cmd.trim().replace(/^([A-Za-z_][A-Za-z0-9_]*=[^\s]+\s+)+/, "");
  clean = clean.replace(/^(?:builtin|command|sudo)\s+/, "");
  const tokens = parseCommandTokens(clean);
  if (!tokens.length) return null;

  const bin = tokens[0].toLowerCase();
  const args = tokens.slice(1);

  if (matchesAny(bin, ["cat", "head", "tail", "less", "more", "bat"])) {
    let idx = 0;
    while (idx < args.length) {
      if (args[idx] === "-n" || args[idx] === "-c") {
        idx += 2;
      } else if (isFlag(args[idx])) {
        idx += 1;
      } else {
        break;
      }
    }
    const filePath = args[idx];
    const target = filePath ? (filePath.split("/").pop() || filePath) : undefined;
    return {
      verb: "read",
      glyph: "file",
      doing: "Reading",
      done: "Read",
      target: target ?? filePath,
      path: filePath,
    };
  }

  if (matchesAny(bin, ["ls", "dir", "tree"])) {
    const nonFlags = args.filter(arg => !isFlag(arg));
    const dirPath = nonFlags[0];
    return {
      verb: "read",
      glyph: "file",
      doing: "Listing",
      done: "Listed",
      target: dirPath ? (dirPath.split("/").pop() || dirPath) : "directory",
      path: dirPath,
    };
  }

  if (matchesAny(bin, ["grep", "egrep", "fgrep", "rg", "ag", "ack"])) {
    const nonFlags = args.filter(arg => !isFlag(arg));
    const pattern = nonFlags[0];
    const filePath = nonFlags[1];
    return {
      verb: "search",
      glyph: "search",
      doing: "Searching",
      done: "Searched",
      target: pattern ? `“${pattern}”` : "files",
      path: filePath,
    };
  }

  if (matchesAny(bin, ["find", "fd", "locate", "which", "whereis", "wc", "stat", "file"])) {
    const nonFlags = args.filter(arg => !isFlag(arg));
    const target = nonFlags[0];
    return {
      verb: "search",
      glyph: "search",
      doing: "Searching",
      done: "Searched",
      target: target ? `“${target}”` : "files",
      path: target,
    };
  }

  if (bin === "git") {
    let subIdx = 0;
    while (subIdx < args.length && isFlag(args[subIdx])) {
      if (args[subIdx] === "-C" || args[subIdx] === "-c") subIdx += 2;
      else subIdx += 1;
    }
    const sub = args[subIdx]?.toLowerCase();
    if (!sub) return null;

    if (sub === "status") {
      return {
        verb: "read",
        glyph: "file",
        doing: "Checking",
        done: "Checked",
        target: "git status",
      };
    }
    if (sub === "diff") {
      return {
        verb: "read",
        glyph: "file",
        doing: "Inspecting",
        done: "Inspected",
        target: "git diff",
      };
    }
    if (sub === "log") {
      return {
        verb: "read",
        glyph: "file",
        doing: "Viewing",
        done: "Viewed",
        target: "git log",
      };
    }
    if (sub === "show") {
      return {
        verb: "read",
        glyph: "file",
        doing: "Inspecting",
        done: "Inspected",
        target: "git show",
      };
    }
    if (sub === "branch" || sub === "tag" || sub === "remote" || sub === "describe") {
      return {
        verb: "read",
        glyph: "file",
        doing: "Checking",
        done: "Checked",
        target: `git ${sub}`,
      };
    }
    return null;
  }

  return null;
}

export function hasUnquotedRedirect(command: string): boolean {
  let inSingle = false;
  let inDouble = false;
  for (let i = 0; i < command.length; i++) {
    const ch = command[i];
    if (ch === "\\" && !inSingle) {
      i++;
      continue;
    }
    if (ch === "'" && !inDouble) {
      inSingle = !inSingle;
      continue;
    }
    if (ch === '"' && !inSingle) {
      inDouble = !inDouble;
      continue;
    }
    if (!inSingle && !inDouble && ch === ">") {
      return true;
    }
  }
  return false;
}

export function classifyExploratoryCommand(rawCommand: string): {
  verb: ToolVerb;
  glyph: ToolGlyph;
  doing: string;
  done: string;
  target?: string;
  path?: string;
} | null {
  const trimmed = rawCommand.trim();
  if (!trimmed) return null;
  if (hasUnquotedRedirect(trimmed)) return null;

  const parts = trimmed.split(/\s*(?:&&|;|\|\|)\s*/).filter(Boolean);
  if (parts.length > 1) {
    const classifiedParts = parts.map(classifySingleCommand);
    if (classifiedParts.some(c => c === null)) return null;
    const first = classifiedParts[0]!;
    return {
      verb: first.verb,
      glyph: first.glyph,
      doing: "Exploring",
      done: "Explored",
      target: trimmed,
    };
  }

  if (trimmed.includes("|")) {
    const pipeParts = trimmed.split(/\s*\|\s*/).filter(Boolean);
    const classifiedPipe = pipeParts.map(classifySingleCommand);
    if (classifiedPipe.some(c => c === null)) return null;
    const first = classifiedPipe[0]!;
    return {
      verb: first.verb,
      glyph: first.glyph,
      doing: first.doing,
      done: first.done,
      target: trimmed,
      path: first.path,
    };
  }

  return classifySingleCommand(trimmed);
}

/** Verb, glyph, wording and target — the half of the shape that depends on
 *  *which* tool ran rather than on how it went. */
function namedToolFacet(item: ConversationItem, data: Record<string, unknown>): {
  verb: ToolVerb; glyph: ToolGlyph; doing: string; done: string; target?: string; command?: string; path?: string;
} {
  const input = objectValue(data.input);
  const name = text(data.name);
  const dataType = String(data.type ?? "");
  const title = item.title ?? "";
  const path = readPath(data);
  const file = path ? basename(path) : undefined;

  if (name) {
    const key = name.toLowerCase();
    if (key === "bash" || key === "shell") {
      const command = text(input.command) ?? text(data.command);
      const exploratory = command ? classifyExploratoryCommand(command) : null;
      if (exploratory) {
        return { ...exploratory, command };
      }
      return { verb: "run", glyph: "terminal", doing: "Running", done: "Ran", target: command ?? (title || "command"), command };
    }
    if (key === "read") return { verb: "read", glyph: "file", doing: "Reading", done: "Read", target: file ?? "file" };
    if (key === "edit" || key === "multiedit" || key === "notebookedit") return { verb: "edit", glyph: "pencil", doing: "Editing", done: "Edited", target: file ?? "file" };
    if (key === "write") return { verb: "edit", glyph: "file-plus", doing: "Writing", done: "Wrote", target: file ?? "file" };
    if (key === "grep" || key === "glob") {
      const pattern = text(input.pattern);
      return { verb: "search", glyph: "search", doing: "Searching", done: "Searched", target: pattern ? `“${pattern}”` : "files" };
    }
    if (key === "websearch") return { verb: "search", glyph: "globe", doing: "Searching the web", done: "Searched the web", target: text(input.query) };
    if (key === "webfetch") return { verb: "search", glyph: "globe", doing: "Fetching", done: "Fetched", target: text(input.url) };
    if (key === "task") return { verb: "tool", glyph: "fork", doing: "Delegating", done: "Delegated", target: text(input.description) };
    if (key === "todowrite") return { verb: "tool", glyph: "list", doing: "Updating tasks", done: "Updated tasks" };
    if (key.startsWith("mcp__")) {
      const parts = name.replace(/^mcp__/, "").split("__");
      const server = parts[0] ?? name;
      const tool = parts.slice(1).join(" ").replaceAll("_", " ") || name;
      return { verb: "tool", glyph: "wrench", doing: `Using ${server}`, done: `Used ${server}`, target: tool };
    }
    return { verb: "tool", glyph: "wrench", doing: `Using ${name}`, done: `Used ${name}`, target: title || undefined };
  }

  // Codex- and OpenCode-shaped items, identified by their item type.
  if (item.type === "diff" || dataType.includes("patch") || dataType.includes("fileChange")) {
    return { verb: "edit", glyph: "pencil", doing: "Editing", done: "Edited", target: file ?? "files" };
  }
  if (dataType === "readFile" || /^read /i.test(title)) {
    const named = file ?? (title.replace(/^read /i, "") || undefined);
    return { verb: "read", glyph: "file", doing: "Reading", done: "Read", target: named ?? "file" };
  }
  if (dataType === "commandExecution" || data.command) {
    const command = text(data.command) ?? (title || undefined);
    const exploratory = command ? classifyExploratoryCommand(command) : null;
    if (exploratory) {
      return { ...exploratory, command };
    }
    return { verb: "run", glyph: "terminal", doing: "Running", done: "Ran", target: command ?? "command", command };
  }
  if (dataType === "webSearch") {
    return { verb: "search", glyph: "globe", doing: "Searching the web", done: "Searched the web", target: title || undefined };
  }
  return { verb: "tool", glyph: "wrench", doing: "Using a tool", done: "Used a tool", target: title || undefined };
}

/* ── Worker delegation items ─────────────────────────────────────────────
   Four different provider events land as `delegation` items and the renderer
   used to sniff them apart with inline `"key" in data` checks. Naming the
   facets once means the fold below and the card that draws them can never
   disagree about what a row is. */

export type DelegationFacet = "spawn" | "result" | "blocked" | "rejected" | "steered";

export function delegationFacet(item: ConversationItem): DelegationFacet {
  if ("childBlocked" in item.data) return "blocked";
  if ("willRetry" in item.data) return "rejected";
  if ("steeredBy" in item.data) return "steered";
  if ("delivered" in item.data) return "result";
  return "spawn";
}

export function delegationChildSessionId(item: ConversationItem): string | undefined {
  return typeof item.data.childSessionId === "string" ? item.data.childSessionId : undefined;
}

/**
 * Collapse each worker's result onto the panel that spawned it.
 *
 * The spawn row is a live panel while the worker runs, so letting the result
 * arrive as its own row further down left the user with two cards for one
 * worker: a stale live one and a disconnected outcome. One worker is one place
 * in the transcript, from "delegated" through to "done".
 *
 * Applied to the merged durable+live list rather than inside either projection,
 * because a spawn read from the forest and a result still only in the live
 * stream is the normal case mid-run.
 */
export function foldWorkerDelegations(items: ConversationItem[]): ConversationItem[] {
  const panelByChild = new Map<string, ConversationItem>();
  const folded: ConversationItem[] = [];
  for (const item of items) {
    if (item.type !== "delegation") { folded.push(item); continue; }
    const childSessionId = delegationChildSessionId(item);
    const facet = delegationFacet(item);
    if (!childSessionId) { folded.push(item); continue; }
    if (facet === "spawn") {
      // Copied because the merge below mutates the row that is already in the
      // output list, and the caller's item must not change underneath it.
      const panel = { ...item, data: { ...item.data } };
      panelByChild.set(childSessionId, panel);
      folded.push(panel);
      continue;
    }
    const panel = facet === "result" ? panelByChild.get(childSessionId) : undefined;
    // An orphan result — durable history truncated, or a branch switched away
    // from the spawn — still has to render. Folding must never lose a row.
    if (!panel) { folded.push(item); continue; }
    panel.data = { ...panel.data, ...item.data };
    panel.status = item.status ?? panel.status;
    panel.title = item.title ?? panel.title;
    // A `[worker result] …` stamp is routing metadata for the panel, not a
    // replacement for the human objective the spawn already showed.
    if (item.text && !workerResultSummary(item.text)) panel.text = item.text;
    // The panel keeps its own key and eventId: the key is what React reconciles
    // on, and the eventId is what the durable/live dedupe upstream matches.
  }
  return folded;
}

/**
 * Image attachments persisted on a user turn, as renderable data URIs.
 *
 * The backend stamps `data.attachments = [{mediaType, dataUri}]` onto the
 * user's message event so the conversation can re-render what was sent after
 * a reload. Malformed payloads return [] — a bad attachment must never be
 * able to break the transcript row it rides on.
 */
export function attachmentUris(data: Record<string, unknown>): string[] {
  const attachments = data.attachments;
  if (!Array.isArray(attachments)) return [];
  return attachments.flatMap((attachment) => {
    const dataUri = (attachment as { dataUri?: unknown } | null)?.dataUri;
    return typeof dataUri === "string" && dataUri.startsWith("data:image/") ? [dataUri] : [];
  });
}
