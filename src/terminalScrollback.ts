// Terminal scrollback shared across TerminalPane mounts. Previously a
// process-lifetime map holding up to 1 MB for every workspace that ever
// streamed, with no global cap and no eviction; it is now an LRU with a total
// budget, so the newest shells keep their history and old workspaces stop
// costing memory forever.

export const SCROLLBACK_PER_SESSION_LIMIT = 1_000_000;
export const SCROLLBACK_TOTAL_BUDGET = 4_000_000;

// Insertion order doubles as recency order: every append re-inserts its key.
const scrollback = new Map<string, string>();

export function scrollbackFor(sessionId: string): string | undefined {
  return scrollback.get(sessionId);
}

export function rememberScrollback(sessionId: string, data: string): void {
  const buffered = `${scrollback.get(sessionId) ?? ""}${data}`.slice(
    -SCROLLBACK_PER_SESSION_LIMIT,
  );
  scrollback.delete(sessionId);
  scrollback.set(sessionId, buffered);
  let total = 0;
  for (const value of scrollback.values()) total += value.length;
  for (const key of scrollback.keys()) {
    if (total <= SCROLLBACK_TOTAL_BUDGET || key === sessionId) break;
    total -= scrollback.get(key)?.length ?? 0;
    scrollback.delete(key);
  }
}

export function clearScrollbackForTests(): void {
  scrollback.clear();
}
