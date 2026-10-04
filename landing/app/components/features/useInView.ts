"use client";

import { useEffect, useRef, useState } from "react";

/*
 * Drives the live panels. Each panel is a timeline of `length` ticks that loops while it is
 * on screen, so the visual reads as the app running rather than a picture of it.
 *
 * The server and the first client render both draw the last tick, so the markup is the
 * finished scene without JavaScript and hydration matches. Scrolling a panel into view
 * restarts it from the top; scrolling away pauses it. `prefers-reduced-motion` never starts
 * the clock, so those readers keep the finished frame.
 */
export function usePlayback<T extends HTMLElement>(length: number, tick = 80) {
  const ref = useRef<T>(null);
  const [t, setT] = useState(length - 1);
  const [live, setLive] = useState(false);

  useEffect(() => {
    const element = ref.current;
    if (!element || window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) setT(0);
        setLive(entry.isIntersecting);
      },
      { threshold: 0.25 },
    );
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    if (!live) return;
    const id = window.setInterval(() => setT(value => (value + 1) % length), tick);
    return () => window.clearInterval(id);
  }, [live, length, tick]);

  return { ref, t } as const;
}

/** The part of `text` typed by tick `t`, starting at tick `start`. */
export function typed(text: string, t: number, start: number, perTick = 2) {
  return text.slice(0, Math.max(0, (t - start) * perTick));
}

/** Eases a number from `from` to `to` between ticks `start` and `end`. */
export function tween(t: number, start: number, end: number, from: number, to: number) {
  const p = Math.min(1, Math.max(0, (t - start) / (end - start)));
  return from + (to - from) * (1 - (1 - p) ** 3);
}
