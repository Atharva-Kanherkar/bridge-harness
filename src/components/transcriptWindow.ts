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

/** How many leading row keys the window remembers to recognise its rows after
 * older history arrives above them. More than one, because the row at the
 * seam can be regrouped once its earlier half arrives. */
const ANCHOR_ROWS = 8;

/** Reads the key of the row at an index, or nothing past the end. */
export type RowKeyAt = (index: number) => string | undefined;

function leadingKeys(total: number, rowKeyAt?: RowKeyAt): string[] {
  if (!rowKeyAt) return [];
  const keys: string[] = [];
  for (let index = 0; index < Math.min(ANCHOR_ROWS, total); index += 1) {
    const key = rowKeyAt(index);
    if (key !== undefined) keys.push(key);
  }
  return keys;
}

/**
 * How far the rows that used to lead the list have moved: positive when older
 * rows arrived above them, negative when the oldest rows left the window.
 * `undefined` when none of them is in the list any more.
 */
export function leadingShift(anchor: readonly string[], total: number, rowKeyAt: RowKeyAt): number | undefined {
  for (let index = 0; index < anchor.length; index += 1) {
    for (let now = 0; now < total; now += 1) {
      if (rowKeyAt(now) === anchor[index]) return now - index;
    }
  }
  return undefined;
}

interface WindowState {
  key: string;
  /** Whether the window has seen a non-empty transcript. */
  seeded: boolean;
  /** First mounted row. Rows before it are older history, not yet mounted. */
  start: number;
  /**
   * The boundary the opening window had when this chat first rendered rows.
   * Tail growth must not move it: a row that was already below the opening
   * window keeps its entrance animation instead of flipping to instant.
   */
  instantBefore: number;
  /** The keys that led the list when the window last looked. */
  anchor: readonly string[];
}

function windowState(key: string, total: number, full: boolean, rowKeyAt?: RowKeyAt): WindowState {
  // `seeded` is what makes history that arrives after the first commit still
  // open on its newest rows instead of mounting all at once.
  return {
    key,
    seeded: total > 0,
    start: initialWindowStart(total, full),
    instantBefore: Math.max(0, total - INITIAL_TRANSCRIPT_ROWS),
    anchor: leadingKeys(total, rowKeyAt),
  };
}

export interface TranscriptWindow {
  /** Index of the first row to mount. */
  start: number;
  /** Rows before this index arrived with the opening window. */
  instantBefore: number;
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
 *
 * `ready` says the rows in `total` belong to `key`. Selecting a chat swaps the
 * session a commit before its forest follows, and seeding from the previous
 * chat's row count is how a short chat used to open a long one fully mounted.
 * Until the forest catches up the window keeps describing the rows it can see.
 *
 * `rowKeyAt` lets the window follow its rows when history arrives *above*
 * them: a long chat opens on its newest page and the full window replaces it a
 * moment later. As an index into the head, the start would otherwise land deep
 * inside the older rows and mount all of them at once. With it, the rows
 * already on screen stay mounted and the older ones hydrate above them.
 */
export function useTranscriptWindow(total: number, key: string, full: boolean, ready = true, rowKeyAt?: RowKeyAt): TranscriptWindow {
  const [state, setState] = useState(() => windowState(key, total, full, rowKeyAt));
  let current = state;
  // Adjust during render, React's own pattern for state derived from props: a
  // new session (or the first rows of a chat that opened empty) must mount its
  // newest rows in this commit, not one commit later. A forced window is
  // remembered, not re-derived: clearing a search highlight must not collapse
  // the history it just opened, or yank the row the reader jumped to.
  if (ready && (current.key !== key || (!current.seeded && total > 0) || (full && current.start !== 0))) {
    current = windowState(key, total, full, rowKeyAt);
    setState(current);
  }
  // The rows at the head changed under the same chat. Move the start by as
  // much as the old leading rows moved, so exactly the rows already mounted
  // stay mounted. Comparing the first key is the whole cost on every other
  // render, including each streamed character.
  if (ready && rowKeyAt && current.key === key && rowKeyAt(0) !== current.anchor[0]) {
    const shift = leadingShift(current.anchor, total, rowKeyAt) ?? 0;
    current = {
      ...current,
      start: full ? 0 : Math.max(0, current.start + shift),
      instantBefore: Math.max(0, current.instantBefore + shift),
      anchor: leadingKeys(total, rowKeyAt),
    };
    setState(current);
  }
  // A rewind can shrink the branch under the window. Clamp to the opening
  // window rather than to `total`: clamping to the row count would leave no
  // rows at all when the branch shrinks past the stored start. Write the clamp
  // into state so the next idle slice starts from the rows that are actually
  // mounted; reading only the clamped value leaves the queue stalled forever.
  //
  // While the forest still belongs to the previous chat, `full` describes the
  // chat being opened, not the rows on screen. Applying it early would mount
  // every old row in the mismatch commit.
  const ceiling = !ready ? initialWindowStart(total) : full ? 0 : initialWindowStart(total);
  const start = Math.min(current.start, ceiling);
  if (ready && start !== current.start) {
    current = { ...current, start };
    setState(current);
  }
  // The idle callback reads the latest total without re-arming on every
  // streamed character: a tail row must not cancel and reschedule hydration.
  const totalRef = useRef(total);
  useEffect(() => { totalRef.current = total; });

  useEffect(() => {
    if (!ready || start <= 0) return;
    let cancelled = false;
    const handle = scheduleIdle(() => {
      if (cancelled) return;
      setState(previous => {
        if (previous.key !== key || previous.start <= 0) return previous;
        const next = nextWindowStart(Math.min(previous.start, initialWindowStart(totalRef.current)));
        return next === previous.start ? previous : { ...previous, start: next };
      });
    });
    return () => { cancelled = true; cancelIdle(handle); };
  }, [start, key, ready]);

  return { start, instantBefore: current.instantBefore };
}
