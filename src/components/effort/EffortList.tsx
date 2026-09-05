// The list style: effort as a vertical radio list with one line of meaning per
// level. It lives in the picker's right pane, so extra levels cost height
// rather than width and nothing is ever squeezed. Digits jump straight to a
// level while the list has focus.

import { cn } from "@/lib/utils";
import { effortIndex, effortMeaning, type EffortControlProps } from "./effortLevels";

export function EffortList({ levels, value, onChange, disabled }: EffortControlProps) {
  const index = effortIndex(levels, value);
  const inert = disabled || !onChange;
  return <div
    role="radiogroup"
    aria-label="Reasoning effort"
    data-effort-style="list"
    className="flex flex-col gap-0.5 outline-none"
    onKeyDown={event => {
      if (inert || !/^[1-9]$/.test(event.key)) return;
      const level = levels[Number(event.key) - 1];
      if (level) { event.preventDefault(); onChange?.(level.value); }
    }}
  >
    {levels.map((level, i) => {
      const on = i === index;
      const meaning = effortMeaning(level.value);
      return <button
        key={level.value}
        type="button"
        role="radio"
        aria-checked={on}
        aria-pressed={on}
        disabled={inert}
        data-effort={level.value}
        data-active={on}
        onClick={() => onChange?.(level.value)}
        className={cn(
          "grid w-full grid-cols-[14px_1fr_auto] items-center gap-2 rounded-[7px] px-2 py-1.5 text-left transition-colors disabled:cursor-default",
          on ? "bg-accent" : "hover:bg-accent",
        )}
      >
        <span aria-hidden="true" className={cn("grid size-3 place-items-center rounded-full border", on ? "border-foreground" : "border-faint")}>
          {on && <span className="size-1.5 rounded-full bg-foreground" />}
        </span>
        <span className="min-w-0">
          <span className="block text-[12px] font-medium text-foreground">{level.label}</span>
          {meaning && <span className="block text-[10.5px] text-muted-foreground">{meaning}</span>}
        </span>
        {i < 9 && <kbd aria-hidden="true" className="rounded border border-border px-1 font-mono text-[9.5px] text-faint">{i + 1}</kbd>}
      </button>;
    })}
  </div>;
}
