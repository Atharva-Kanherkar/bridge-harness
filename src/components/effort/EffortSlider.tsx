// The default thinking control: one rail, one thumb, one label at a time.
//
// Why a slider beats the old segmented row: its width does not depend on how
// many levels a model has. Claude's five and Codex's six draw into the same
// 72px footer; only the tick spacing differs. Every mark is an unlabeled
// button with a screen-reader label, so the footer's contract — one button
// per level carrying `data-effort` and `aria-pressed` — is unchanged.

import { useEffect, useRef, useState } from "react";
import { cn } from "@/lib/utils";
import { harnessFillClass } from "../harnessMarks";
import { effortIndex, type EffortControlProps } from "./effortLevels";

export function EffortSlider({ levels, value, onChange, disabled, harness, modelLabel }: EffortControlProps) {
  const index = Math.max(0, effortIndex(levels, value));
  const last = levels.length - 1;
  const ratio = last > 0 ? index / last : 0;
  const inert = disabled || !onChange;
  const rail = useRef<HTMLDivElement>(null);
  const fill = harnessFillClass(harness);
  // Replays the thumb pop and label fade on every change, not on first paint.
  const [pulse, setPulse] = useState(0);
  const first = useRef(true);
  useEffect(() => {
    if (first.current) { first.current = false; return; }
    setPulse(count => count + 1);
  }, [index]);

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

  return <div className="flex h-full flex-col justify-between" data-effort-style="slider">
    <div className="flex items-baseline justify-between text-[11px] text-muted-foreground">
      <span>Thinking</span>
      <span key={pulse} className={cn("min-w-[6ch] text-right font-medium tabular-nums text-foreground", pulse > 0 && "animate-effort-label")}>{levels[index]?.label ?? ""}</span>
    </div>
    <div
      ref={rail}
      className={cn("relative h-6 touch-none select-none", inert ? "cursor-default" : "cursor-pointer")}
      onPointerDown={event => {
        if (inert) return;
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
        const on = i <= index;
        return <button
          key={level.value}
          type="button"
          tabIndex={-1}
          aria-pressed={i === index}
          aria-label={`${modelLabel}: think ${level.label}`}
          disabled={inert}
          data-effort={level.value}
          data-active={i === index}
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
        aria-valuenow={index}
        aria-valuetext={levels[index]?.label}
        aria-disabled={inert || undefined}
        onKeyDown={event => {
          if (inert) return;
          const step = event.key === "ArrowRight" || event.key === "ArrowUp" ? 1 : event.key === "ArrowLeft" || event.key === "ArrowDown" ? -1 : 0;
          if (step) { event.preventDefault(); commit(index + step); }
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
    </div>
  </div>;
}
