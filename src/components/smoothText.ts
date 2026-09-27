import { useEffect, useState } from "react";
import { useReducedMotion } from "framer-motion";

/** How long a burst of streamed text takes to reveal: short enough to read as live, long enough to flow. */
export const REVEAL_MS = 120;

/** Characters shown `elapsedMs` into revealing from `from` up to `target`, linearly. */
export function revealedLength(from: number, target: number, elapsedMs: number): number {
  if (target <= from) return target;
  const progress = Math.min(1, Math.max(0, elapsedMs) / REVEAL_MS);
  return Math.min(target, Math.ceil(from + (target - from) * progress));
}

/** Where a new reveal starts: where the old one had reached, unless the text was replaced rather than extended. */
export function revealStart(previous: string, shown: number, next: string, streaming: boolean): number {
  if (!streaming || !next.startsWith(previous.slice(0, shown))) return next.length;
  return Math.min(shown, next.length);
}

/**
 * Streamed text, revealed at an even pace instead of in provider-sized lurches.
 * Text present on mount shows at once; every later extension drains over `REVEAL_MS`.
 */
export function useSmoothText(text: string, streaming: boolean): string {
  const reduced = useReducedMotion() ?? false;
  const [run, setRun] = useState(() => ({ text, from: text.length, start: 0 }));
  const [clock, setClock] = useState(0);
  let current = run;
  if (text !== run.text) {
    const now = performance.now();
    const shownNow = revealedLength(run.from, run.text.length, now - run.start);
    current = { text, from: reduced ? text.length : revealStart(run.text, shownNow, text, streaming), start: now };
    setRun(current);
  }
  let shown = revealedLength(current.from, text.length, clock - current.start);
  useEffect(() => {
    if (shown >= text.length) return;
    const frame = requestAnimationFrame(setClock);
    return () => cancelAnimationFrame(frame);
  }, [shown, text.length, clock]);
  // Never split a surrogate pair: half an emoji draws as a replacement glyph.
  if (shown > 0 && shown < text.length && /[\uD800-\uDBFF]/.test(text[shown - 1])) shown -= 1;
  return text.slice(0, shown);
}
