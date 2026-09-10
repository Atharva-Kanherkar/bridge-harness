"use client";

import Image from "next/image";
import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { capabilities } from "../content/hero";

const DWELL_MS = 7000;

/*
 * Capability switcher over a real capture of the app. A row of tabs, each with a title and
 * one line of copy; the active tab carries a progress bar that fills over the dwell time,
 * then the next capability takes over. Hovering or focusing anything in the switcher holds
 * the current one. The frame is width-constrained to the page, so nothing bleeds off-screen,
 * and it is shown whole at its native 16:10 ratio, never cropped; the page scrolls instead.
 */
export default function HeroFrame() {
  const [active, setActive] = useState(0);
  const [held, setHeld] = useState(false);
  const [reduced, setReduced] = useState(false);
  const tabRefs = useRef<(HTMLButtonElement | null)[]>([]);
  const current = capabilities[active];

  useEffect(() => {
    const query = window.matchMedia("(prefers-reduced-motion: reduce)");
    const update = () => setReduced(query.matches);
    update();
    query.addEventListener("change", update);
    return () => query.removeEventListener("change", update);
  }, []);

  useEffect(() => {
    if (held || reduced) return;
    const timer = window.setTimeout(() => setActive((index) => (index + 1) % capabilities.length), DWELL_MS);
    return () => window.clearTimeout(timer);
  }, [active, held, reduced]);

  function select(index: number) {
    const next = (index + capabilities.length) % capabilities.length;
    setActive(next);
    tabRefs.current[next]?.focus();
  }

  function onKeyDown(event: KeyboardEvent<HTMLButtonElement>, index: number) {
    const keys: Record<string, number> = { ArrowRight: index + 1, ArrowLeft: index - 1, Home: 0, End: capabilities.length - 1 };
    if (!(event.key in keys)) return;
    event.preventDefault();
    select(keys[event.key]);
  }

  return (
    <div
      className="flex flex-col"
      onPointerEnter={() => setHeld(true)}
      onPointerLeave={() => setHeld(false)}
      onFocusCapture={() => setHeld(true)}
      onBlurCapture={() => setHeld(false)}
    >
      <div
        role="tablist"
        aria-label="What Bridge does"
        className="-mx-6 flex gap-2 overflow-x-auto px-6 pb-3 [scrollbar-width:none] sm:mx-0 sm:grid sm:grid-cols-4 sm:gap-2 xl:grid-cols-7 sm:px-0 sm:overflow-visible [&::-webkit-scrollbar]:hidden"
      >
        {capabilities.map((item, i) => {
          const selected = i === active;
          return (
            <button
              key={item.id}
              ref={(el) => {
                tabRefs.current[i] = el;
              }}
              type="button"
              role="tab"
              id={`tab-${item.id}`}
              aria-selected={selected}
              aria-controls="hero-frame"
              tabIndex={selected ? 0 : -1}
              onClick={() => setActive(i)}
              onKeyDown={(event) => onKeyDown(event, i)}
              className={`group relative flex w-[220px] shrink-0 flex-col overflow-hidden rounded-lg border px-3.5 pb-3.5 pt-3 text-left transition-colors duration-300 sm:w-auto ${
                selected ? "border-border-card bg-card" : "border-transparent hover:bg-card/60"
              }`}
            >
              <span className={`text-[13px] font-medium leading-5 transition-colors ${selected ? "text-foreground" : "text-muted-foreground group-hover:text-foreground"}`}>
                {item.label}
              </span>
              <span className={`mt-1 line-clamp-2 text-[12px] leading-[1.35] transition-colors ${selected ? "text-muted-foreground" : "text-faint"}`}>
                {item.title}
              </span>
              <span aria-hidden="true" className="absolute inset-x-0 bottom-0 h-px bg-border">
                {selected && (
                  <span
                    key={`${item.id}:${held}`}
                    className={`block h-full origin-left bg-foreground ${held || reduced ? "w-full" : "animate-[grow-x_7s_linear_forwards]"}`}
                  />
                )}
              </span>
            </button>
          );
        })}
      </div>

      <p key={current.id} className="min-h-10 pb-4 text-[13px] leading-5 text-muted-foreground animate-fade-up motion-reduce:animate-none">
        <span className="font-medium text-foreground">{current.title}. </span>
        {current.text}
      </p>

      <div
        id="hero-frame"
        role="tabpanel"
        aria-labelledby={`tab-${current.id}`}
        className="relative aspect-[16/10] w-full overflow-hidden rounded-xl border border-border-card bg-background shadow-[0_0_0_1px_#000,0_30px_90px_-30px_rgba(0,0,0,0.9)]"
      >
        {capabilities.map((item, i) => (
          <Image
            key={item.id}
            src={item.image}
            alt={i === active ? item.alt : ""}
            width={1600}
            height={1000}
            priority={i === 0}
            unoptimized
            className={`absolute inset-0 h-full w-full object-contain transition-opacity duration-500 ${
              i === active ? "opacity-100" : "opacity-0"
            }`}
          />
        ))}
      </div>
    </div>
  );
}
