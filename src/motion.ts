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
 * A transition at `duration`, or an instant one when the user prefers reduced
 * motion. Call it at the top of any component that animates.
 */
export function useMotionTransition(duration: number = MOTION_DURATION.reveal): Transition {
  const reduced = useReducedMotion();
  return reduced ? { duration: 0 } : { duration, ease: BRIDGE_EASE };
}

/** A stagger for a list revealed as a unit; flat under reduced motion. */
export function useMotionStagger(duration: number = MOTION_DURATION.reveal): Transition {
  const reduced = useReducedMotion();
  return reduced
    ? { duration: 0 }
    : { duration, ease: BRIDGE_EASE, staggerChildren: MOTION_STAGGER };
}
