// The sentence style: the footer reads as one line of prose and the adverb is
// the control. "Think deeply with Sonnet 5." Drag the word sideways to scrub,
// click to step, or use the arrow keys. The letters scramble for a beat and
// settle — motion that confirms a change without moving a single box.

import { useEffect, useRef, useState } from "react";
import { cn } from "@/lib/utils";
import { harnessFillClass } from "../harnessMarks";
import { effortIndex, effortWord, type EffortControlProps } from "./effortLevels";

const ALPHABET = "abcdefghijklmnopqrstuvwxyz";
const SCRAMBLE_MS = 260;
const SCRAMBLE_FRAME_MS = 28;
/** Pixels of drag per level, tuned so the whole ladder fits under one wrist. */
const SCRUB_PX = 28;

function reducedMotion(): boolean {
  return typeof window !== "undefined" && typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

/** The word as currently drawn: settles to `target` within SCRAMBLE_MS. */
function useScramble(target: string): { text: string; settled: boolean } {
  const [state, setState] = useState({ text: target, settled: true });
  const previous = useRef(target);
  useEffect(() => {
    if (previous.current === target) return;
    previous.current = target;
    if (reducedMotion()) { setState({ text: target, settled: true }); return; }
    const started = performance.now();
    let timer: ReturnType<typeof setTimeout> | undefined;
    const frame = () => {
      const progress = Math.min(1, (performance.now() - started) / SCRAMBLE_MS);
      const keep = Math.floor(progress * target.length);
      const text = [...target].map((char, i) => i < keep || char === " " ? char : ALPHABET[Math.floor(Math.random() * ALPHABET.length)]).join("");
      if (progress < 1) { setState({ text, settled: false }); timer = setTimeout(frame, SCRAMBLE_FRAME_MS); }
      else setState({ text: target, settled: true });
    };
    frame();
    return () => { if (timer) clearTimeout(timer); };
  }, [target]);
  return state;
}

export function EffortSentence({ levels, value, onChange, disabled, harness, modelLabel }: EffortControlProps) {
  // Off the ladder (unset: the provider's own default) reads as "normally"
  // with nothing lit; the first click steps onto the first level.
  const known = effortIndex(levels, value) >= 0;
  const index = Math.max(0, effortIndex(levels, value));
  const last = levels.length - 1;
  const inert = disabled || !onChange;
  const fill = harnessFillClass(harness);
  const current = known ? levels[index] : undefined;
  const { text, settled } = useScramble(current ? effortWord(current.value) : "normally");
  const drag = useRef<{ startX: number; startIndex: number; moved: boolean } | null>(null);

  const commit = (next: number) => {
    const clamped = Math.min(last, Math.max(0, next));
    const level = levels[clamped];
    if (!inert && level && level.value !== value) onChange?.(level.value);
  };

  return <div className="flex h-full flex-col justify-between" data-effort-style="sentence">
    <div className="flex items-baseline justify-between text-[11px] text-muted-foreground">
      <span>Thinking</span>
      <span className="min-w-[6ch] text-right font-mono text-[10px] text-faint">{current?.value ?? ""}</span>
    </div>
    <p className="flex items-center gap-1.5 whitespace-nowrap text-[13px] leading-6 text-muted-foreground">
      <span>Think</span>
      <button
        type="button"
        disabled={inert}
        aria-label={`Reasoning effort: ${current?.label ?? ""}. Click for the next level, drag to scrub.`}
        data-effort={current?.value}
        data-active="true"
        aria-pressed="true"
        onPointerDown={event => {
          if (inert) return;
          drag.current = { startX: event.clientX, startIndex: index, moved: false };
          // Capture keeps the scrub alive past the button's edge. Optional
          // because not every DOM (jsdom) implements it.
          event.currentTarget.setPointerCapture?.(event.pointerId);
        }}
        onPointerMove={event => {
          const state = drag.current;
          if (!state) return;
          const steps = Math.round((event.clientX - state.startX) / SCRUB_PX);
          if (Math.abs(event.clientX - state.startX) > 4) state.moved = true;
          if (state.startIndex + steps !== index || !known) commit(state.startIndex + steps);
        }}
        onPointerUp={() => {
          const state = drag.current;
          drag.current = null;
          if (state && !state.moved) commit(!known ? 0 : index === last ? 0 : index + 1);
        }}
        onPointerCancel={() => { drag.current = null; }}
        onKeyDown={event => {
          if (inert) return;
          if (event.key === "ArrowRight" || event.key === "ArrowUp") { event.preventDefault(); commit(index + 1); }
          if (event.key === "ArrowLeft" || event.key === "ArrowDown") { event.preventDefault(); commit(index - 1); }
          if (event.key === "Enter" || event.key === " ") { event.preventDefault(); commit(!known ? 0 : index === last ? 0 : index + 1); }
        }}
        className={cn(
          "min-w-[12ch] touch-none select-none rounded-md border border-border bg-muted px-2 text-center font-semibold tabular-nums text-foreground transition-colors",
          inert ? "cursor-default" : "cursor-ew-resize hover:border-foreground/40",
          !settled && "text-muted-foreground",
        )}
      >{text}</button>
      <span className="min-w-0 truncate">with {modelLabel}.</span>
    </p>
    <div className="flex h-1.5 gap-1" aria-hidden="true">
      {levels.map((level, i) => <span key={level.value} className={cn("min-w-0 flex-1 rounded-[2px] transition-[background-color,opacity] duration-200", known && i <= index ? fill : "bg-faint-2 opacity-40")} style={{ transitionDelay: `${known && i <= index ? i * 30 : 0}ms` }} />)}
    </div>
  </div>;
}
