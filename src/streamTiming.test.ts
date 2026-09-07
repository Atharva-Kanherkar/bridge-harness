import { beforeEach, expect, it } from "vitest";
import { appendAgentEventBatch } from "./agentEvents";
import { clearStreamTiming, recordStreamCommit, recordStreamPaintProxy, recordStreamReceipt, streamTimingSnapshot } from "./streamTiming";
import type { AgentEvent } from "./types";
import { asWireKind } from "./transcript/wire";
const event = (id: string): AgentEvent => ({ id: 0, sequence: 0, protocolVersion: 1, sessionId: "s", kind: asWireKind("message.delta"),
  itemId: "m", text: "private text", role: null, status: null, title: null, data: {}, createdAt: "now",
  providerMeta: { bridgeStreamTiming: { frameId: id, eventId: id, dbWaitMs: 2, normalizationMs: 1, persistenceMs: 3, receiptToPublicationMs: 8 } },
});
beforeEach(clearStreamTiming);
it("correlates independent native and webview durations without retaining content", () => {
  const frame = event("a");
  recordStreamReceipt(frame, 100);
  const ids = recordStreamCommit([frame], 106);
  recordStreamPaintProxy(ids, 132);
  expect(streamTimingSnapshot()[0]).toMatchObject({ eventId: "a", receiptToCommitMs: 6, receiptToPaintProxyMs: 32, receiptToPublicationMs: 8 });
  expect(JSON.stringify(streamTimingSnapshot())).not.toContain("private text");
  expect(recordStreamCommit([frame], 200)).toEqual([]);
});
it("records the newest sample after coalescing and bounds retained samples", () => {
  const a = event("a"), b = event("b");
  recordStreamReceipt(a, 1); recordStreamReceipt(b, 2);
  expect(recordStreamCommit(appendAgentEventBatch([a], [b]), 5)).toEqual(["b"]);
  for (let i = 0; i < 600; i++) recordStreamReceipt(event(String(i)), i);
  expect(streamTimingSnapshot()).toHaveLength(512);
  expect(streamTimingSnapshot()[0].eventId).toBe("88");
});
it("ignores missing or malformed timing metadata", () => {
  const frame = event("a");
  frame.providerMeta = {};
  recordStreamReceipt(frame, 1);
  frame.providerMeta = { bridgeStreamTiming: { eventId: "a", frameId: "a", dbWaitMs: -1 } };
  recordStreamReceipt(frame, 1);
  expect(streamTimingSnapshot()).toEqual([]);
});
