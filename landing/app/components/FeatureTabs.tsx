"use client";

import { useCallback, useRef, useState, type KeyboardEvent } from "react";
import AppMockup from "./AppMockup";
import { scenes } from "../content/scenes";

export default function FeatureTabs() {
  const [active, setActive] = useState(0);
  const [replay, setReplay] = useState(0);
  const tabRefs = useRef<(HTMLButtonElement | null)[]>([]);
  const held = useRef(false);
  const scene = scenes[active];

  const onCycleEnd = useCallback(() => {
    if (held.current) {
      setReplay((value) => value + 1);
      return;
    }
    setActive((index) => (index + 1) % scenes.length);
  }, []);

  function select(index: number) {
    const next = (index + scenes.length) % scenes.length;
    setActive(next);
    tabRefs.current[next]?.focus();
  }

  function onKeyDown(event: KeyboardEvent<HTMLButtonElement>, index: number) {
    if (event.key === "ArrowRight") {
      event.preventDefault();
      select(index + 1);
    } else if (event.key === "ArrowLeft") {
      event.preventDefault();
      select(index - 1);
    } else if (event.key === "Home") {
      event.preventDefault();
      select(0);
    } else if (event.key === "End") {
      event.preventDefault();
      select(scenes.length - 1);
    }
  }

  return (
    <>
      <div className="mt-20 w-full overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
        <div role="tablist" aria-label="What Bridge does" className="mx-auto flex w-max rounded-lg border border-border bg-card p-1 text-[13px]">
          {scenes.map((s, i) => (
            <button
              key={s.id}
              ref={(el) => {
                tabRefs.current[i] = el;
              }}
              type="button"
              role="tab"
              id={`tab-${s.id}`}
              aria-selected={i === active}
              aria-controls={`panel-${s.id}`}
              tabIndex={i === active ? 0 : -1}
              onClick={() => setActive(i)}
              onKeyDown={(event) => onKeyDown(event, i)}
              className={`whitespace-nowrap rounded-md px-3 py-1.5 transition-colors ${i === active ? "bg-muted text-foreground" : "text-muted-foreground hover:text-foreground"}`}
            >
              {s.tab}
            </button>
          ))}
        </div>
      </div>
      <p key={scene.id} className="mt-5 max-w-2xl text-[15px] leading-7 text-muted-foreground animate-fade-up motion-reduce:animate-none">
        {scene.blurb}
      </p>
      <div
        id={`panel-${scene.id}`}
        role="tabpanel"
        aria-labelledby={`tab-${scene.id}`}
        className="mt-6 w-full"
        onPointerEnter={() => {
          held.current = true;
        }}
        onPointerLeave={() => {
          held.current = false;
        }}
        onFocusCapture={() => {
          held.current = true;
        }}
        onBlurCapture={() => {
          held.current = false;
        }}
      >
        <AppMockup key={`${scene.id}:${replay}`} scene={scene} onCycleEnd={onCycleEnd} />
      </div>
    </>
  );
}
