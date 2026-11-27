import { useCallback, useRef } from "react";
import { useLatest } from "./useLatest";

/**
 * A function whose identity never changes but always calls the newest `fn`.
 * For handlers passed to memoized children: an inline arrow is a new prop on
 * every render, and one new prop is enough to re-render a whole transcript.
 * Only call the result from event handlers or effects, never during render.
 */
export function useStableCallback<Args extends unknown[], Result>(fn: (...args: Args) => Result): (...args: Args) => Result {
  const latest = useLatest(fn);
  return useCallback((...args: Args) => latest.current(...args), [latest]);
}

/**
 * The previous array when `next` holds the same items in the same order. A
 * filter over a shared buffer returns a new array every time the buffer
 * changes, even when none of the changes belong to this slice; keeping the
 * old array lets memoized consumers skip the render.
 */
export function useShallowStableArray<T>(next: T[]): T[];
export function useShallowStableArray<T>(next: T[] | undefined): T[] | undefined;
export function useShallowStableArray<T>(next: T[] | undefined): T[] | undefined {
  // The ref is an identity cache: it only decides whether the last array can be
  // reused, never what renders. State would cost the caller (App) a second
  // render for every new event batch.
  /* oxlint-disable react/refs -- an identity cache read and written in render on purpose */
  const previous = useRef(next);
  const current = previous.current;
  if (current !== next && !(current && next && sameItems(current, next))) previous.current = next;
  return previous.current;
  /* oxlint-enable react/refs */
}

export function sameItems<T>(left: readonly T[], right: readonly T[]): boolean {
  if (left.length !== right.length) return false;
  for (let index = 0; index < left.length; index += 1) {
    if (left[index] !== right[index]) return false;
  }
  return true;
}
