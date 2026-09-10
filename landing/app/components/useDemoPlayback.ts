"use client";

import { useEffect, useLayoutEffect, useRef, useState } from "react";

const useIsomorphicLayoutEffect = typeof window === "undefined" ? useEffect : useLayoutEffect;

export type Playback = { typed: string; shown: number; playing: boolean };

export function useDemoPlayback(
  sceneId: string,
  promptText: string,
  entryCount: number,
  onCycleEnd?: () => void,
): Playback {
  const [playing, setPlaying] = useState(false);
  const [run, setRun] = useState(0);
  const [typed, setTyped] = useState("");
  const cycleEnd = useRef(onCycleEnd);
  const [shown, setShown] = useState(entryCount);

  useEffect(() => {
    cycleEnd.current = onCycleEnd;
  }, [onCycleEnd]);

  useIsomorphicLayoutEffect(() => {
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    setTyped("");
    setShown(0);
    setRun(0);
    setPlaying(true);
  }, [sceneId]);

  useEffect(() => {
    if (!playing) return;

    const timers: number[] = [];
    const at = (delay: number, action: () => void) => timers.push(window.setTimeout(action, delay));

    at(0, () => {
      setTyped("");
      setShown(0);
    });

    const characters = promptText.length;
    const perCharacter = Math.max(9, Math.min(26, 1500 / Math.max(characters, 1)));
    let cursor = 500;

    for (let i = 1; i <= characters; i += 1) at(cursor + i * perCharacter, () => setTyped(promptText.slice(0, i)));
    cursor += characters * perCharacter + 460;

    at(cursor, () => {
      setTyped("");
      setShown(1);
    });
    cursor += 780;

    for (let i = 2; i <= entryCount; i += 1) {
      at(cursor, () => setShown(i));
      cursor += 1000;
    }

    at(cursor + 4200, () => {
      if (cycleEnd.current) cycleEnd.current();
      else setRun((value) => value + 1);
    });

    return () => timers.forEach(clearTimeout);
  }, [sceneId, playing, run, promptText, entryCount]);

  return { typed, shown, playing };
}
