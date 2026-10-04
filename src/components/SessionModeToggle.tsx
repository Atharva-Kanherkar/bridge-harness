import type { KeyboardEvent } from "react";
import { useRef } from "react";
import { MessageSquare, Network } from "lucide-react";
import { cn } from "@/lib/utils";
import type { WorkspaceSessionKind } from "../types";

export const SESSION_MODES: ReadonlyArray<{ id: WorkspaceSessionKind; label: string; description: string; icon: typeof Network }> = [
  { id: "orchestrator", label: "Orchestrator", description: "Bridge plans the work and delegates to workers.", icon: Network },
  { id: "direct", label: "Direct", description: "Talk to the selected harness directly, with no Bridge orchestration.", icon: MessageSquare },
];

export function sessionModeDescription(mode: WorkspaceSessionKind): string {
  return SESSION_MODES.find(item => item.id === mode)!.description;
}

/** How a new workspace chat runs. Chosen once, before the first message: the
 *  session keeps its kind for every later turn. */
export function SessionModeToggle({ value, onChange, disabled, describedBy }: {
  value: WorkspaceSessionKind;
  onChange: (mode: WorkspaceSessionKind) => void;
  disabled?: boolean;
  /** Id of the visible explanation of the active mode. */
  describedBy?: string;
}) {
  const buttons = useRef<Array<HTMLButtonElement | null>>([]);
  const select = (index: number) => {
    const next = SESSION_MODES[(index + SESSION_MODES.length) % SESSION_MODES.length];
    onChange(next.id);
    buttons.current[SESSION_MODES.indexOf(next)]?.focus();
  };
  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    if (event.key === "ArrowRight" || event.key === "ArrowDown") { event.preventDefault(); select(index + 1); }
    else if (event.key === "ArrowLeft" || event.key === "ArrowUp") { event.preventDefault(); select(index - 1); }
  };
  return <div role="radiogroup" aria-label="Chat mode" aria-describedby={describedBy} className="inline-flex h-8 shrink-0 items-center gap-0.5 rounded-lg border border-border bg-background p-0.5">
    {SESSION_MODES.map((mode, index) => {
      const checked = mode.id === value;
      const Icon = mode.icon;
      return <button
        key={mode.id}
        ref={node => { buttons.current[index] = node; }}
        type="button"
        role="radio"
        aria-checked={checked}
        tabIndex={checked ? 0 : -1}
        disabled={disabled}
        title={mode.description}
        onClick={() => onChange(mode.id)}
        onKeyDown={event => onKeyDown(event, index)}
        className={cn(
          "inline-flex h-6 items-center gap-1.5 rounded-md px-2 text-[12px] font-medium transition-colors disabled:opacity-50",
          checked ? "bg-accent text-foreground shadow-control" : "text-muted-foreground hover:text-foreground",
        )}
      >
        <Icon className="h-3.5 w-3.5" strokeWidth={1.75} aria-hidden="true" />
        {mode.label}
      </button>;
    })}
  </div>;
}
