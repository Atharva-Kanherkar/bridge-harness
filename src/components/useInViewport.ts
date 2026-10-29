import { useEffect, useRef, useState, type RefObject } from "react";

/**
 * The distance ahead of the viewport at which a block counts as visible, so
 * colour arrives just before a block scrolls in rather than while it is
 * already being read.
 */
const DEFAULT_ROOT_MARGIN = "200px";

/**
 * Whether an element has ever been in the viewport.
 *
 * One-shot on purpose: once a block has been seen, it stays visible to this
 * hook. Scrolling back out keeps the colour it already has and never schedules
 * a second tokenization pass, which is the same rule `COLORIZE_DEBOUNCE_MS`
 * enforces for a growing fence.
 *
 * Where `IntersectionObserver` is unavailable (jsdom, older webviews) the
 * element counts as visible immediately, so the caller behaves exactly as it
 * did before the gate existed.
 */
export function useInViewport<T extends Element>(rootMargin = DEFAULT_ROOT_MARGIN): [RefObject<T>, boolean] {
  const ref = useRef<T>(null);
  const [visible, setVisible] = useState(() => typeof IntersectionObserver === "undefined");

  useEffect(() => {
    if (visible) return;
    const element = ref.current;
    if (!element || typeof IntersectionObserver === "undefined") {
      setVisible(true);
      return;
    }
    const observer = new IntersectionObserver(entries => {
      if (!entries.some(entry => entry.isIntersecting)) return;
      setVisible(true);
      observer.disconnect();
    }, { rootMargin });
    observer.observe(element);
    return () => observer.disconnect();
  }, [rootMargin, visible]);

  return [ref, visible];
}
