// The default thinking control: a "Thinking" header with one label at a time,
// and the shared rail beneath it.

import { useEffect, useRef, useState } from "react";
import { cn } from "@/lib/utils";
import { EffortRail } from "./EffortRail";
import { effortIndex, type EffortControlProps } from "./effortLevels";

export function EffortSlider(props: EffortControlProps) {
  const { levels, value } = props;
  const index = effortIndex(levels, value);
  const label = index >= 0 ? levels[index].label : "Default";
  // Fades the label in on every change, not on first paint.
  const [pulse, setPulse] = useState(0);
  const first = useRef(true);
  useEffect(() => {
    if (first.current) { first.current = false; return; }
    setPulse(count => count + 1);
  }, [label]);
  return <div className="flex h-full flex-col justify-between" data-effort-style="slider">
    <div className="flex items-baseline justify-between text-[11px] text-muted-foreground">
      <span>Thinking</span>
      <span key={pulse} className={cn("min-w-[6ch] text-right font-medium tabular-nums text-foreground", pulse > 0 && "animate-effort-label")}>{label}</span>
    </div>
    <EffortRail {...props} />
  </div>;
}
