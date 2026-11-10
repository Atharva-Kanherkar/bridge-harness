import { useEffect, useState } from "react";
import { bridgeApi } from "./api";
import type { UnreadSummary } from "./types";

export const NOTHING_UNREAD: UnreadSummary = { total: 0, needsYou: 0, byChat: {}, bySession: {} };

/**
 * The unread summary, kept current. It is reread on every `attention/changed`
 * and whenever `sessionsKey` changes, because archiving or deleting a chat
 * changes what counts without moving the ledger. A response that lands after
 * a newer read was sent is dropped, and a failed read keeps the last summary.
 */
export function useUnreadSummary(sessionsKey: string): UnreadSummary {
  const [summary, setSummary] = useState<UnreadSummary>(NOTHING_UNREAD);
  useEffect(() => {
    let active = true;
    let latest = 0;
    let unlisten: (() => void) | undefined;
    const read = () => {
      const ticket = ++latest;
      bridgeApi.unreadSummary().then(
        next => { if (active && ticket === latest) setSummary(next); },
        () => undefined,
      );
    };
    // Read once the hint is wired, so a change between the two is not missed.
    void bridgeApi.onAttentionChanged(read).then(
      stop => { if (active) unlisten = stop; else stop(); },
      () => undefined,
    ).finally(() => { if (active) read(); });
    return () => {
      active = false;
      unlisten?.();
    };
  }, [sessionsKey]);
  return summary;
}
