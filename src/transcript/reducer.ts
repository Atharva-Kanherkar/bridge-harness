/**
 * The one conversation reducer: typed events in, conversation items out.
 *
 * There used to be two of these — a live one branching on kind prefixes and a
 * durable one branching on entry kinds — and they disagreed, quietly, about
 * what a replayed patch was, whether a worker result was a delegation, and
 * which reasoning card a summary belonged to. A reader watching a turn stream
 * and the same reader after a reload were looking at two different programs.
 *
 * This is pure: state in, state out, no clock, no I/O, no provider names, no
 * wire kinds. Both projections run it, so a divergence between live and replay
 * has to be a divergence in the codec, where it is one table to read.
 */

import { stripBridgeFences } from "../humanize";
import { orderTranscript, type TranscriptEvent } from "./events";
import { itemIdentity, type ConversationItem, type ConversationItemType } from "./item";
import { readToolCall } from "./toolCall";

/**
 * Defense in depth for a backend-tagged maintenance frame. Content shape is
 * deliberately irrelevant: a user may legitimately ask for the checkpoint
 * schema, while a malformed maintenance reply or refusal is still internal.
 */
export function isInternalCompactionEnvelope(
  _text: string,
  data: Record<string, unknown> = {},
): boolean {
  return data.bridgeInternalOrigin === "compaction";
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

/** Working state. One object so every step is visibly a fold over it. */
interface Fold {
  items: Map<string, ConversationItem>;
  /** Requests an answer can name, keyed `<scope>:<requestEventId>`. */
  requests: Map<string, ConversationItem>;
  /** Rows a later lifecycle half can join, keyed by provider item id. */
  byItemId: Map<string, ConversationItem>;
  /** Assistant prose emitted inside a compaction maintenance window. */
  internal: Set<ConversationItem>;
  compacting: boolean;
  turnIndex: number;
}

export function reduceTranscript(events: TranscriptEvent[]): ConversationItem[] {
  const fold: Fold = {
    items: new Map(),
    requests: new Map(),
    byItemId: new Map(),
    internal: new Set(),
    compacting: false,
    turnIndex: 0,
  };

  for (const event of orderTranscript(events)) {
    applyEvent(fold, event);
  }

  const reduced = [...fold.items.values()]
    .filter(item => item.type !== "message" || !fold.internal.has(item))
    .map(item => item.type === "message"
      ? { ...item, text: stripBridgeFences(stripWorkerResultBlocks(item.text)) }
      : item)
    .filter(item => item.type !== "reasoning" || item.text.trim().length > 0)
    .filter(item => item.type !== "message" || item.text.trim().length > 0);
  // Stable, so items that share a sequence keep the order they were folded in.
  reduced.sort((left, right) => left.sequence - right.sequence);
  return reduced.map(withIdentity);
}

function applyEvent(fold: Fold, event: TranscriptEvent): void {
  const envelope = event.envelope;

  // The compaction maintenance window. A checkpoint round trip is Bridge
  // talking to the model on the user's behalf; its prose is not conversation.
  if (event.type === "compaction" && event.phase === "requested") fold.compacting = true;
  const internalProse = (event.type === "message.delta" || event.type === "message.completed")
    && (fold.compacting || isInternalCompactionEnvelope("", envelope.providerData));
  if (event.type === "compaction" && event.phase !== "requested") fold.compacting = false;

  switch (event.type) {
    case "usage":
      return;
    case "session.lifecycle":
      if (event.settles) settleThinking(fold);
      return;
    case "turn.started":
      fold.turnIndex += 1;
      return;
    case "turn.completed":
      settleThinking(fold);
      return;

    case "message.delta": {
      if (!event.text) return;
      const key = envelope.key ?? `message:live:${fold.turnIndex}`;
      const item = fold.items.get(key) ?? place(fold, {
        key, type: "message", eventId: envelope.eventId, role: event.role,
        status: "streaming", text: "", data: {}, sequence: envelope.sequence,
        entryId: envelope.entryId,
      });
      item.text += event.text;
      item.status = "streaming";
      item.eventId = envelope.eventId;
      if (internalProse) fold.internal.add(item);
      return;
    }

    case "message.completed": {
      const key = envelope.key ?? `message:live:${fold.turnIndex}`;
      const held = fold.items.get(key);
      // A provider that only names a message when it finishes leaves the
      // streamed chunks under a turn-scoped key. Adopting them keeps one bubble
      // instead of a ghost above the real reply.
      const adopted = held ? undefined : adoptStreamingAssistant(fold, event.role, event.text);
      const target = held ?? adopted;
      if (!event.text && !target) return;
      const item = target ?? place(fold, {
        key, type: "message", eventId: envelope.eventId, role: event.role,
        status: event.status, title: event.title, text: "", data: {},
        sequence: envelope.sequence, entryId: envelope.entryId,
      });
      item.eventId = envelope.eventId;
      item.status = event.status ?? item.status;
      item.title = event.title ?? item.title;
      item.role = event.role ?? item.role;
      if (event.text) item.text = event.text;
      item.data = { ...item.data, ...envelope.providerData };
      if (adopted && envelope.key && item.key !== envelope.key) rekey(fold, item, envelope.key);
      if (envelope.itemId) item.itemId = envelope.itemId;
      if (internalProse) fold.internal.add(item);
      return;
    }

    case "thinking.delta": {
      if (!event.text) return;
      const key = envelope.key ?? `reasoning:live:${fold.turnIndex}`;
      const item = fold.items.get(key) ?? place(fold, {
        key, type: "reasoning", eventId: envelope.eventId, status: "streaming",
        text: "", data: {}, sequence: envelope.sequence, entryId: envelope.entryId,
      });
      item.text += event.text;
      item.status = "streaming";
      item.eventId = envelope.eventId;
      if (envelope.itemId) index(fold, item, envelope.itemId, true);
      return;
    }

    case "thinking.started":
    case "thinking.completed": {
      const key = envelope.key ?? `reasoning:live:${fold.turnIndex}`;
      const held = fold.items.get(key) ?? joined(fold, envelope.itemId);
      const adopted = held ? undefined : lastStreamingThinking(fold);
      const target = held ?? adopted;
      if (target) {
        target.status = event.status;
        if (event.text) target.text = event.text;
        else if (!target.text && event.summary) target.text = event.summary;
        if (event.type === "thinking.completed" && event.title) target.title = event.title;
        target.data = { ...target.data, ...envelope.providerData };
        target.eventId = envelope.eventId;
        if (adopted && envelope.key && target.key !== envelope.key) rekey(fold, target, envelope.key);
        if (envelope.itemId) index(fold, target, envelope.itemId, envelope.key === envelope.itemId);
        return;
      }
      const text = event.text || event.summary;
      if (!text) return;
      const item = place(fold, {
        key, type: "reasoning", eventId: envelope.eventId, status: event.status,
        title: event.type === "thinking.completed" ? event.title : undefined,
        text, data: { ...envelope.providerData }, sequence: envelope.sequence,
        entryId: envelope.entryId,
      });
      if (envelope.itemId) index(fold, item, envelope.itemId, envelope.key === envelope.itemId);
      return;
    }

    case "tool.started":
    case "tool.completed": {
      const type: ConversationItemType = event.surface === "diff" ? "diff" : "activity";
      const item = upsert(fold, event, type, {}, true);
      item.status = event.status ?? item.status;
      item.title = event.title ?? item.title;
      item.role = event.role ?? item.role;
      if (event.text) item.text = event.text;
      item.data = { ...item.data, ...envelope.providerData };
      restat(item);
      return;
    }

    case "tool.progress": {
      const type: ConversationItemType = event.surface === "diff" ? "diff" : "activity";
      const item = upsert(fold, event, type, { status: event.status ?? "inProgress", title: event.title }, true);
      item.text += event.outputDelta;
      item.status = event.status ?? item.status;
      item.data = { ...item.data, ...envelope.providerData };
      restat(item);
      return;
    }

    case "plan.updated": {
      const key = envelope.key ?? "current-plan";
      const held = fold.items.get(key);
      const item = held ?? place(fold, {
        key, type: "plan", eventId: envelope.eventId, text: "", data: {},
        sequence: envelope.sequence, entryId: envelope.entryId,
      });
      item.eventId = envelope.eventId;
      item.status = event.status;
      item.title = event.title;
      item.text = event.text;
      // A plan is a snapshot, not an accumulation: the newest one is the plan.
      item.data = envelope.providerData;
      return;
    }

    case "approval.requested": {
      const item = upsert(fold, event, "approval", { status: event.status, title: event.title });
      item.status = event.status;
      item.title = event.title;
      item.text = event.text;
      item.data = { ...item.data, ...envelope.providerData };
      fold.requests.set(`approval:${envelope.eventId}`, item);
      return;
    }

    case "approval.resolved": {
      const target = event.requestEventId === undefined
        ? undefined
        : fold.requests.get(`approval:${event.requestEventId}`);
      // An answer whose request is not on this branch, or has already scrolled
      // out of the live window, has nothing to say on its own.
      if (!target) return;
      target.status = event.decision ?? event.status ?? "resolved";
      target.data = { ...target.data, resolution: event.resolution };
      return;
    }

    case "interaction.requested": {
      const item = upsert(fold, event, event.interaction, { status: event.status, title: event.title });
      item.status = event.status;
      item.title = event.title;
      item.text = event.text;
      item.data = { ...item.data, ...envelope.providerData };
      fold.requests.set(`${event.interaction}:${envelope.eventId}`, item);
      return;
    }

    case "interaction.settled": {
      const target = event.requestEventId === undefined
        ? undefined
        : fold.requests.get(`${event.interaction}:${event.requestEventId}`);
      if (!target) return;
      target.status = event.status ?? event.decision ?? "resolved";
      target.data = { ...target.data, ...envelope.providerData, resolution: event.resolution };
      return;
    }

    case "delegation.updated": {
      const item = upsert(fold, event, "delegation", { role: "system", title: event.title, status: event.status }, true);
      item.role = "system";
      item.status = event.status ?? item.status;
      item.title = event.title ?? item.title;
      if (event.text) item.text = event.text;
      item.data = { ...item.data, ...envelope.providerData };
      return;
    }

    case "artifact.ready": {
      const item = upsert(fold, event, "artifact", { title: event.title, status: event.status });
      item.status = event.status ?? item.status;
      item.title = event.title ?? item.title;
      if (event.text) item.text = event.text;
      item.data = { ...item.data, ...envelope.providerData };
      return;
    }

    case "checkpoint":
    case "compaction":
    case "branch.summary":
    case "error":
    case "notice": {
      const type: ConversationItemType = event.type === "checkpoint" ? "checkpoint"
        : event.type === "compaction" ? "compaction"
        : event.type === "branch.summary" ? "branch-summary"
        : event.type === "error" ? "error"
        : "activity";
      const item = upsert(fold, event, type, { title: event.title, status: event.status });
      item.status = event.status ?? item.status;
      item.title = event.title || item.title;
      item.role = event.type === "notice" ? event.role ?? item.role : item.role;
      if (event.text) item.text = event.text;
      item.data = { ...item.data, ...envelope.providerData };
      return;
    }

    case "raw": {
      if (!event.inspectable) return;
      const item = upsert(fold, event, "raw", { title: event.title });
      item.title = event.title;
      item.text = event.text;
      item.data = { ...envelope.providerData, collapsed: true, inspectable: true };
      return;
    }

    case "unknown": {
      // Loud rather than lost. A kind nothing here has a name for used to fold
      // into a generic wrench row, which made a new provider frame look like an
      // anonymous tool call. It gets the raw surface instead: collapsed, but
      // present and inspectable.
      const item = upsert(fold, event, "raw", { title: `Unrecognized event: ${event.wireKind}` });
      item.title = `Unrecognized event: ${event.wireKind}`;
      item.data = { ...event.raw, wireKind: event.wireKind, collapsed: true, inspectable: true };
      return;
    }
  }
}

/* ── Fold helpers ──────────────────────────────────────────────────────── */

function place(fold: Fold, item: ConversationItem): ConversationItem {
  fold.items.set(item.key, item);
  return item;
}

/**
 * Register a row under its provider item id, so a later lifecycle half finds
 * it, and stamp the id when the row is keyed by it.
 *
 * The stamp is conditional because an approval, a permission and a question
 * carry the item id of the call they are about, not an identity of their own:
 * stamping it would give the permission row the same identity as the command
 * it guards, and the live/durable merge would then treat one as a duplicate of
 * the other.
 */
function index(fold: Fold, item: ConversationItem, itemId: string, ownsId: boolean): void {
  if (ownsId) item.itemId = itemId;
  fold.byItemId.set(itemId, item);
}

function joined(fold: Fold, itemId: string | undefined): ConversationItem | undefined {
  return itemId ? fold.byItemId.get(itemId) : undefined;
}

function rekey(fold: Fold, item: ConversationItem, key: string): void {
  fold.items.delete(item.key);
  item.key = key;
  fold.items.set(key, item);
}

/**
 * The row this frame belongs to: the one under its own key, the one an earlier
 * lifecycle half registered under the same provider item id, or a new one.
 *
 * The two lookups are what makes a replayed `tool.started`/`tool.completed`
 * pair one row: they are separate forest entries with separate entry keys, and
 * only the item id says they are the same call.
 */
function upsert(
  fold: Fold,
  event: TranscriptEvent,
  type: ConversationItemType,
  seed: Partial<ConversationItem> = {},
  join = false,
): ConversationItem {
  const envelope = event.envelope;
  const key = envelope.key ?? `${type}:${envelope.eventId}`;
  const held = fold.items.get(key) ?? (join ? joined(fold, envelope.itemId) : undefined);
  const item = held ?? place(fold, {
    key, type, eventId: envelope.eventId, text: "", data: {},
    sequence: envelope.sequence, entryId: envelope.entryId,
    createdAt: envelope.createdAt, ...seed,
  });
  item.eventId = envelope.eventId;
  // A row keyed by its own item id owns that id; a row that merely mentions one
  // (an approval about a tool call) does not.
  if (envelope.itemId && envelope.key === envelope.itemId) item.itemId = envelope.itemId;
  if (envelope.itemId && join) index(fold, item, envelope.itemId, envelope.key === envelope.itemId);
  return item;
}

/**
 * Re-read the tool facet from the row as it now stands, not from one frame.
 *
 * A single frame is not enough: a command's arguments arrive on the start and
 * its exit code on the completion, and ACP does not repeat the tool category on
 * the update at all. The surface is the row's own, for the same reason.
 */
function restat(item: ConversationItem): void {
  item.tool = readToolCall({
    title: item.title,
    text: item.text,
    status: item.status,
    surface: item.type === "diff" ? "diff" : "activity",
    data: item.data,
  });
}

function settleThinking(fold: Fold): void {
  // Both spellings are recognized defensively: the codec defaults a
  // `thinking.started` frame to "streaming", but a provider or an older
  // durable entry may still label it "inProgress".
  for (const item of fold.items.values()) {
    if (item.type === "reasoning" && (item.status === "streaming" || item.status === "inProgress")) item.status = "completed";
  }
}

function lastStreamingThinking(fold: Fold): ConversationItem | undefined {
  for (const candidate of [...fold.items.values()].reverse()) {
    if (candidate.type === "reasoning" && candidate.status === "streaming") return candidate;
  }
  return undefined;
}

function adoptStreamingAssistant(
  fold: Fold,
  role: string | undefined,
  text: string,
): ConversationItem | undefined {
  if (role === "user") return undefined;
  const completed = text.trim();
  if (!completed) return undefined;
  for (const candidate of [...fold.items.values()].reverse()) {
    if (candidate.type !== "message" || candidate.role === "user" || candidate.status !== "streaming") continue;
    const streamed = candidate.text.trim();
    if (streamed && (completed === streamed || completed.startsWith(streamed))) return candidate;
  }
  return undefined;
}

function withIdentity(item: ConversationItem): ConversationItem {
  const itemId = item.itemId ?? (typeof item.data.itemId === "string" ? item.data.itemId : undefined);
  const next = itemId && item.itemId !== itemId ? { ...item, itemId } : item;
  return { ...next, identity: itemIdentity(next) };
}
