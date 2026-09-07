/**
 * Loading the golden streams.
 *
 * Separate from `golden.test.ts` so the jsdom render test can use the same
 * fixtures without duplicating the hydration, and so neither test has to know
 * that a fixture is JSON on disk.
 */

import acpOther from "./fixtures/acp-other.json";
import claude from "./fixtures/claude.json";
import codex from "./fixtures/codex.json";
import cursor from "./fixtures/cursor.json";
import opencode from "./fixtures/opencode.json";
import { asWireKind, readWireKind } from "./wire";
import { normalizeAgentEvent } from "./codec";
import { reduceTranscript } from "./reducer";
import type { ConversationItem } from "./item";
import type { AgentEvent, SessionEntry } from "../types";

export const HARNESSES = ["claude", "codex", "cursor", "opencode"] as const;
export type GoldenHarness = (typeof HARNESSES)[number];

interface RawFixtureEvent {
  id: number;
  sequence: number;
  kind: string;
  itemId?: string;
  role?: string;
  status?: string;
  title?: string;
  text?: string;
  data?: Record<string, unknown>;
}

const STREAMS: Record<GoldenHarness, unknown> = { claude, codex, cursor, opencode };

/** One harness's turn, as `bridgeApi.onAgentEvent` would deliver it. */
export function harnessStream(harness: GoldenHarness): AgentEvent[] {
  return fixtureStream(harness, STREAMS[harness] as RawFixtureEvent[]);
}

export function acpOtherStream(): AgentEvent[] {
  return fixtureStream("cursor", acpOther);
}

function fixtureStream(harness: GoldenHarness, frames: RawFixtureEvent[]): AgentEvent[] {
  return frames.map(raw => ({
    id: raw.id,
    sessionId: harness,
    sequence: raw.sequence,
    protocolVersion: 1,
    kind: asWireKind(raw.kind),
    itemId: raw.itemId ?? null,
    role: raw.role ?? null,
    status: raw.status ?? null,
    title: raw.title ?? null,
    text: raw.text ?? null,
    data: raw.data ?? {},
    providerMeta: { adapter: harness },
    createdAt: "2026-09-05T10:00:00Z",
  }));
}

export function reduceHarness(harness: GoldenHarness): ConversationItem[] {
  return reduceTranscript(harnessStream(harness).map(normalizeAgentEvent));
}

/**
 * The same turn, as the durable writer would have stored it.
 *
 * Mirrors `store::session_event_in_transaction`: only frames with
 * `sequence > 0` are persisted (transient deltas, progress and turn markers
 * carry `sequence: 0` and are never written to the forest); a
 * `message.completed` is rewritten to `user.message`/`assistant.message` by
 * role; every other kind is stored unchanged. The payload carries the raw
 * event's `itemId`, `role`, `status`, `title`, `text` and `data`, exactly the
 * fields `normalizeSessionEntry` reads back out.
 */
export function durableEntries(harness: GoldenHarness): SessionEntry[] {
  return durableEntriesFrom(harness, harnessStream(harness));
}

/**
 * The same rule over any live stream, so a test that generates its frames
 * rather than loading them can still ask what the forest would hold.
 */
export function durableEntriesFrom(sessionId: string, events: AgentEvent[]): SessionEntry[] {
  const entries: SessionEntry[] = [];
  let parentEntryId: string | null = null;
  for (const event of events) {
    if (event.sequence <= 0) continue;
    const id = `${sessionId}-e${event.sequence}`;
    const wireKind = readWireKind(event.kind);
    const kind = wireKind === "message.completed"
      ? (event.role === "user" ? "user.message" : "assistant.message")
      : wireKind;
    entries.push({
      id,
      sessionId,
      parentEntryId,
      sequence: event.sequence,
      semanticSchemaVersion: 2,
      kind,
      payload: {
        itemId: event.itemId ?? null,
        role: event.role ?? null,
        status: event.status ?? null,
        title: event.title ?? null,
        text: event.text ?? null,
        data: event.data ?? {},
      },
      providerEventId: null,
      contextVisibility: "eligible",
      tokenEstimate: null,
      createdAt: event.createdAt,
    });
    parentEntryId = id;
  }
  return entries;
}
