import type { SessionForestSnapshot } from "./types";

/** How many of a chat's newest entries an opening page carries, and how many a
 * snapshot too large for the cache keeps. Enough to fill the first screen and
 * the idle slices after it; small enough to arrive at once. */
export const FIRST_PAGE_ENTRIES = 160;

/** Snapshots that hold only the newest entries of a longer branch. The full
 * window replaces them as soon as it arrives. */
const partialForests = new WeakSet<SessionForestSnapshot>();

export function markPartialForest(value: SessionForestSnapshot): SessionForestSnapshot {
  partialForests.add(value);
  return value;
}

export function isPartialForest(value: SessionForestSnapshot | undefined): boolean {
  return !!value && partialForests.has(value);
}

/** A page snapshot that already holds the whole branch is a complete snapshot.
 * A daemon that predates `limit` returns the full window to a page request;
 * that is complete too, as far as this client can make it. */
export function pageIsComplete(page: SessionForestSnapshot, requested = FIRST_PAGE_ENTRIES): boolean {
  const window = page.entryWindow as SessionForestSnapshot["entryWindow"] | undefined;
  if (!window) return true;
  return window.returned >= window.total || window.returned > requested;
}

/** Approximate retained size in UTF-16 bytes, from the shape alone. Walking
 * nodes reads each string's length without copying it, so a 20 MB snapshot
 * costs one pass over its few hundred thousand nodes rather than building a
 * 20 MB string. `JSON.stringify` on every refresh is what this replaces. */
export function estimateForestBytes(value: unknown): number {
  let bytes = 0;
  const stack: unknown[] = [value];
  while (stack.length) {
    const node = stack.pop();
    if (typeof node === "string") bytes += node.length * 2 + 2;
    else if (typeof node === "number" || typeof node === "boolean") bytes += 8;
    else if (Array.isArray(node)) { bytes += 2; for (const item of node) stack.push(item); }
    else if (node && typeof node === "object") {
      for (const [key, item] of Object.entries(node)) { bytes += key.length * 2 + 4; stack.push(item); }
    }
  }
  return bytes;
}

export interface CachedForest {
  value: SessionForestSnapshot;
  /** The forest digest the value was read under, when the caller knew it. */
  digest?: string;
  /** Only the newest entries: show it, then fetch the full window. */
  partial: boolean;
}

/** A cache is a convenience, not history. Evicted chats reload from SQLite.
 * Count and size limits include tool data, images and metadata, which a
 * text-only budget misses. Sizes are estimates, not RSS.
 *
 * A chat too large to keep whole keeps its newest page instead. Long chats are
 * the ones that open slowly, and they used to be exactly the ones this cache
 * refused, so every reopen of a long chat started from nothing. */
export class ForestCache {
  private entries = new Map<string, CachedForest & { bytes: number }>();
  private bytes = 0;
  // Room for the chats a reader moves between plus the ones the sidebar warmed
  // on hover. Bytes are UTF-16 estimates of the snapshot, not heap size.
  constructor(private maxEntries = 8, private maxBytes = 48 * 1024 * 1024, private pageEntries = FIRST_PAGE_ENTRIES) {}
  get(id: string) {
    return this.peek(id)?.value;
  }
  peek(id: string): CachedForest | undefined {
    const entry = this.entries.get(id);
    if (!entry) return undefined;
    this.entries.delete(id);
    this.entries.set(id, entry);
    return { value: entry.value, digest: entry.digest, partial: entry.partial };
  }
  set(id: string, value: SessionForestSnapshot, digest?: string) {
    const old = this.entries.get(id);
    if (old) { this.bytes -= old.bytes; this.entries.delete(id); }
    let stored = value;
    let partial = isPartialForest(value);
    let bytes = estimateForestBytes(value);
    // The newest page of a chat too large to keep whole.
    if (bytes > this.maxBytes && value.entries.length > this.pageEntries) {
      stored = markPartialForest({ ...value, entries: value.entries.slice(-this.pageEntries) });
      partial = true;
      bytes = estimateForestBytes(stored);
    }
    if (bytes > this.maxBytes) return;
    this.entries.set(id, { value: stored, digest: partial ? undefined : digest, partial, bytes });
    this.bytes += bytes;
    while (this.entries.size > this.maxEntries || this.bytes > this.maxBytes) {
      const oldest = this.entries.keys().next().value!;
      this.bytes -= this.entries.get(oldest)!.bytes;
      this.entries.delete(oldest);
    }
  }
  has(id: string) { return this.entries.has(id); }
  get size() { return this.entries.size; }
  get retainedBytes() { return this.bytes; }
}
