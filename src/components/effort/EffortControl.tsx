// The thinking-effort control: one segmented row, one segment per level the
// model lists. A click picks a level; arrows, Home and End move and pick.
// Segments share the row by their natural width, so Codex's six levels and a
// long name like Ultracode fit the picker without squeezing the short ones.

import { SegmentedRadio } from "@/components/ui/segmented";
import { effortIndex, effortMeaning, type EffortControlProps } from "./effortLevels";

export function EffortControl({ levels, value, onChange, disabled }: EffortControlProps) {
  const index = effortIndex(levels, value);
  return <SegmentedRadio
    label="Reasoning effort"
    className="w-full"
    itemClassName="min-w-0 flex-auto"
    value={index < 0 ? undefined : levels[index].value}
    options={levels.map(level => ({
      value: level.value,
      label: <span className="truncate">{level.label}</span>,
      hint: effortMeaning(level.value) || undefined,
      data: { "data-effort": level.value },
    }))}
    onChange={onChange}
    disabled={disabled}
  />;
}
