import { useEffect, useRef, useState, type RefObject } from "react";

/**
 * The distance ahead of the viewport at which a block counts as visible, so
 * colour arrives just before a block scrolls in rather than while it is
 * already being read.
 */
const DEFAULT_ROOT_MARGIN = "200px";

interface Watcher {
  observer: IntersectionObserver;
  callbacks: Map<Element, () => void>;
}

/// One observer per scroll root and margin, not one per code fence. The
/// transcript can hold hundreds of fences; they all watch the same scroller.
const watchers = new Map<Element | null, Map<string, Watcher>>();

/**
 * The nearest ancestor that actually scrolls this element, if there is one.
 * `rootMargin` is measured against the observer's root; with the default
 * viewport root, a fence inside the transcript scroller gets no look-ahead at
 * all because the scroller clips it before the viewport sees it.
 */
function scrollParent(element: Element): Element | null {
  let node = element.parentElement;
  while (node) {
    const style = getComputedStyle(node);
    if (/(auto|scroll|overlay)/.test(style.overflowY)) return node;
    node = node.parentElement;
  }
  return null;
}

function watch(element: Element, root: Element | null, rootMargin: string, onVisible: () => void): () => void {
  let byMargin = watchers.get(root);
  if (!byMargin) {
    byMargin = new Map();
    watchers.set(root, byMargin);
  }
  let watcher = byMargin.get(rootMargin);
  if (!watcher) {
    const callbacks = new Map<Element, () => void>();
    const observer = new IntersectionObserver(entries => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        const callback = callbacks.get(entry.target);
        if (!callback) continue;
        callbacks.delete(entry.target);
        observer.unobserve(entry.target);
        callback();
      }
      if (callbacks.size === 0) {
        observer.disconnect();
        // A late cleanup from an older watcher must not evict the watcher that
        // replaced it for the same root and margin.
        if (byMargin!.get(rootMargin)?.callbacks === callbacks) {
          byMargin!.delete(rootMargin);
          if (byMargin!.size === 0 && watchers.get(root) === byMargin) watchers.delete(root);
        }
      }
    }, { root, rootMargin });
    watcher = { observer, callbacks };
    byMargin.set(rootMargin, watcher);
  }
  watcher.callbacks.set(element, onVisible);
  watcher.observer.observe(element);
  return () => {
    watcher!.callbacks.delete(element);
    watcher!.observer.unobserve(element);
    if (watcher!.callbacks.size === 0) {
      watcher!.observer.disconnect();
      if (byMargin!.get(rootMargin) === watcher) {
        byMargin!.delete(rootMargin);
        if (byMargin!.size === 0 && watchers.get(root) === byMargin) watchers.delete(root);
      }
    }
  };
}

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
    return watch(element, scrollParent(element), rootMargin, () => setVisible(true));
  }, [rootMargin, visible]);

  return [ref, visible];
}
