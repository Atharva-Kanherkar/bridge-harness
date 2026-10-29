/// <reference types="vite/client" />
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
 * The outer key is a `WeakMap`: a forest snapshot belongs to one session, and
 * once the app drops it the projection goes with it. An LRU of arrays would
 * hold eight whole forests alive and would have to guess when one is dead. Only
 * the leaves within one array are bounded, because that array is the snapshot
 * the UI is still reading and a branch switch should not replay it.
 */
const MAX_LEAVES_PER_ARRAY = 4;

interface LeafVariant {
  leaf: string | null;
  items: ConversationItem[];
}

interface EntryVariants {
  /** Most recently used first. */
  variants: LeafVariant[];
}

let cache = new WeakMap<SessionEntry[], EntryVariants>();

function project(entries: SessionEntry[], leaf: string | null): ConversationItem[] {
  const items = projectSessionConversation(entries, leaf);
  // A cached array is shared by every caller. Freezing it in development turns
  // an accidental in-place edit into a loud error instead of a projection that
  // silently changes under another reader.
  if (import.meta.env.DEV) Object.freeze(items);
  return items;
}

export function projectSessionConversationCached(
  entries: SessionEntry[],
  activeLeafId: string | null,
): ConversationItem[] {
  const leaf = activeLeafId ?? null;
  const cached = cache.get(entries);
  if (cached) {
    const index = cached.variants.findIndex(variant => variant.leaf === leaf);
    if (index !== -1) {
      const [hit] = cached.variants.splice(index, 1);
      cached.variants.unshift(hit);
      return hit.items;
    }
    const items = project(entries, leaf);
    cached.variants.unshift({ leaf, items });
    if (cached.variants.length > MAX_LEAVES_PER_ARRAY) cached.variants.pop();
    return items;
  }

  const items = project(entries, leaf);
  cache.set(entries, { variants: [{ leaf, items }] });
  return items;
}

/** Test seam: start each test from a clean cache. */
export function resetProjectionCache(): void {
  cache = new WeakMap();
}
