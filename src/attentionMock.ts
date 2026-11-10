// The attention ledger without a backend: what `bun run dev` and component
// tests read. Same rules as `bridge_core::attention` — a new row supersedes
// the chat's older unseen rows, reads skip superseded rows, and seen is
// per chat — so a mock-mode screen behaves like the real one.
import type { AttentionItem, AttentionKind, ListAttentionParams, ListAttentionResult, MarkAttentionSeenResult, UnreadSummary } from "./protocol/generated/protocol";

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

/** How strongly a kind claims a chat's row, as in core's rollup. */
const STRENGTH: Partial<Record<AttentionKind, number>> = { needsYou: 5, escalation: 4, posted: 3, failed: 2, finished: 1 };

/**
 * The unread summary core would answer. `parentOf` gives a live session's
 * parent: `null` for a top-level chat, `undefined` for one that is gone or
 * archived. A session is unread while its newest row is unseen and
 * unanswered, and rolls up to the top-level chat above it.
 */
export function mockUnreadSummary(parentOf: (sessionId: string) => string | null | undefined): UnreadSummary {
  const newest = new Map<string, StoredItem>();
  for (const row of rows) if (row.sessionId) newest.set(row.sessionId, row);
  const byChat: Record<string, AttentionKind> = {};
  const bySession: Record<string, AttentionKind> = {};
  for (const [sessionId, row] of newest) {
    const strength = STRENGTH[row.kind];
    if (row.seenAt || row.resolvedAt || !strength || parentOf(sessionId) === undefined) continue;
    bySession[sessionId] = row.kind;
    let current = sessionId;
    let root: string | undefined;
    const visited = new Set<string>();
    while (!visited.has(current)) {
      visited.add(current);
      const parent = parentOf(current);
      if (parent === undefined) break;
      if (parent === null) { root = current; break; }
      current = parent;
    }
    if (root && strength > (STRENGTH[byChat[root]] ?? 0)) byChat[root] = row.kind;
  }
  const chats = Object.values(byChat);
  return {
    total: chats.length,
    needsYou: chats.filter(kind => kind === "needsYou" || kind === "escalation").length,
    byChat,
    bySession,
  };
}

export function onMockAttentionChanged(handler: () => void): () => void {
  listeners.add(handler);
  return () => { listeners.delete(handler); };
}

export function resetMockAttention() {
  rows = [];
  nextSequence = 1;
}
