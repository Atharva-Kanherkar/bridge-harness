import { projectSessionConversation, type ConversationItem } from "../conversation";
import type { SessionEntry } from "../types";

/**
 * `projectSessionConversation` is a full replay of the active branch, and the
 * app asks the same question over and over: the pending-send reconciliation
 * re-projects the whole forest on every streaming flush, the send watermark
 * projects it again, and a cold open projects it once for the "has this chat
 * started" check. On a 1,500-entry branch that is tens of milliseconds of
 * repeated work per delta.
 *
 * The forest snapshot is immutable and `mergeForestSnapshot` preserves array
 * identity when nothing changed, so the entries array plus the active leaf is a
 * sound key. This wrapper memoizes the projection without changing the pure
 * function's contract.
 *
 * The caches are deliberately small: a session's snapshot is the only entry
 * array the UI re-reads, and a leaf changes only when the branch does. Both are
 * LRU-bounded so an app left open for days cannot pin every forest it has seen.
 */
const MAX_ENTRY_ARRAYS = 8;
const MAX_LEAVES_PER_ARRAY = 4;

interface LeafVariant {
  leaf: string | null;
  items: ConversationItem[];
}

interface EntryVariants {
  /** Most recently used first. */
  variants: LeafVariant[];
}

const cache = new Map<SessionEntry[], EntryVariants>();

export function projectSessionConversationCached(
  entries: SessionEntry[],
  activeLeafId: string | null,
): ConversationItem[] {
  const leaf = activeLeafId ?? null;
  const cached = cache.get(entries);
  if (cached) {
    const index = cached.variants.findIndex(variant => variant.leaf === leaf);
    if (index !== -1) {
      // Refresh both levels so the eviction order tracks real use.
      const [hit] = cached.variants.splice(index, 1);
      cached.variants.unshift(hit);
      cache.delete(entries);
      cache.set(entries, cached);
      return hit.items;
    }
    const items = projectSessionConversation(entries, leaf);
    cached.variants.unshift({ leaf, items });
    if (cached.variants.length > MAX_LEAVES_PER_ARRAY) cached.variants.pop();
    cache.delete(entries);
    cache.set(entries, cached);
    return items;
  }

  const items = projectSessionConversation(entries, leaf);
  cache.set(entries, { variants: [{ leaf, items }] });
  if (cache.size > MAX_ENTRY_ARRAYS) {
    const oldest = cache.keys().next();
    if (!oldest.done) cache.delete(oldest.value);
  }
  return items;
}

/** Test seam: start each test from a clean cache. */
export function resetProjectionCache(): void {
  cache.clear();
}
