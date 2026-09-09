/**
 * The one place the wire is read.
 *
 * Two doors in, one union out: `normalizeAgentEvent` for the live stream and
 * `normalizeSessionEntry` for a durable forest entry. Everything a provider did
 * differently — where Claude hangs a tool name, where Codex hangs an item type,
 * where OpenCode nests a `state`, where the forest wraps a payload in an
 * envelope — is decided here, once, at ingestion. Nothing above this module
 * reads a wire kind, and nothing above it knows a harness exists.
 *
 * The kind vocabulary is owned by the Rust normalizers
 * (`bridge-core/src/agent.rs`, `acp_events.rs`) and the durable writer
 * (`store::session_event_in_transaction`, which rewrites `message.completed`
 * into `user.message`/`assistant.message` and refuses to persist deltas,
 * `*.progress`, `turn.*`, `usage.updated`, `plan.updated` and
 * `question.settled`). Families are matched by prefix, so a new member of a
 * family the Rust side owns lands on the right card instead of falling through.
 * A kind outside every family is reported, never guessed at.
 */

import type { AgentEvent, SessionEntry } from "../types";
import { readWireKind } from "./wire";
import { readToolCall, type ToolCallDisplay } from "./toolCall";
import { harnessLabel } from "../utils";
import type {
  MessageRole,
  ToolSurface,
  TranscriptEnvelope,
  TranscriptEvent,
} from "./events";

/* ── Small readers ─────────────────────────────────────────────────────── */

