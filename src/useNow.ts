import { useCallback, useState, useSyncExternalStore } from "react";

/**
 * The wall clock in milliseconds, read outside render so rendering stays pure.
 * It re-reads the clock whenever `intervalMs` changes and then every
 * `intervalMs` while it is a number. `null` holds the last reading, so a view
 * with nothing live costs no timer.
 */
export function useNow(intervalMs: number | null = 1000): number {
  const [clock] = useState(createClock);
  const subscribe = useCallback((onChange: () => void) => clock.follow(onChange, intervalMs), [clock, intervalMs]);
  return useSyncExternalStore(subscribe, clock.read, clock.read);
}

function createClock() {
  let now = Date.now();
  return {
    read: () => now,
    follow(onChange: () => void, intervalMs: number | null) {
      // React compares the snapshot again right after subscribing, so moving
      // the reading here re-renders with a fresh clock without a second timer.
      now = Date.now();
      if (intervalMs === null) return () => {};
      const id = setInterval(() => {
        now = Date.now();
        onChange();
      }, intervalMs);
      return () => clearInterval(id);
    },
  };
}
