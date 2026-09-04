/**
 * Loading the golden streams.
 *
 * Separate from `golden.test.ts` so the jsdom render test can use the same
 * fixtures without duplicating the hydration, and so neither test has to know
 * that a fixture is JSON on disk.
 */

import claude from "./fixtures/claude.json";
import codex from "./fixtures/codex.json";
import cursor from "./fixtures/cursor.json";
import opencode from "./fixtures/opencode.json";
import { asWireKind } from "./wire";
import { normalizeAgentEvent } from "./codec";
import { reduceTranscript } from "./reducer";
import type { ConversationItem } from "./item";
import type { AgentEvent } from "../types";

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
  return (STREAMS[harness] as RawFixtureEvent[]).map(raw => ({
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
