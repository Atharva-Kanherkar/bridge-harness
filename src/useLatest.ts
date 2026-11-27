import { useLayoutEffect, useRef } from "react";

/**
 * A ref that always holds the newest `value`, for callbacks and effects that
 * must not re-subscribe when a prop changes. It is synced in a layout effect,
 * not during render, so it is safe under concurrent rendering: a render React
 * throws away never leaks its props into a callback.
 *
 * Read `.current` from event handlers, effects and timers, never during render.
 * Call it before any layout effect in the same component that reads it.
 */
export function useLatest<T>(value: T): { readonly current: T } {
  const latest = useRef(value);
  useLayoutEffect(() => {
    latest.current = value;
  });
  return latest;
}
