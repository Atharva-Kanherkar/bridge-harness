import { useEffect, useRef } from "react";
import { bridgeApi } from "./api";
import type { AttentionItem } from "./types";

/** How long a request waits before it may notify. An auto-approved permission
 * (Full access) is opened and answered within a few milliseconds; holding
 * past that is what keeps it from flashing a toast. */
export const NEEDS_YOU_HOLD_MS = 1_000;
const PAGE_SIZE = 100;
const RETRY_FIRST_MS = 1_000;
const RETRY_MAX_MS = 30_000;
/** What this client knows how to show. A newer daemon's kinds arrive as
 * `other` (or, from a TypeScript peer, as a string this build has never seen)
 * and are skipped rather than shown as something they are not. */
const SHOWN_KINDS: ReadonlySet<string> = new Set<AttentionItem["kind"]>(["finished", "failed", "needsYou"]);

/**
 * Follows the core attention ledger and hands each new row to `onArrive`.
 *
 * Launch is a moment, not a sequence: rows created before mount are never
 * replayed, and rows created after it are delivered even when they landed
 * before the cursor existed (the launch read was slow, or failed and was
 * retried). The cursor then starts at the ledger's newest sequence. Each
 * `attention/changed` hint reads
 * from the cursor; reads never overlap, and a hint that lands mid-read buys
 * exactly one more read. Seen, resolved and session-less rows are skipped. A
 * `needsYou` row is held, then re-read, and arrives only if it is still the
 * same open request.
 */
export function useAttentionFeed(onArrive: (item: AttentionItem) => void, holdMs = NEEDS_YOU_HOLD_MS): void {
  const onArriveRef = useRef(onArrive);
  onArriveRef.current = onArrive;

  useEffect(() => {
    let active = true;
    const mountedAt = Date.now();
    let cursor: number | null = null;
    let reading = false;
    let readAgain = false;
    let initializing = false;
    let retryDelay = RETRY_FIRST_MS;
    let retryTimer: number | undefined;
    const holds = new Set<number>();
    let unlisten: (() => void) | undefined;

    const deliver = (item: AttentionItem) => {
      if (active) onArriveRef.current(item);
    };

    const recheck = async (item: AttentionItem) => {
      try {
        const page = await bridgeApi.listAttention({ afterSequence: item.sequence - 1, limit: 1 });
        const current = page.items[0];
        // A different row back means this one was superseded.
        if (current?.sequence === item.sequence && !current.resolvedAt && !current.seenAt) deliver(current);
      } catch {
        // A failed re-check drops the row rather than risk a stale approval card.
      }
    };

    const consider = (item: AttentionItem) => {
      if (item.seenAt || item.resolvedAt || !item.sessionId || !SHOWN_KINDS.has(item.kind)) return;
      if (item.kind !== "needsYou") {
        deliver(item);
        return;
      }
      const timer = window.setTimeout(() => {
        holds.delete(timer);
        if (active) void recheck(item);
      }, holdMs);
      holds.add(timer);
    };

    const read = async () => {
      if (cursor === null) {
        // No cursor yet: a launch read is out, or failed. A hint means the
        // backend is answering, so try now rather than wait for the timer.
        readAgain = true;
        void initialize();
        return;
      }
      if (reading) {
        // Mid-read: remember the hint and read once more after.
        readAgain = true;
        return;
      }
      reading = true;
      try {
        do {
          readAgain = false;
          for (;;) {
            const page = await bridgeApi.listAttention({ afterSequence: cursor, limit: PAGE_SIZE });
            if (!active) return;
            page.items.forEach(consider);
            if (page.items.length < PAGE_SIZE) {
              cursor = Math.max(cursor, page.latestSequence);
              break;
            }
            cursor = page.items[page.items.length - 1].sequence;
          }
        } while (readAgain && active);
      } catch {
        // An older daemon without the ledger, or a dropped connection: the
        // next hint reads again from the same cursor.
      } finally {
        reading = false;
      }
    };

    const initialize = async () => {
      if (cursor !== null || initializing || !active) return;
      initializing = true;
      window.clearTimeout(retryTimer);
      retryTimer = undefined;
      try {
        const head = await bridgeApi.listAttention({ limit: 1 });
        if (!active) return;
        const latest = head.latestSequence;
        // The cursor is about to skip everything up to `latest`; deliver what
        // on its last page was created after launch.
        const recent = await bridgeApi.listAttention({ afterSequence: Math.max(0, latest - PAGE_SIZE), limit: PAGE_SIZE });
        if (!active) return;
        cursor = latest;
        recent.items
          .filter(item => item.sequence <= latest && Date.parse(item.createdAt) > mountedAt)
          .forEach(consider);
      } catch {
        // The daemon is starting or reconnecting: one failure is not "this
        // backend has no ledger". Try again, backing off.
        if (active) {
          retryTimer = window.setTimeout(() => { void initialize(); }, retryDelay);
          retryDelay = Math.min(retryDelay * 2, RETRY_MAX_MS);
        }
        return;
      } finally {
        initializing = false;
      }
      if (readAgain) void read();
    };

    void (async () => {
      // Subscribe before taking the cursor, so a row written in between is
      // read rather than lost.
      try {
        const off = await bridgeApi.onAttentionChanged(() => { void read(); });
        if (!active) { off(); return; }
        unlisten = off;
      } catch {
        // No attention channel on this backend; nothing to follow.
        return;
      }
      void initialize();
    })();

    return () => {
      active = false;
      window.clearTimeout(retryTimer);
      holds.forEach(timer => window.clearTimeout(timer));
      holds.clear();
      unlisten?.();
    };
  }, [holdMs]);
}
