import { useEffect, useState } from "react";
import { useReducedMotion, type Transition } from "framer-motion";

/**
 * The one motion vocabulary the app animates with.
 *
 * Framer Motion is here for the transitions CSS cannot express — an *exit*, and
 * a height that animates to `auto`. Everything already expressible as a Tailwind
 * `transition-colors` or `active:scale-*` stays exactly that; a hover tint does
 * not need a JavaScript animation loop.
 *
 * Reduced motion is handled by collapsing the *duration*, not by branching to a
 * second set of variants: the same enter/exit code path runs and simply lands on
 * its final frame. That is what keeps the reduced-motion rendering from becoming
 * an untested parallel universe.
 */

/** The entrance curve `index.css` already uses, so JS and CSS motion agree. */
export const BRIDGE_EASE = [0.22, 1, 0.36, 1] as const;

/** Durations, in seconds. Short and directional — nothing here loops. */
export const MOTION_DURATION = {
  /** Status glyph swaps, chips, ticks. */
  tick: 0.16,
  /** Rows entering the transcript, disclosure bodies opening. */
  reveal: 0.22,
  /** Overlays and dialogs — the longest thing we animate. */
  overlay: 0.26,
} as const;

/** Gap between consecutive rows revealed together, matching `chat-message-enter`. */
export const MOTION_STAGGER = 0.04;

/**
 * The user's reduce preference, read live at mount. Framer's own
 * `useReducedMotion` caches a module-level snapshot on first use, which a
 * test (or a surface that mounts before the query lands) cannot influence;
 * this one queries per mount so the GitHub pane's choreography gate — and
 * anything else that asks directly — sees the actual setting.
 */
export function usePrefersReducedMotion(): boolean {
  const [reduced, setReduced] = useState(() =>
    typeof window !== "undefined" && typeof window.matchMedia === "function"
      ? window.matchMedia("(prefers-reduced-motion: reduce)").matches
      : false,
  );
  useEffect(() => {
    if (typeof window.matchMedia !== "function") return;
    const query = window.matchMedia("(prefers-reduced-motion: reduce)");
    const sync = () => setReduced(query.matches);
    sync();
    query.addEventListener("change", sync);
    return () => query.removeEventListener("change", sync);
  }, []);
  return reduced;
}

/**
 * A transition at `duration`, or an instant one when the user prefers reduced
 * motion. Call it at the top of any component that animates.
 */
export function useMotionTransition(duration: number = MOTION_DURATION.reveal, delay = 0): Transition {
  const reduced = useReducedMotion();
  // The delay goes too: under reduced motion "instant" has to mean instant, not
  // instant-after-a-pause.
  return reduced ? { duration: 0 } : { duration, delay, ease: BRIDGE_EASE };
}

/** A stagger for a list revealed as a unit; flat under reduced motion. */
export function useMotionStagger(duration: number = MOTION_DURATION.reveal): Transition {
  const reduced = useReducedMotion();
  return reduced
    ? { duration: 0 }
    : { duration, ease: BRIDGE_EASE, staggerChildren: MOTION_STAGGER };
}

/**
 * How a scroll driven from JavaScript should move: smoothly, unless the user
 * asked for less motion. The stylesheet's reduced-motion rule cannot reach an
 * explicit `behavior: "smooth"`, so every such call asks here instead.
 */
export function scrollBehavior(): ScrollBehavior {
  const reduce = typeof window !== "undefined" ? window.matchMedia?.("(prefers-reduced-motion: reduce)") : undefined;
  return reduce?.matches ? "auto" : "smooth";
}
