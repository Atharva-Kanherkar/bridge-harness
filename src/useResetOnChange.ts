import { useState } from "react";

/**
 * Runs `reset` during render on the first render whose `key` differs from the
 * one the last render saw. It never runs on mount: state starts from its own
 * initial value.
 *
 * This is React's "adjusting some state when a prop changes" pattern. An effect
 * that resets state paints the stale value first and then renders again; this
 * corrects it before anything paints. `reset` may only set this component's
 * own state. Side effects (refs, a parent's callback, the DOM) stay in effects.
 */
export function useResetOnChange(key: readonly unknown[], reset: () => void): void {
  const [seen, setSeen] = useState(key);
  if (!sameKey(seen, key)) {
    setSeen(key);
    reset();
  }
}

function sameKey(left: readonly unknown[], right: readonly unknown[]): boolean {
  return left.length === right.length && left.every((value, index) => Object.is(value, right[index]));
}
