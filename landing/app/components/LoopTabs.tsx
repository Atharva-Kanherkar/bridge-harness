"use client";

import { useRef, useState, type KeyboardEvent } from "react";
import MockEntry from "./MockEntry";
import { loopSteps } from "../content/loop";

export default function LoopTabs() {
  const [active, setActive] = useState(0);
  const tabRefs = useRef<(HTMLButtonElement | null)[]>([]);
  const step = loopSteps[active];

  function select(index: number) {
    const next = (index + loopSteps.length) % loopSteps.length;
    setActive(next);
    tabRefs.current[next]?.focus();
  }

  function onKeyDown(event: KeyboardEvent<HTMLButtonElement>, index: number) {
    const keys: Record<string, number> = {
      ArrowDown: index + 1,
      ArrowRight: index + 1,
      ArrowUp: index - 1,
      ArrowLeft: index - 1,
      Home: 0,
      End: loopSteps.length - 1,
    };
    if (!(event.key in keys)) return;
    event.preventDefault();
    select(keys[event.key]);
  }

  return (
    <div className="mt-12 grid min-w-0 gap-8 lg:grid-cols-[220px_minmax(0,1fr)]">
      <div
        role="tablist"
        aria-orientation="vertical"
        aria-label="The Bridge dev loop"
        className="flex gap-1 overflow-x-auto text-[13px] lg:flex-col lg:overflow-visible [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
      >
        {loopSteps.map((item, i) => (
          <button
            key={item.id}
            ref={(el) => {
              tabRefs.current[i] = el;
            }}
            type="button"
            role="tab"
            id={`loop-tab-${item.id}`}
            aria-selected={i === active}
            aria-controls={`loop-panel-${item.id}`}
            tabIndex={i === active ? 0 : -1}
            onClick={() => setActive(i)}
            onKeyDown={(event) => onKeyDown(event, i)}
            className={`whitespace-nowrap rounded-md px-3 py-2 text-left transition-colors lg:border-l lg:pl-4 ${
              i === active
                ? "bg-muted text-foreground lg:border-foreground lg:bg-transparent"
                : "text-muted-foreground hover:text-foreground lg:border-border"
            }`}
          >
            {item.tab}
          </button>
        ))}
      </div>

      <div
        key={step.id}
        id={`loop-panel-${step.id}`}
        role="tabpanel"
        aria-labelledby={`loop-tab-${step.id}`}
        className="grid min-w-0 gap-6 animate-fade-up motion-reduce:animate-none md:grid-cols-2"
      >
        <div>
          <h3 className="text-lg font-medium leading-7">{step.title}</h3>
          <p className="mt-3 text-[14px] leading-6 text-muted-foreground">{step.text}</p>
        </div>
        <div className="flex min-w-0 flex-col gap-3">
          {step.entries.map((entry, i) => (
            <MockEntry key={i} entry={entry} />
          ))}
        </div>
      </div>
    </div>
  );
}
