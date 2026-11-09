// The attention ledger without a backend: what `bun run dev` and component
// tests read. Same rules as `bridge_core::attention` — a new row supersedes
// the chat's older unseen rows, reads skip superseded rows, and seen is
// per chat — so a mock-mode screen behaves like the real one.
import type { AttentionItem, ListAttentionParams, ListAttentionResult, MarkAttentionSeenResult } from "./protocol/generated/protocol";

type StoredItem = AttentionItem & { superseded: boolean };

let rows: StoredItem[] = [];
let nextSequence = 1;
const listeners = new Set<() => void>();

function changed() {
  listeners.forEach(listener => listener());
}

export type MockAttentionInput = Pick<AttentionItem, "kind" | "sessionId" | "title"> & Partial<Pick<AttentionItem, "body" | "approvalEventId" | "severity">>;

/** Append a row the way core would, and announce it. */
export function pushMockAttention(input: MockAttentionInput): AttentionItem {
  const sequence = nextSequence++;
  for (const row of rows) {
    if (row.sessionId === input.sessionId && !row.seenAt && !row.superseded) row.superseded = true;
  }
  const item: StoredItem = {
    sequence,
    id: `mock-attention-${sequence}`,
    kind: input.kind,
    severity: input.severity ?? (input.kind === "finished" ? "informational" : "actionable"),
    sessionId: input.sessionId,
    approvalEventId: input.approvalEventId ?? null,
    title: input.title,
    body: input.body ?? null,
    createdAt: new Date().toISOString(),
    resolvedAt: null,
    seenAt: null,
    superseded: false,
  };
  rows.push(item);
  changed();
  return publicItem(item);
}

/** Answer a mock request, as an approval resolution would in core. */
export function resolveMockAttention(sequence: number) {
  const row = rows.find(item => item.sequence === sequence);
  if (row && !row.resolvedAt) {
    row.resolvedAt = new Date().toISOString();
    changed();
  }
}

function publicItem({ superseded: _superseded, ...item }: StoredItem): AttentionItem {
  return structuredClone(item);
}

export function mockListAttention(params: ListAttentionParams = {}): ListAttentionResult {
  const after = params.afterSequence ?? 0;
  const limit = Math.min(Math.max(params.limit ?? 100, 1), 500);
  const items = rows
    .filter(row => row.sequence > after && !row.superseded && (!params.unseenOnly || !row.seenAt))
    .slice(0, limit)
    .map(publicItem);
  return { items, latestSequence: rows.at(-1)?.sequence ?? 0 };
}

export function mockMarkAttentionSeen(sessionId: string): MarkAttentionSeenResult {
  const at = new Date().toISOString();
  let updated = 0;
  for (const row of rows) {
    if (row.sessionId === sessionId && !row.seenAt) {
      row.seenAt = at;
      updated += 1;
    }
  }
  if (updated) changed();
  return { updated };
}

export function onMockAttentionChanged(handler: () => void): () => void {
  listeners.add(handler);
  return () => { listeners.delete(handler); };
}

export function resetMockAttention() {
  rows = [];
  nextSequence = 1;
}
