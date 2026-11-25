import { useEffect, useMemo, useState } from "react";
import { useReducedMotion } from "framer-motion";
import { cn } from "@/lib/utils";
import type { GreetingPart } from "../greetings";

const START_MS = 280;
const HOLD_MS = 3400;
const FADE_MS = 320;

/** Character offsets where each LLM-sized token ends: short words land whole,
 *  long ones arrive in three-to-four character pieces, as a model streams them. */
export function tokenBoundaries(text: string): number[] {
  const ends: number[] = [];
  for (const match of text.matchAll(/\s*\S+/g)) {
    const start = match.index ?? 0;
    const word = match[0];
    const lead = word.length - word.trimStart().length;
    if (word.length - lead <= 6) {
      ends.push(start + word.length);
      continue;
    }
    for (let at = start + lead + 4; at < start + word.length; at += 3) ends.push(at);
    ends.push(start + word.length);
  }
  return ends;
}

/** Uneven per-token delay, slower after punctuation, the way streamed text stutters. */
export function tokenDelay(text: string, end: number, index: number): number {
  const base = 42 + ((index * 37) % 5) * 13;
  return /[,.?!]$/.test(text.slice(0, end)) ? base + 90 : base;
}

type Phase = "streaming" | "fading";

/**
 * A headline that streams in token by token like a model reply, holds, fades,
 * and streams the next line. Every character is laid out from the start and
 * only revealed, so wrapping never shifts as tokens land; a hidden copy of the
 * longest line keeps the height still between lines. Reduced motion shows the
 * first line whole and never rotates.
 */
export function StreamingGreeting({ lines, projectClassName }: { lines: GreetingPart[][]; projectClassName?: string }) {
  const reduced = useReducedMotion() ?? false;
  const [index, setIndex] = useState(0);
  const [token, setToken] = useState(0);
  const [phase, setPhase] = useState<Phase>("streaming");
  const line = lines[index % lines.length] ?? [];
  const text = line.map(part => part.text).join("");
  const ends = useMemo(() => tokenBoundaries(text), [text]);
  const longest = useMemo(() => lines.reduce<GreetingPart[]>((best, next) => (
    next.reduce((sum, part) => sum + part.text.length, 0) > best.reduce((sum, part) => sum + part.text.length, 0) ? next : best
  ), []), [lines]);

  useEffect(() => {
    if (reduced) return;
    let timer: ReturnType<typeof setTimeout>;
    if (phase === "streaming" && token < ends.length) {
      timer = setTimeout(() => setToken(token + 1), token === 0 ? START_MS : tokenDelay(text, ends[token - 1], token));
    } else if (phase === "streaming") {
      if (lines.length <= 1) return;
      timer = setTimeout(() => setPhase("fading"), HOLD_MS);
    } else {
      timer = setTimeout(() => {
        setIndex(current => (current + 1) % lines.length);
        setToken(0);
        setPhase("streaming");
      }, FADE_MS);
    }
    return () => clearTimeout(timer);
  }, [reduced, phase, token, ends, text, lines.length]);

  const shown = reduced ? text.length : token === 0 ? 0 : ends[Math.min(token, ends.length) - 1] ?? text.length;
  const streaming = !reduced && shown < text.length;

  // The caret sits where the reveal has reached, at the end of a part when the
  // reveal stops exactly on a boundary.
  let offset = 0;
  let caretPart = -1;
  const rendered = line.map((part, partIndex) => {
    const from = offset;
    offset += part.text.length;
    const visible = part.text.slice(0, Math.max(0, shown - from));
    const hidden = part.text.slice(visible.length);
    const caret = streaming && caretPart < 0 && shown >= from && shown <= offset;
    if (caret) caretPart = partIndex;
    const styled = part.kind === "project" ? projectClassName : undefined;
    return <span key={partIndex}>
      {visible && <span className={styled}>{visible}</span>}
      {caret && <Caret />}
      {hidden && <span className="invisible">{hidden}</span>}
    </span>;
  });

  return <span className="grid">
    <span className="sr-only">{text}</span>
    <span aria-hidden="true" className="invisible col-start-1 row-start-1">{longest.map(part => part.text).join("")}</span>
    <span aria-hidden="true" className={cn("col-start-1 row-start-1 transition-opacity duration-300", phase === "fading" && "opacity-0")}>
      {rendered}
    </span>
  </span>;
}

/** Zero-width so it never nudges the wrap; the bar hangs off its left edge. */
function Caret() {
  return <span className="relative inline-block w-0 align-baseline">
    <span className="animate-caret-blink absolute bottom-[-0.08em] left-[0.06em] h-[0.9em] w-[0.07em] rounded-full bg-current opacity-70" />
  </span>;
}
