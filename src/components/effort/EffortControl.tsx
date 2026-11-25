// The thinking-effort control: one segmented row, one segment per level the
// model lists. A click picks a level; arrows, Home and End move and pick.
// Segments share the row by their natural width, so Codex's six levels and a
// long name like Ultracode fit the picker without squeezing the short ones.

import { cn } from "@/lib/utils";
import { effortIndex, effortMeaning, type EffortControlProps } from "./effortLevels";

export function EffortControl({ levels, value, onChange, disabled }: EffortControlProps) {
  const index = effortIndex(levels, value);
  const inert = disabled || !onChange;
  return <div role="radiogroup" aria-label="Reasoning effort" className="flex w-full gap-0.5 rounded-md bg-muted p-0.5 text-caption">
    {levels.map((level, i) => {
      const on = i === index;
      return <button
        key={level.value}
        type="button"
        role="radio"
        aria-checked={on}
        // Roving focus: the current level, or the first when none is set.
        tabIndex={on || (index < 0 && i === 0) ? 0 : -1}
        disabled={inert}
        data-effort={level.value}
        title={effortMeaning(level.value) || undefined}
        onClick={() => { if (!on) onChange?.(level.value); }}
        onKeyDown={event => {
          if (inert) return;
          const step = event.key === "ArrowRight" || event.key === "ArrowDown" ? 1 : event.key === "ArrowLeft" || event.key === "ArrowUp" ? -1 : 0;
          if (!step && event.key !== "Home" && event.key !== "End") return;
          event.preventDefault();
          // Arrows wrap, as in any radio group; from no level they land on
          // the first or the last.
          const next = event.key === "Home" ? 0
            : event.key === "End" ? levels.length - 1
            : index < 0 ? (step > 0 ? 0 : levels.length - 1)
            : (i + step + levels.length) % levels.length;
          if (next !== index) onChange?.(levels[next].value);
          event.currentTarget.parentElement?.querySelectorAll<HTMLButtonElement>('[role="radio"]')[next]?.focus();
        }}
        className={cn(
          "h-7 min-w-0 flex-auto truncate rounded px-2 font-medium outline-none transition-colors focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-default",
          on ? "bg-card text-foreground shadow-xs" : "text-muted-foreground enabled:hover:text-foreground",
          inert && !on && "opacity-60",
        )}
      >{level.label}</button>;
    })}
  </div>;
}