function stringValue(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

function objectValue(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : {};
}

function numberValue(value: unknown): number | undefined {
  if (typeof value === "number") return Number.isFinite(value) ? value : undefined;
  if (typeof value === "string" && value.trim()) {
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : undefined;
  }
  return undefined;
}

const ROLES: ReadonlySet<string> = new Set(["user", "assistant", "system", "tool"]);

function messageRole(value: unknown, fallback: MessageRole = "assistant"): MessageRole {
  const role = stringValue(value);
  return role && ROLES.has(role) ? role as MessageRole : fallback;
}

function stringList(value: unknown): string {
  return Array.isArray(value) ? value.join("\n") : "";
}

/** Resolved reasoning text: the streamed `text`, or Codex's summary-only payload. */
export function reasoningDisplayText(text?: string | null, data: Record<string, unknown> = {}): string {
  if (text) return text;
  const summary = data.summary;
  if (typeof summary === "string") return summary;
  return stringList(summary);
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

function humanizeKind(kind: string): string {
  return kind.replace(/[._-]+/g, " ").replace(/^\w/, (letter) => letter.toUpperCase());
}

/** Pull a human-readable error string from an error payload, tolerant of provider shapes. */
function errorText(payload: Record<string, unknown>): string {
  const direct = stringValue(payload.text);
  if (direct) return direct;
  const data = objectValue(payload.data);
  const error = objectValue(data.error);
  return stringValue(error.message) ?? stringValue(data.message) ?? stringValue(data.reason) ?? "";
}

/* ── The vocabulary ────────────────────────────────────────────────────── */

/**
 * The kind every harness's own compaction boundary arrives as.
 *
 * Mirrors `NATIVE_COMPACTION_KIND` in `bridge-core/src/agent.rs`. This is the
 * only kind the "Context compacted" card is drawn from: a Bridge checkpoint on
 * a hot session frees no provider tokens, so it says "Checkpoint saved"
 * instead. See `docs/compaction-and-resume.md`.
 */
const NATIVE_COMPACTION_KIND = "context.compacted";

/** Families the Rust side owns. A new member of one of these is not unknown. */
const KNOWN_PREFIXES = [
  "message.", "reasoning.", "tool.", "command.", "file_change.", "diff.",
  "plan.", "approval.", "permission.", "question.", "delegation.", "artifact.",
  "compaction.", "turn.", "session.", "worker.", "workspace.", "provider.",
  "raw.", "model.", "mode.", "config.", "commands.", "todo.", "extension.",
  "usage.", "branch.", "handoff.", "runtime.", "effort.", "checkpoint.",
  "context.",
  // Codex's fallback family: `normalize_item` in `bridge-core/src/agent.rs`
  // (around line 757) maps any Codex item type outside its named set — a
  // `readFile`, for instance — to `item.started`/`item.completed`. Live, not
  // legacy: it is how an unrecognized Codex item still lands on a tool card.
  "item.",
] as const;

/** Standalone kinds with no dot to match on. */
const KNOWN_EXACT: ReadonlySet<string> = new Set([
  "error", "checkpoint", "compaction", "reasoning",
  "user.message", "assistant.message",
]);

function isKnownKind(kind: string): boolean {
  return KNOWN_EXACT.has(kind) || KNOWN_PREFIXES.some(prefix => kind.startsWith(prefix));
}

const TOOL_PREFIXES = ["tool.", "command.", "file_change.", "diff.", "item."] as const;

function isToolKind(kind: string): boolean {
  return TOOL_PREFIXES.some(prefix => kind.startsWith(prefix));
}

/**
 * Which surface a tool-shaped kind draws on.
 *
 * Codex, Claude and OpenCode all get their edits normalized to `file_change.*`
 * upstream, so the kind alone says "diff". ACP does not: Cursor sends every
 * call as `tool.*` and names the category on the payload, which is why an ACP
 * patch used to render as a plain tool row with the diff hidden inside it.
 */
function surfaceFor(kind: string, data: Record<string, unknown>): ToolSurface {
  if (kind.startsWith("file_change.") || kind.startsWith("diff.")) return "diff";
  return data.kind === "edit" ? "diff" : "activity";
}

/**
 * A kind nothing here has a name for.
 *
 * Reported in development rather than swallowed: a provider that starts sending
 * a new frame should show up as a loud line in the console and a visible,
 * inspectable card in the transcript, not as one more anonymous wrench row.
 */
function inDevelopment(): boolean {
  // Narrowed locally rather than by pulling in `vite/client`: this is the only
  // module that asks, and it asks for exactly one boolean.
  return (import.meta as ImportMeta & { env?: { DEV?: boolean } }).env?.DEV === true;
}

/**
 * Kinds already reported this process, keyed `${origin}:${kind}`.
 *
 * `reduceConversation` re-normalizes the whole live window on every flush, so
 * without this an unknown kind would log on every one of those instead of
 * once when it first shows up.
 */
const reportedUnknownKinds = new Set<string>();

function reportUnknown(kind: string, origin: string): void {
  if (!inDevelopment()) return;
  const key = `${origin}:${kind}`;
  if (reportedUnknownKinds.has(key)) return;
  reportedUnknownKinds.add(key);
  // eslint-disable-next-line no-console
  console.error(`[transcript] unmapped ${origin} event kind: ${kind}`);
}

/* ── Live events ───────────────────────────────────────────────────────── */

/**
 * One live provider frame, normalized.
 *
 * Total by construction: every kind lands on a variant, and anything outside
 * the vocabulary lands on `unknown`.
 */
export function normalizeAgentEvent(raw: AgentEvent): TranscriptEvent {
  const kind = readWireKind(raw.kind);
  const data = raw.data;
  const itemId = raw.itemId ?? undefined;
  const text = raw.text ?? "";
  const title = raw.title ?? undefined;
  const status = raw.status ?? undefined;
  const envelope: TranscriptEnvelope = {
    sessionId: raw.sessionId,
    sequence: raw.sequence,
    eventId: raw.id,
    itemId,
    createdAt: raw.createdAt,
    origin: "live",
    key: liveKey(kind, itemId, raw.id),
    causalAnchor: raw.causalAnchor,
    providerData: data,
  };
  const tool = (surface: ToolSurface): ToolCallDisplay =>
    readToolCall({ title, text, status, surface, data });

  if (kind === "message.delta") {
    return { type: "message.delta", envelope, role: raw.role ? messageRole(raw.role) : undefined, text };
  }
  if (kind === "reasoning.delta") {
    return { type: "thinking.delta", envelope, text };
  }
  if (kind === "reasoning.started") {
    return { type: "thinking.started", envelope, text, summary: reasoningDisplayText(null, data), status: status ?? "streaming" };
  }
  if (kind === "reasoning" || kind.startsWith("reasoning.")) {
    return { type: "thinking.completed", envelope, text, summary: reasoningDisplayText(null, data), title, status: status ?? "completed" };
  }
  if (kind === "message.completed" || kind.startsWith("message.")) {
    return { type: "message.completed", envelope, role: raw.role ? messageRole(raw.role) : undefined, text, title, status };
  }
  if (kind === "tool.progress" || kind.endsWith(".output_delta") || kind === "diff.delta") {
    const surface = surfaceFor(kind, data);
    return { type: "tool.progress", envelope, surface, title, outputDelta: text, status, tool: tool(surface) };
  }
  if (kind.startsWith("plan.")) {
    return { type: "plan.updated", envelope, title: title ?? "Plan", text, status };
  }
  if (kind.startsWith("delegation.") || kind === "worker.result") {
    return { type: "delegation.updated", envelope, title, text, status };
  }
  if (kind === "approval.requested") {
    return { type: "approval.requested", envelope, title: title ?? "Approval required", text, status: "pending" };
  }
  if (kind === "approval.resolved") {
    return {
      type: "approval.resolved",
      envelope,
      requestEventId: numberValue(data.requestEventId),
      decision: stringValue(data.decision),
      status,
      resolution: { ...raw } as unknown as Record<string, unknown>,
    };
  }
  const interaction = interactionKind(kind);
  if (interaction) {
    if (kind.endsWith(".requested")) {
      return {
        type: "interaction.requested",
        envelope,
        interaction,
        title: title ?? (interaction === "permission" ? "Permission required" : "Question"),
        text,
        status: status ?? "pending",
      };
    }
    return {
      type: "interaction.settled",
      envelope,
      interaction,
      requestEventId: numberValue(data.requestEventId),
      decision: stringValue(data.decision),
      status,
      resolution: { ...raw } as unknown as Record<string, unknown>,
    };
  }
  if (kind.startsWith("artifact.")) {
    return { type: "artifact.ready", envelope, title, text, status };
  }
  if (kind === "checkpoint") {
    return { type: "checkpoint", envelope, title: "Checkpoint", text: text || stringValue(data.summary) || "", status };
  }
  if (kind === NATIVE_COMPACTION_KIND) {
    return contextCompactedEvent(envelope, data, status);
  }
  if (kind === "compaction" || kind.startsWith("compaction.")) {
    return compactionEvent(kind, envelope, { ...data, text, status }, text, status);
  }
  if (kind === "branch.summary") {
    return { type: "branch.summary", envelope, title: "Branch summary", text: text || stringValue(data.summary) || "", status };
  }
  if (kind === "error" || kind === "runtime.failed") {
    return { type: "error", envelope, title, text, status: status ?? "failed" };
  }
  if (kind === "turn.started") return { type: "turn.started", envelope };
  if (kind.startsWith("turn.")) return { type: "turn.completed", envelope, status };
  if (kind === "usage.updated") return { type: "usage", envelope };
  if (kind === "session.model_changed") {
    // The switch milestone is its own row, live and replayed alike: the
    // backend publishes this event on the bus at commit time, so the divider
    // appears immediately rather than only after a reload.
    return { type: "model.change", envelope, title, text, status };
  }
  if (kind.startsWith("session.")) {
    return { type: "session.lifecycle", envelope, settles: kind === "session.idle" };
  }
  if (kind.startsWith("provider.") || kind.startsWith("raw.")) {
    // Live raw frames have no stable identity to reconcile against the durable
    // twin the forest already holds, so they are carried but not drawn. Replay
    // is where a reviewer inspects them.
    return { type: "raw", envelope, title: title ?? "Raw provider event", text, inspectable: false };
  }
  if (isToolKind(kind)) {
    const surface = surfaceFor(kind, data);
    const started = kind.endsWith(".started");
    return started
      ? { type: "tool.started", envelope, surface, title, text, status, role: raw.role ? messageRole(raw.role) : undefined, tool: tool(surface) }
      : { type: "tool.completed", envelope, surface, title, text, status, role: raw.role ? messageRole(raw.role) : undefined, tool: tool(surface) };
  }
  if (isKnownKind(kind)) {
    // A family Bridge owns with no card of its own: a stale-base warning, a
    // settled ACP approval, a mode switch. `user.message`/`assistant.message`
    // only reach the live channel as replayed frames; the forest projection is
    // where a stored turn becomes a bubble.
    return { type: "notice", envelope, title, text, status, role: raw.role ? messageRole(raw.role) : undefined };
  }
  reportUnknown(kind, "live");
  return { type: "unknown", envelope, wireKind: kind, raw: { ...data } };
}

/**
 * The row a live frame belongs to.
 *
 * An approval, a permission and a question are keyed by their own event id
 * rather than by the item id they carry: providers name the interaction after
 * the call it is about — Claude sends `permission.requested` under the tool's
 * own id — and keying on that would merge the question into the command,
 * leaving the reader a tool row with buttons on it and no way to answer.
 *
 * Everything else prefers the provider's item id, so a lifecycle's two halves
 * meet. The synthetic fallback keeps the wire kind, so two halves that carry no
 * item id stay two rows, exactly as they always have.
 */
function liveKey(kind: string, itemId: string | undefined, eventId: number): string | undefined {
  if (kind === "approval.requested" || kind === "approval.resolved") return `approval:${eventId}`;
  const interaction = interactionKind(kind);
  if (interaction) return `${interaction}:${eventId}`;
  if (itemId) return itemId;
  if (kind.startsWith("plan.")) return "current-plan";
  // Reasoning and prose without an item id borrow the turn's key, which only
  // the reducer knows.
  if (kind.startsWith("reasoning.") || kind === "reasoning" || kind.startsWith("message.")) return undefined;
  return `${kind}:${eventId}`;
}

function interactionKind(kind: string): "permission" | "question" | undefined {
  if (kind.startsWith("permission.")) return "permission";
  if (kind.startsWith("question.")) return "question";
  return undefined;
}

/**
 * The harness's own compaction boundary.
 *
 * The card names a token figure only when the provider sent one. Codex and
 * OpenCode report a boundary with no numbers at all, and inventing a zero
 * would read as "the context shrank to nothing".
 */
function contextCompactedEvent(
  envelope: TranscriptEnvelope,
  data: Record<string, unknown>,
  status: string | undefined,
): TranscriptEvent {
  const harness = stringValue(data.harness);
  const preTokens = numberValue(data.preTokens);
  const postTokens = numberValue(data.postTokens);
  return {
    type: "context.compacted",
    envelope,
    harness,
    trigger: stringValue(data.trigger),
    preTokens,
    postTokens,
    title: "Context compacted",
    text: contextCompactedText(harness, preTokens, postTokens),
    status: status ?? "completed",
  };
}

function contextCompactedText(
  harness: string | undefined,
  preTokens: number | undefined,
  postTokens: number | undefined,
): string {
  const who = harness ? harnessLabel(harness) : "The harness";
  if (preTokens !== undefined && postTokens !== undefined) {
    return `${who} summarised its context, ${tokens(preTokens)} down to ${tokens(postTokens)}.`;
  }
  if (preTokens !== undefined) {
    return `${who} summarised its context at ${tokens(preTokens)}.`;
  }
  return `${who} summarised its own context.`;
}

/** A token count at the precision a reader can hold in their head. */
function tokens(value: number): string {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M tokens`;
  if (value >= 1_000) return `${Math.round(value / 1_000)}k tokens`;
  return `${value} tokens`;
}

function compactionEvent(
  kind: string,
  envelope: TranscriptEnvelope,
  payload: Record<string, unknown>,
  text: string,
  status: string | undefined,
): TranscriptEvent {
  if (kind === "compaction.requested") {
    return {
      type: "compaction",
      envelope,
      phase: "requested",
      title: "Compaction requested",
      text: compactionReasonLabel(stringValue(payload.reason)),
      status,
    };
  }
  if (kind === "compaction.failed") {
    return {
      type: "compaction",
      envelope,
      phase: "failed",
      title: "Compaction failed",
      // New entries carry safe, classified copy. The raw reason stays only as a
      // compatibility fallback for transcripts written by older builds.
      text: stringValue(payload.message) ?? stringValue(payload.reason) ?? "",
      status: "failed",
    };
  }
  // A committed Bridge boundary. It is not "Context compacted": committing a
  // checkpoint writes forest entries and moves the session head without
  // touching the adapter, so on a hot session the provider's context is
  // exactly as full as it was. What it did do is save a summary the next cold
  // start or model handoff will read. The harness's own boundary is a
  // different entry with a different card.
  return {
    type: "compaction",
    envelope,
    phase: "completed",
    title: "Checkpoint saved",
    text: stringValue(payload.summary) ?? text,
    status,
  };
}

/* ── Durable entries ───────────────────────────────────────────────────── */

/**
 * One immutable forest entry, re-encoded into the same union.
 *
 * Returns `null` for an entry that is not conversation at all — the session
 * lifecycle plumbing that used to replay as collapsed "used tools" groups
 * before and around the user's messages, but only after a reload, which is
 * exactly what made it read as a glitch.
 */
export function normalizeSessionEntry(entry: SessionEntry): TranscriptEvent | null {
  if (entry.semanticSchemaVersion < 1 || entry.semanticSchemaVersion > 2) {
    throw new Error(`Unsupported semantic event schema version ${entry.semanticSchemaVersion} on entry ${entry.id}`);
  }
  const kind = entry.kind;
  const payload = entry.payload;
  const nested = objectValue(payload.data);
  const itemId = stringValue(payload.itemId);
  const status = stringValue(payload.status);
  const flat = { ...payload, ...nested };
  const envelope: TranscriptEnvelope = {
    sessionId: entry.sessionId,
    sequence: entry.sequence,
    eventId: entry.sequence,
    itemId,
    entryId: entry.id,
    createdAt: entry.createdAt,
    origin: "durable",
    key: `entry:${entry.id}`,
    // The stored wrapper is flattened here so a replayed row renders like the
    // live one it is a twin of. Card-shaped entries keep their envelope below.
    providerData: flat,
  };
  const raw = entry.kind.startsWith("provider.")
    || entry.kind.startsWith("raw.")
    || entry.contextVisibility.toLowerCase().includes("raw");
  if (raw) {
    return {
      type: "raw",
      envelope: { ...envelope, providerData: payload },
      title: stringValue(payload.title) ?? "Raw provider event",
      text: stringValue(payload.text) ?? "",
      inspectable: true,
    };
  }
  const carded = { ...envelope, providerData: payload };
  const text = stringValue(payload.text) ?? "";

  if (kind === "user.message" || kind === "assistant.message") {
    return {
      type: "message.completed",
      envelope: carded,
      role: kind === "user.message" ? "user" : messageRole(payload.role),
      text,
      status,
    };
  }
  if (kind === "checkpoint") {
    return { type: "checkpoint", envelope: carded, title: "Checkpoint", text: stringValue(payload.summary) ?? "", status };
  }
  if (kind === NATIVE_COMPACTION_KIND) {
    return contextCompactedEvent(carded, nested, status);
  }
  if (kind === "compaction" || kind.startsWith("compaction.")) {
    return compactionEvent(kind, carded, payload, text, status);
  }
  if (kind === "branch.summary") {
    return { type: "branch.summary", envelope: carded, title: "Branch summary", text: stringValue(payload.summary) ?? "", status };
  }
  if (kind === "error" || kind === "runtime.failed") {
    return {
      type: "error",
      envelope: carded,
      title: stringValue(payload.title) ?? "Agent error",
      text: errorText(payload),
      status: status ?? "failed",
    };
  }
  if (kind === "reasoning" || kind.startsWith("reasoning.")) {
    if (kind === "reasoning.delta") {
      return { type: "thinking.delta", envelope: carded, text };
    }
    if (kind === "reasoning.started") {
      return { type: "thinking.started", envelope: carded, text, summary: reasoningDisplayText(null, payload), status: status ?? "streaming" };
    }
    return {
      type: "thinking.completed",
      envelope: carded,
      text,
      summary: reasoningDisplayText(null, payload),
      title: stringValue(payload.title) ?? "Thought for a moment",
      status: status ?? "completed",
    };
  }

  // Everything below reads the flattened wrapper: the inner event data (tool
  // input, command, output…) wins, so a replayed row renders like a live one.
  const title = stringValue(payload.title) ?? humanizeKind(kind);
  // Sparse tool updates must preserve the start title when the reducer joins them.
  const toolTitle = stringValue(payload.title);
  const body = stringValue(payload.text) ?? stringValue(payload.summary) ?? stringValue(payload.reason) ?? "";
  const role = payload.role ? messageRole(payload.role) : undefined;
  const tool = (surface: ToolSurface): ToolCallDisplay =>
    readToolCall({ title: toolTitle, text: body, status, surface, data: flat });

  if (kind === "approval.requested") {
    return { type: "approval.requested", envelope, title, text: body, status: status ?? "pending" };
  }
  if (kind === "approval.resolved") {
    return {
      type: "approval.resolved",
      envelope,
      requestEventId: numberValue(payload.requestEventId ?? nested.requestEventId),
      decision: stringValue(payload.decision) ?? stringValue(nested.decision),
      status: stringValue(payload.status),
      resolution: payload,
    };
  }
  const interaction = interactionKind(kind);
  if (interaction) {
    if (kind.endsWith(".requested")) {
      return { type: "interaction.requested", envelope, interaction, title, text: body, status: status ?? "pending" };
    }
    return {
      type: "interaction.settled",
      envelope,
      interaction,
      requestEventId: numberValue(payload.requestEventId ?? nested.requestEventId),
      decision: stringValue(nested.decision),
      status: stringValue(payload.status) ?? stringValue(nested.status),
      resolution: payload,
    };
  }
  if (kind === "worker.result") {
    // Canonical results also exist after restart, without a live delivery
    // event. Give them the same result facet so they settle the spawn panel.
    return {
      type: "delegation.updated",
      envelope: { ...envelope, providerData: { ...flat, delivered: false } },
      title: "Worker result",
      text: stringValue(payload.summary) ?? body,
      status,
    };
  }
  if (kind.startsWith("delegation.")) {
    return { type: "delegation.updated", envelope, title, text: body, status };
  }
  if (kind.startsWith("artifact.")) {
    return { type: "artifact.ready", envelope, title, text: body, status };
  }
  if (kind === "plan.updated" || kind.startsWith("plan.")) {
    return { type: "plan.updated", envelope, title, text: body, status };
  }
  if (kind === "turn.started") return { type: "turn.started", envelope };
  if (kind.startsWith("turn.")) return { type: "turn.completed", envelope, status };
  if (kind === "usage.updated") return { type: "usage", envelope };
  if (kind === "session.model_changed") {
    // The switch milestone is its own row: grouping and rendering key off the
    // normalized type, never off a payload field.
    return { type: "model.change", envelope, title, text: body, status };
  }
  if (kind.startsWith("session.") && kind !== "session.model_changed") {
    // Deliberately narrower than the live filter: every other `session.*`
    // frame is lifecycle plumbing with no row of its own.
    return null;
  }
  if (isToolKind(kind)) {
    const surface = surfaceFor(kind, flat);
    const started = kind.endsWith(".started");
    return started
      ? { type: "tool.started", envelope, surface, title: toolTitle, text: body, status, role, tool: tool(surface) }
      : { type: "tool.completed", envelope, surface, title: toolTitle, text: body, status, role, tool: tool(surface) };
  }
  if (isKnownKind(kind)) {
    return { type: "notice", envelope, title, text: body, status, role };
  }
  reportUnknown(kind, "durable");
  return { type: "unknown", envelope, wireKind: kind, raw: flat };
}
