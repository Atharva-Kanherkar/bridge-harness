import { readWireKind } from "./transcript/wire";
import type { AgentEvent } from "./types";

export interface StreamSample {
  eventId: string;
  frameId: string;
  sessionId: string;
  kind: string;
  dbWaitMs: number;
  normalizationMs: number;
  persistenceMs: number;
  receiptToPublicationMs: number;
  webviewReceivedAt: number;
  receiptToCommitMs?: number;
  receiptToPaintProxyMs?: number;
}
const LIMIT = 512;
const samples = new Map<string, StreamSample>();
const eventSamples = new WeakMap<AgentEvent, string>();

/** No text, prompts, tool arguments or credentials. Native timestamps are
 * durations, never subtracted from the independent webview clock. */
export function recordStreamReceipt(event: AgentEvent, now = performance.now()): void {
  const timing = event.providerMeta.bridgeStreamTiming;
  if (!timing || typeof timing !== "object") return;
  const t = timing as Record<string, unknown>;
  if (typeof t.eventId !== "string" || typeof t.frameId !== "string") return;
  const fields = ["dbWaitMs", "normalizationMs", "persistenceMs", "receiptToPublicationMs"] as const;
  if (fields.some(field => typeof t[field] !== "number" || !Number.isFinite(t[field]) || (t[field] as number) < 0)) return;
  samples.set(t.eventId, {
    eventId: t.eventId, frameId: t.frameId, sessionId: event.sessionId, kind: readWireKind(event.kind),
    dbWaitMs: t.dbWaitMs as number, normalizationMs: t.normalizationMs as number,
    persistenceMs: t.persistenceMs as number, receiptToPublicationMs: t.receiptToPublicationMs as number,
    webviewReceivedAt: now,
  });
  eventSamples.set(event, t.eventId);
  while (samples.size > LIMIT) samples.delete(samples.keys().next().value!);
}

/** Coalesced events retain the newest frame's providerMeta. */
function sampleFor(event: AgentEvent): StreamSample | undefined {
  const timing = event.providerMeta.bridgeStreamTiming as { eventId?: string } | undefined;
  const id = eventSamples.get(event) ?? timing?.eventId;
  return id ? samples.get(id) : undefined;
}
export function recordStreamCommit(events: AgentEvent[], now = performance.now()): string[] {
  const committed: string[] = [];
  for (const event of events) {
    const sample = sampleFor(event);
    if (!sample || sample.receiptToCommitMs !== undefined) continue;
    sample.receiptToCommitMs = Math.max(0, now - sample.webviewReceivedAt);
    committed.push(sample.eventId);
  }
  return committed;
}
export function recordStreamPaintProxy(ids: string[], now = performance.now()): void {
  for (const id of ids) {
    const sample = samples.get(id);
    if (sample && sample.receiptToPaintProxyMs === undefined) sample.receiptToPaintProxyMs = Math.max(0, now - sample.webviewReceivedAt);
  }
}
export function streamTimingSnapshot(): StreamSample[] { return [...samples.values()].map(sample => ({ ...sample })); }
export function clearStreamTiming(): void { samples.clear(); }

// Inspector-only, opt-in samples arrive only when the native process enables
// BRIDGE_STREAM_TIMING. Bounded memory, no console flood or telemetry writes.
if (typeof window !== "undefined") {
  Object.assign(window, { bridgeStreamTiming: { snapshot: streamTimingSnapshot, clear: clearStreamTiming } });
}
