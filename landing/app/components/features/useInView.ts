"use client";

import { useEffect, useRef, useState } from "react";

/*
 * Drives the staged panels. `animation-timeline: view()` was doing this before, which meant
 * the animation silently never ran outside Chromium; an observer plays everywhere.
 *
 * `armed` stays false until after mount, so the server-rendered markup carries no
 * `data-play` attribute and the parts are plainly visible without JavaScript. Leaving the
 * viewport resets it, so scrolling back replays the panel.
 */
export function useInView<T extends HTMLElement>() {
  const ref = useRef<T>(null);
  const [state, setState] = useState<"idle" | "out" | "in">("idle");

  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    setState("out");
    const observer = new IntersectionObserver(
      ([entry]) => setState(entry.isIntersecting ? "in" : "out"),
      { threshold: 0.3, rootMargin: "0px 0px -8% 0px" },
    );
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  return { ref, play: state === "idle" ? undefined : state === "in" ? "true" : "false" } as const;
}
