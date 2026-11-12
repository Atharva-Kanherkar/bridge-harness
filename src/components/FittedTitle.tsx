import { Fragment, useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from "react";

/**
 * A chat title that fits its width by wording, not by an ellipsis.
 *
 * The title job writes each automatic title as a ladder: the same name at five
 * or so lengths, from a word or two to a dozen (`title_jobs::ladder`). This
 * shows the longest rung the row has room for, so dragging the rail narrower
 * rewords "Why the SQLite WAL keeps growing in bridged" down to "SQLite WAL
 * growth" and then "WAL growth" instead of cutting it to "Why the SQLite W…".
 * A chat with no ladder (a user's own name, a harness Bridge cannot name on)
 * renders its title exactly as before.
 *
 * When the rung changes, only the words the new one adds animate in; the words
 * both share stay where they are, so the title reads as growing or shrinking in
 * place rather than being swapped out.
 */

/** How wide a string renders in the title's font, or null when this webview cannot say. */
export type MeasureText = (text: string) => number | null;

/** The longest rung that fits `width`, or the narrowest when none does (it is
 *  then truncated like any title). Rungs are compared by measured width rather
 *  than by their order, so a ladder that arrives out of order still picks right.
 *  Null when the text cannot be measured, which leaves the plain title. */
export function pickRung(rungs: readonly string[], width: number, measure: MeasureText): string | null {
  let best: { rung: string; width: number } | null = null;
  let narrowest: { rung: string; width: number } | null = null;
  for (const rung of rungs) {
    const measured = measure(rung);
    if (measured === null) return null;
    if (!narrowest || measured < narrowest.width) narrowest = { rung, width: measured };
    // half a pixel of slack: layout widths are fractional, measurements rounded.
    if (measured <= width + 0.5 && (!best || measured > best.width)) best = { rung, width: measured };
  }
  return (best ?? narrowest)?.rung ?? null;
}

const fold = (word: string) => word.toLowerCase().replace(/[^\p{L}\p{N}#]/gu, "");

/** For each word of `next`, whether `previous` did not already show it. Counted,
 *  so a word `next` repeats animates the second time if `previous` had it once. */
export function addedWords(previous: string, next: string): boolean[] {
  const left = new Map<string, number>();
  for (const word of previous.split(/\s+/)) left.set(fold(word), (left.get(fold(word)) ?? 0) + 1);
  return next.split(/\s+/).filter(Boolean).map(word => {
    const count = left.get(fold(word)) ?? 0;
    if (count > 0) left.set(fold(word), count - 1);
    return count === 0;
  });
}

let context: OffscreenCanvasRenderingContext2D | null | undefined;
const widths = new Map<string, number>();

function textWidth(text: string, font: string): number | null {
  if (context === undefined) {
    context = typeof OffscreenCanvas === "function" ? new OffscreenCanvas(1, 1).getContext("2d") : null;
  }
  if (!context) return null;
  const key = `${font}\n${text}`;
  let width = widths.get(key);
  if (width === undefined) {
    context.font = font;
    width = context.measureText(text).width;
    if (widths.size > 4000) widths.clear();
    widths.set(key, width);
  }
  return width;
}

/** Measures in the element's own font, read now: a row's title turns medium
 *  weight when the chat is selected, which makes every rung wider. */
function measureIn(element: HTMLElement): MeasureText {
  const style = getComputedStyle(element);
  const font = `${style.fontStyle} ${style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
  // canvas ignores CSS letter-spacing; the rows use a slightly negative one.
  const spacing = Number.parseFloat(style.letterSpacing) || 0;
  return text => {
    const width = textWidth(text, font);
    return width === null ? null : width + spacing * [...text].length;
  };
}

// One observer for every title on screen, rather than one per row.
const watched = new Map<Element, (width: number) => void>();
let observer: ResizeObserver | undefined;

function refitAll() {
  // a web font that finished loading changes every width measured before it.
  widths.clear();
  for (const [element, refit] of watched) refit(element.getBoundingClientRect().width);
}

function watch(element: Element, refit: (width: number) => void): () => void {
  if (typeof ResizeObserver === "undefined") return () => {};
  observer ??= new ResizeObserver(entries => {
    for (const entry of entries) watched.get(entry.target)?.(entry.contentRect.width);
  });
  if (watched.size === 0) document.fonts?.addEventListener?.("loadingdone", refitAll);
  watched.set(element, refit);
  observer.observe(element);
  return () => {
    observer?.unobserve(element);
    watched.delete(element);
    if (watched.size === 0) document.fonts?.removeEventListener?.("loadingdone", refitAll);
  };
}

// Static markup (the rail's server-rendered tests) has no layout to fit to.
const useIsomorphicLayoutEffect = typeof window === "undefined" ? useEffect : useLayoutEffect;

/** Gap between the words a swap adds, so they arrive left to right. */
const WORD_STAGGER_MS = 35;

export function FittedTitle({ title, ladder, className }: {
  /** The chat's title: shown when there is no ladder, or before it is measured. */
  title: string;
  /** The title at several lengths, shortest first. */
  ladder?: readonly string[];
  className?: string;
}) {
  const ref = useRef<HTMLSpanElement>(null);
  // `from` is what was showing before this rung, when the change should animate.
  const [shown, setShown] = useState<{ text: string; from: string | null } | null>(null);
  const titleRef = useRef(title);
  titleRef.current = title;
  const mounted = useRef(false);
  const rungs = ladder && ladder.length > 1 ? ladder : null;
  const key = rungs?.join("\n") ?? "";

  // Layout effect, so the first fitted rung is in place before the row paints:
  // a row never flashes its title and then rewords itself.
  useIsomorphicLayoutEffect(() => {
    const element = ref.current;
    if (!element || !rungs) {
      setShown(null);
      return;
    }
    // A row that mounts already fitted does not animate; a ladder that arrives
    // for a row on screen, or a resize, does.
    let quiet = !mounted.current;
    const refit = (width: number) => {
      const text = pickRung(rungs, width, measureIn(element));
      setShown(current => {
        if (text === null) return null;
        if (current?.text === text) return current;
        return { text, from: quiet ? null : current?.text ?? titleRef.current };
      });
    };
    refit(element.getBoundingClientRect().width);
    quiet = false;
    mounted.current = true;
    return watch(element, refit);
    // `rungs` is keyed by its contents; `className` changes the font weight.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, className]);

  const text = shown?.text ?? title;
  const from = shown?.from ?? null;
  if (from === null) {
    return <span ref={ref} className={className}>{text}</span>;
  }
  const added = addedWords(from, text);
  let order = 0;
  return (
    <span ref={ref} className={className}>
      {text.split(/\s+/).filter(Boolean).map((word, index) => (
        <Fragment key={index}>
          {index > 0 && " "}
          {added[index] ? (
            // Keyed by the rung, so a word animates again on every change.
            <span
              key={text}
              className="title-word-in"
              style={{ "--title-word-delay": `${order++ * WORD_STAGGER_MS}ms` } as CSSProperties}
            >
              {word}
            </span>
          ) : word}
        </Fragment>
      ))}
    </span>
  );
}
