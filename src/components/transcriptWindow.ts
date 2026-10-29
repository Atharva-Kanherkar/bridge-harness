import { useEffect, useRef, useState } from "react";

/**
 * How much of a chat is on screen at first paint.
 *
 * A long turn-heavy chat produces one row per thought, tool run and message.
 * Mounting every row in the opening commit is what made opening a 1,500-event
 * chat take seconds: the newest message cannot paint until the oldest row has
 * rendered. The reader only ever needs the bottom of the transcript first, so
 * the opening commit mounts that and the rest arrives in idle slices above it.
 */
export const INITIAL_TRANSCRIPT_ROWS = 32;
export const TRANSCRIPT_HYDRATE_CHUNK = 48;

/**
 * Index of the first row mounted on the first commit: everything before it is
 * older history that hydrates later. `full` starts at the head, mounting
 * everything.
 */
export function initialWindowStart(total: number, full = false): number {
  if (full) return 0;
  return Math.max(0, Math.max(0, total) - INITIAL_TRANSCRIPT_ROWS);
}

/** One idle slice: the window start moves one chunk toward the head. */
export function nextWindowStart(start: number): number {
  return Math.max(0, Math.max(0, start) - TRANSCRIPT_HYDRATE_CHUNK);
}

interface WindowState {
  key: string;
  /** Whether the window has seen a non-empty transcript. */
  seeded: boolean;
  /** First mounted row. Rows before it are older history, not yet mounted. */
  start: number;
}

function windowState(key: string, total: number, full: boolean): WindowState {
  // `seeded` is what makes history that arrives after the first commit still
  // open on its newest rows instead of mounting all at once.
  return { key, seeded: total > 0, start: initialWindowStart(total, full) };
}

type IdleHandle = { kind: "idle" | "timeout"; id: number };

function scheduleIdle(callback: () => void): IdleHandle {
  if (typeof window === "undefined") return { kind: "timeout", id: -1 };
  // WKWebView (Safari) still has no idle callback; a frame-spaced timeout is
  // the fallback, and it keeps the same one-slice-at-a-time shape.
  if (typeof window.requestIdleCallback === "function") {
    return { kind: "idle", id: window.requestIdleCallback(callback, { timeout: 200 }) };
  }
  return { kind: "timeout", id: window.setTimeout(callback, 16) };
}

function cancelIdle(handle: IdleHandle): void {
  if (handle.id < 0 || typeof window === "undefined") return;
  if (handle.kind === "idle" && typeof window.cancelIdleCallback === "function") window.cancelIdleCallback(handle.id);
  else window.clearTimeout(handle.id);
}

/**
 * The index of the first transcript row to mount now.
 *
 * The window is keyed by session: switching chats starts from the newest rows
 * again, and a pending callback for the previous chat cannot add rows to the
 * new one. Rows arriving at the *tail* (a streaming reply, an optimistic
 * bubble) do not move the start: it is an index into the head, so those rows
 * simply join the live tail and no mounted row is ever remounted.
 *
 * `full` mounts everything at once (start zero). Callers use it for a row that
 * must exist on the first commit (a search jump) and for a chat reopening at a
 * remembered offset, which a partial transcript cannot restore.
 */
export function useTranscriptWindow(total: number, key: string, full: boolean): number {
  const [state, setState] = useState(() => windowState(key, total, full));
  let current = state;
  // Adjust during render, React's own pattern for state derived from props: a
  // new session (or the first rows of a chat that opened empty) must mount its
  // newest rows in this commit, not one commit later. A forced window is
  // remembered, not re-derived: clearing a search highlight must not collapse
  // the history it just opened, or yank the row the reader jumped to.
  if (current.key !== key || (!current.seeded && total > 0) || (full && current.start !== 0)) {
    current = full
      ? { key, seeded: true, start: 0 }
      : windowState(key, total, false);
    setState(current);
  }
  // A rewind can shrink the branch under the window.
  const start = full ? 0 : Math.min(current.start, Math.max(0, total));
  const startRef = useRef(start);
  startRef.current = start;

  useEffect(() => {
    if (startRef.current <= 0) return;
    let cancelled = false;
    const handle = scheduleIdle(() => {
      if (cancelled) return;
      setState(previous => previous.key === key && previous.start > 0
        ? { ...previous, start: nextWindowStart(previous.start) }
        : previous);
    });
    return () => { cancelled = true; cancelIdle(handle); };
  }, [start, key]);

  return start;
}
