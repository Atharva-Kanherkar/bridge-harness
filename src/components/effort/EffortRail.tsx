// The rail every slider-driven thinking control shares: one tick per level,
// a fill in the harness tint, a thumb that pops when the level changes.
//
// Its width does not depend on how many levels a model has — Claude's five and
// Codex's six draw into the same box; only the tick spacing differs. Every
// mark is an unlabeled button with a screen-reader label, so the footer's
// contract — one button per level carrying `data-effort` and `aria-pressed` —
// holds for every style built on it.

import { useEffect, useRef, useState } from "react";
import { cn } from "@/lib/utils";
import { harnessFillClass } from "../harnessMarks";
import { effortIndex, type EffortControlProps } from "./effortLevels";

export function EffortRail({ levels, value, onChange, disabled, harness, modelLabel }: EffortControlProps) {
  // A value off the ladder (unset: the provider's own default) parks the thumb
  // at the start with nothing lit and says so, rather than claiming "Low".
  const known = effortIndex(levels, value) >= 0;
  const index = Math.max(0, effortIndex(levels, value));
  const last = levels.length - 1;
  const ratio = known && last > 0 ? index / last : 0;
  const inert = disabled || !onChange;
  const rail = useRef<HTMLDivElement>(null);
  const fill = harnessFillClass(harness);
  // Replays the thumb pop on every change, not on first paint.
  const [pulse, setPulse] = useState(0);
  const first = useRef(true);
  useEffect(() => {
    if (first.current) { first.current = false; return; }
    setPulse(count => count + 1);
  }, [index, known]);

  const commit = (next: number) => {
    const clamped = Math.min(last, Math.max(0, next));
    const level = levels[clamped];
    if (!inert && level && level.value !== value) onChange?.(level.value);
  };
  const commitFromClientX = (clientX: number) => {
    const box = rail.current?.getBoundingClientRect();
    if (!box || box.width === 0) return;
    commit(Math.round(((clientX - box.left) / box.width) * last));
  };

  return <div
    ref={rail}
    className={cn("relative h-6 touch-none select-none", inert ? "cursor-default" : "cursor-pointer")}
    onPointerDown={event => {
      if (inert) return;
      // Capture keeps a drag alive past the rail's edge. Optional because not
      // every DOM (jsdom) implements it.
      event.currentTarget.setPointerCapture?.(event.pointerId);
      commitFromClientX(event.clientX);
    }}
    onPointerMove={event => { if (!inert && event.currentTarget.hasPointerCapture?.(event.pointerId)) commitFromClientX(event.clientX); }}
  >
    <div className="absolute inset-x-0 top-1/2 h-1 -translate-y-1/2 rounded-full bg-muted" />
    <div
      className={cn("absolute left-0 top-1/2 h-1 -translate-y-1/2 rounded-full transition-[width] duration-300 ease-[cubic-bezier(.2,.8,.2,1)]", fill, inert && "opacity-60")}
      style={{ width: `${ratio * 100}%` }}
    />
    {levels.map((level, i) => {
      const on = known && i <= index;
      return <button
        key={level.value}
        type="button"
        tabIndex={-1}
        aria-pressed={known && i === index}
        aria-label={`${modelLabel}: think ${level.label}`}
        disabled={inert}
        data-effort={level.value}
        data-active={known && i === index}
        onClick={() => commit(i)}
        className="absolute top-1/2 grid size-5 -translate-x-1/2 -translate-y-1/2 place-items-center disabled:cursor-default"
        style={{ left: `${last > 0 ? (i / last) * 100 : 0}%` }}
      >
        {/* Marks at or below the value light up in order, 30ms apart. */}
        <span aria-hidden="true" className={cn("block size-1.5 rounded-full transition-[background-color,transform] duration-200", on ? cn(fill, "scale-100") : "scale-75 bg-faint-2")} style={{ transitionDelay: `${on ? i * 30 : 0}ms` }} />
        <span className="sr-only">{level.label}</span>
      </button>;
    })}
    <div
      role="slider"
      tabIndex={inert ? -1 : 0}
      aria-label="Reasoning effort"
      aria-valuemin={0}
      aria-valuemax={last}
      aria-valuenow={known ? index : 0}
      aria-valuetext={known ? levels[index]?.label : "Default"}
      aria-disabled={inert || undefined}
      onKeyDown={event => {
        if (inert) return;
        const step = event.key === "ArrowRight" || event.key === "ArrowUp" ? 1 : event.key === "ArrowLeft" || event.key === "ArrowDown" ? -1 : 0;
        // From "Default" the first step lands on the first level.
        if (step) { event.preventDefault(); commit(known ? index + step : 0); }
        if (event.key === "Home") { event.preventDefault(); commit(0); }
        if (event.key === "End") { event.preventDefault(); commit(last); }
      }}
      className="absolute top-1/2 -translate-x-1/2 -translate-y-1/2 outline-none transition-[left] duration-300 ease-[cubic-bezier(.2,.8,.2,1)]"
      style={{ left: `${ratio * 100}%` }}
    >
      <span key={pulse} className={cn("relative block size-3.5 rounded-full border-2 border-popover bg-foreground shadow-[0_1px_3px_rgba(0,0,0,0.35)]", pulse > 0 && "animate-effort-thumb")}>
        {/* Halo in the harness tint: the one accent the popover carries. */}
        <span aria-hidden="true" className={cn("absolute -inset-1.5 rounded-full opacity-0", fill, pulse > 0 && "animate-effort-halo")} />
      </span>
    </div>
  </div>;
}
