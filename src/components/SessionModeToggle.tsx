import { MessageSquare, Network } from "lucide-react";
import { SegmentedRadio } from "@/components/ui/segmented";
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
  return <SegmentedRadio
    label="Chat mode"
    describedBy={describedBy}
    value={value}
    options={SESSION_MODES.map(mode => {
      const Icon = mode.icon;
      return { value: mode.id, label: <><Icon className="size-3" strokeWidth={1.75} aria-hidden="true" />{mode.label}</>, hint: mode.description };
    })}
    onChange={onChange}
    disabled={disabled}
  />;
}
