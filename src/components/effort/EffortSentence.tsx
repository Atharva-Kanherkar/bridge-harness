// The sentence style: the same rail as the slider, captioned by one line of
// prose instead of a bare level name. "Claude Opus thinks deeply." The adverb
// sits last so its width never shifts anything before it; on change its
// letters scramble for a beat and settle — motion that confirms the level
// without moving a single box.

import { useEffect, useRef, useState } from "react";
import { cn } from "@/lib/utils";
import { EffortRail } from "./EffortRail";
import { effortIndex, effortWord, type EffortControlProps } from "./effortLevels";

const ALPHABET = "abcdefghijklmnopqrstuvwxyz";
const SCRAMBLE_MS = 260;
const SCRAMBLE_FRAME_MS = 28;

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

export function EffortSentence(props: EffortControlProps) {
  const { levels, value, modelLabel } = props;
  const index = effortIndex(levels, value);
  const current = index >= 0 ? levels[index] : undefined;
  // Off the ladder (unset: the provider's own default) reads as "normally".
  const { text, settled } = useScramble(current ? effortWord(current.value) : "normally");
  return <div className="flex h-full flex-col justify-between" data-effort-style="sentence">
    <p className="flex items-baseline gap-1 whitespace-nowrap text-[12.5px] leading-5 text-muted-foreground">
      <span className="min-w-0 truncate">{modelLabel} thinks</span>
      <span data-testid="effort-word" className={cn("font-semibold text-foreground transition-colors", !settled && "text-muted-foreground")}>{text}</span>
      <span>.</span>
      <span className="ml-auto font-mono text-[10px] text-faint">{current?.value ?? ""}</span>
    </p>
    <EffortRail {...props} />
  </div>;
}
