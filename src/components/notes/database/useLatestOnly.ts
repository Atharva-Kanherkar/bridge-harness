// Answers that may land out of order (two schema reads, a read and a write)
// apply newest first only: each request takes a ticket when it starts, and an
// answer older than one already applied is dropped.

import { useMemo, useRef } from "react";

export interface LatestOnly {
  /** A ticket for a request starting now. */
  take: () => number;
  /** Whether the answer to `ticket` may apply; records it as the newest if so. */
  accept: (ticket: number) => boolean;
}

export function useLatestOnly(): LatestOnly {
  const issued = useRef(0);
  const applied = useRef(0);
  return useMemo(() => ({
    take: () => ++issued.current,
    accept: ticket => {
      if (ticket <= applied.current) return false;
      applied.current = ticket;
      return true;
    },
  }), []);
}
