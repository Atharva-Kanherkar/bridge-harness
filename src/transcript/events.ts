/**
 * The transcript event union — the one shape the conversation reducer reads.
 *
 * Below this module the world is per-provider: Codex speaks ACP items, Claude
 * speaks `tool_use` blocks, OpenCode speaks parts with a nested `state`, Cursor
 * speaks ACP session updates, and the durable forest speaks entries. Above it
 * there is one closed union, and a reducer with no provider branches in it.
 *
 * Two rules keep it closed:
 *
 * 1. Every semantic field a reducer or a card reads is a named property on its
 *    variant. Nothing is inferred from a bag at reduce time.
 * 2. The untyped remainder of a provider payload travels in exactly one field,
 *    `providerData`, which nothing under `src/transcript/` may branch on. It
 *    exists because `ConversationItem.data` is still read by the cards for
 *    detail (`childBlocked`, `divergence`, `actions`, `attachments`, …);
 *    typing those out is a separate change and is recorded as a divergence in
 *    `testing/feat-issue-488-normalized-events.md`.
 */

import type { ToolCallDisplay } from "./toolCall";

/** Where a frame reached the UI from. */
export type TranscriptOrigin = "live" | "durable";

/** Which surface a tool call draws on: a plain activity row, or a diff card. */
export type ToolSurface = "activity" | "diff";

/** Who spoke. Narrower than the wire's open `role` string. */
export type MessageRole = "user" | "assistant" | "system" | "tool";

/** The two request/answer interactions that share one fold. */
export type InteractionKind = "permission" | "question";

/** Which half of the compaction lifecycle a card reports. */
export type CompactionPhase = "requested" | "completed" | "failed";

/**
 * What every transcript event carries, whatever it says.
 *
 * `eventId` is the number an interaction resolves by: live it is the session
 * event's own id, durably it is the forest entry's sequence. They are the same
 * number for one frame, which is why an approval raised live can be answered
 * after a reload.
 */
export interface TranscriptEnvelope {
  sessionId: string;
  /** Ordering key. Durable frames carry the forest sequence; transient live
   *  frames arrive as 0 and are anchored by `orderTranscript`. */
  sequence: number;
  eventId: number;
  /** Provider item id, when the provider sent one. */
  itemId?: string;
  /** Forest entry id, for replayed frames. */
  entryId?: string;
  createdAt?: string;
  origin: TranscriptOrigin;
  /**
   * The reducer's map key for the item this event creates or updates, when the
   * frame has a stable identity of its own. Absent for a transient frame that
   * has to borrow the turn's fallback key.
   */
  key?: string;
  /** First-arrival anchor preserved through live coalescing. */
  causalAnchor?: number;
  /**
   * Which runtime produced this frame, as the backend stamped it on
   * `providerMeta.adapter` (live) or inside the stored payload (durable).
   *
   * A harness id, not a label — nothing under `src/transcript/` interprets it.
   * It exists because a row outlives the session's current harness: after a
   * chat is switched from Codex to OpenCode, the Codex failures already in the
   * transcript are still Codex failures, and a card that reads the session's
   * harness instead relabels history.
   */
  adapter?: string;
  /** The untyped remainder. Never branched on inside `src/transcript/`. */
  providerData: Record<string, unknown>;
}

interface Framed {
  envelope: TranscriptEnvelope;
}

/* ── Prose ─────────────────────────────────────────────────────────────── */

export interface MessageDelta extends Framed {
  type: "message.delta";
  /** Absent when the provider did not say; the reducer keeps what it had. */
  role?: MessageRole;
  text: string;
}

export interface MessageCompleted extends Framed {
  type: "message.completed";
  role?: MessageRole;
  text: string;
  title?: string;
  status?: string;
}

/* ── Thinking ──────────────────────────────────────────────────────────── */

export interface ThinkingStarted extends Framed {
  type: "thinking.started";
  /** The streamed thought, when the provider sent one. */
  text: string;
  /** Codex sends the thought only as a summary; used when `text` is empty. */
  summary: string;
  status: string;
}

export interface ThinkingDelta extends Framed {
  type: "thinking.delta";
  text: string;
}

export interface ThinkingCompleted extends Framed {
  type: "thinking.completed";
  /** The streamed thought, when the provider sent one. */
  text: string;
  /** Codex sends the thought only as a summary; used when `text` is empty. */
  summary: string;
  title?: string;
  status: string;
}

/* ── Tool calls ────────────────────────────────────────────────────────── */

export interface ToolCallStarted extends Framed {
  type: "tool.started";
  surface: ToolSurface;
  title?: string;
  text: string;
  status?: string;
  role?: MessageRole;
  tool: ToolCallDisplay;
}

/** A progress frame: streamed command output, a streamed patch, an ACP update. */
export interface ToolCallProgress extends Framed {
  type: "tool.progress";
  surface: ToolSurface;
  title?: string;
  /** Appended to the item's body. Snapshots arrive as `tool.completed`. */
  outputDelta: string;
  status?: string;
  tool: ToolCallDisplay;
}

export interface ToolCallCompleted extends Framed {
  type: "tool.completed";
  surface: ToolSurface;
  title?: string;
  text: string;
  status?: string;
  role?: MessageRole;
  tool: ToolCallDisplay;
}

/* ── Plan ──────────────────────────────────────────────────────────────── */

export interface PlanUpdated extends Framed {
  type: "plan.updated";
  title: string;
  text: string;
  status?: string;
}

/* ── Approvals and interactions ────────────────────────────────────────── */

export interface ApprovalRequested extends Framed {
  type: "approval.requested";
  title: string;
  text: string;
  status: string;
}

export interface ApprovalResolved extends Framed {
  type: "approval.resolved";
  /** The `eventId` of the request this answers. */
  requestEventId?: number;
  decision?: string;
  status?: string;
  /** The frame as the card keeps it, under `data.resolution`. */
  resolution: Record<string, unknown>;
}

export interface InteractionRequested extends Framed {
  type: "interaction.requested";
  interaction: InteractionKind;
  title: string;
  text: string;
  status: string;
}

export interface InteractionSettled extends Framed {
  type: "interaction.settled";
  interaction: InteractionKind;
  requestEventId?: number;
  decision?: string;
  status?: string;
  /** The frame as the card wants to keep it, under `data.resolution`. */
  resolution: Record<string, unknown>;
}

/* ── Delegation ────────────────────────────────────────────────────────── */

export interface DelegationUpdated extends Framed {
  type: "delegation.updated";
  title?: string;
  text: string;
  status?: string;
}

/* ── Artifacts and history ─────────────────────────────────────────────── */

export interface ArtifactReady extends Framed {
  type: "artifact.ready";
  title?: string;
  text: string;
  status?: string;
}

export interface CheckpointRecorded extends Framed {
  type: "checkpoint";
  title: string;
  text: string;
  status?: string;
}

export interface CompactionReported extends Framed {
  type: "compaction";
  phase: CompactionPhase;
  title: string;
  text: string;
  status?: string;
}

/**
 * The harness compacted its own context window.
 *
 * Its own variant rather than a `CompactionReported` phase, because it is a
 * different fact with a different owner: Bridge records this, it does not cause
 * it, and it must not touch the maintenance fold that a Bridge checkpoint
 * request opens. `preTokens` and `postTokens` are present only when the
 * provider reported them, so a card can say the window shrank by a number only
 * when a number was actually sent. See `docs/compaction-and-resume.md`.
 */
export interface ContextCompacted extends Framed {
  type: "context.compacted";
  harness?: string;
  trigger?: string;
  preTokens?: number;
  postTokens?: number;
  title: string;
  text: string;
  status?: string;
}

export interface BranchSummary extends Framed {
  type: "branch.summary";
  title: string;
  text: string;
  status?: string;
}

export interface TranscriptError extends Framed {
  type: "error";
  title?: string;
  text: string;
  status: string;
}

/* ── Notices, turns, lifecycle ─────────────────────────────────────────── */

/**
 * A frame Bridge knows about that has no card of its own: a handoff brief, a
 * stale base warning, a settled ACP approval. It draws as an activity row.
 * Distinct from `unknown`, which is a kind nothing here has a name for.
 * (A model switch used to land here too; it has its own card now, below.)
 */
export interface TranscriptNotice extends Framed {
  type: "notice";
  title?: string;
  text: string;
  status?: string;
  role?: MessageRole;
}

/**
 * A model/harness switch: the milestone divider that reports what the next
 * provider inherits. Normalized to its own type so grouping and rendering key
 * off `type`, never off a payload field.
 */
export interface ModelChanged extends Framed {
  type: "model.change";
  title?: string;
  text: string;
  status?: string;
}

export interface TurnStarted extends Framed {
  type: "turn.started";
}

export interface TurnCompleted extends Framed {
  type: "turn.completed";
  status?: string;
}

/** A `session.*` frame. `settles` marks the ones that end streaming thinking. */
export interface SessionLifecycle extends Framed {
  type: "session.lifecycle";
  settles: boolean;
}

export interface UsageReported extends Framed {
  type: "usage";
}

/* ── Raw and unknown ───────────────────────────────────────────────────── */

/** A provider frame kept for inspection: collapsed, never conversation. */
export interface RawProviderFrame extends Framed {
  type: "raw";
  title: string;
  text: string;
  /**
   * Whether this frame is worth drawing. A replayed frame is: a reviewer can
   * expand it. A live one is not: it has no stable identity to reconcile
   * against the durable twin the forest will hold a moment later, so drawing it
   * would double every raw row for the length of a turn.
   */
  inspectable: boolean;
}

/**
 * A kind outside every family this build knows. Reported in dev, rendered as a
 * collapsed raw card in production — never folded into generic activity, which
 * is how a new provider event used to disappear into a wrench row.
 */
export interface UnknownEvent extends Framed {
  type: "unknown";
  wireKind: string;
  raw: Record<string, unknown>;
}

export type TranscriptEvent =
  | MessageDelta
  | MessageCompleted
  | ThinkingStarted
  | ThinkingDelta
  | ThinkingCompleted
  | ToolCallStarted
  | ToolCallProgress
  | ToolCallCompleted
  | PlanUpdated
  | ApprovalRequested
  | ApprovalResolved
  | InteractionRequested
  | InteractionSettled
  | DelegationUpdated
  | ArtifactReady
  | CheckpointRecorded
  | CompactionReported
  | ContextCompacted
  | BranchSummary
  | TranscriptError
  | TranscriptNotice
  | ModelChanged
  | TurnStarted
  | TurnCompleted
  | SessionLifecycle
  | UsageReported
  | RawProviderFrame
  | UnknownEvent;

export type TranscriptEventType = TranscriptEvent["type"];

/**
 * Causal order for a mixed window.
 *
 * Transient frames — deltas, turn markers, progress — are published with
 * sequence 0: only frames the forest persisted carry a real one. Sorting on
 * that zero pulled every streamed bubble above every durably sequenced card, so
 * a turn read as "all prose, then all tools" instead of the order it happened.
 * Arrival order is commit order, so a transient frame belongs just after the
 * last persisted frame before it. Coalescing moves a merged delta to the tail
 * for eviction, so its first-arrival anchor travels separately.
 */
export function orderTranscript(events: TranscriptEvent[]): TranscriptEvent[] {
  // The live window opens wherever the subscription started, not at the forest
  // root: frames ahead of the first persisted one still belong just before it,
  // never before history that was already durable. With nothing persisted yet
  // there is no anchor at all, so the window sorts after everything until the
  // first durable frame arrives and re-anchors it.
  const firstDurable = events.find(event => event.envelope.sequence > 0)?.envelope.sequence;
  let lastDurable = (firstDurable ?? Number.MAX_SAFE_INTEGER) - 1;
  return events
    .map(event => {
      const { envelope } = event;
      if (envelope.sequence > 0) {
        lastDurable = envelope.sequence;
        return event;
      }
      const anchor = envelope.causalAnchor ?? lastDurable;
      return { ...event, envelope: { ...envelope, sequence: anchor + 0.5 } } as TranscriptEvent;
    })
    .sort((left, right) => left.envelope.sequence - right.envelope.sequence);
}
