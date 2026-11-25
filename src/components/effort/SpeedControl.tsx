// The chat's speed: standard, or a faster service tier the selected model
// lists (Claude Code's fast mode, Codex's Fast and Ultrafast). A segmented
// radio group in the model picker's footer; it renders nothing for a model
// with no faster tier, so most pickers never show it.

import { Zap } from "lucide-react";
import { SegmentedRadio } from "@/components/ui/segmented";
import type { AdapterDescriptor } from "../../types";
import type { SpeedTier } from "../../protocol/generated/protocol";
import { Tip } from "@/components/ui/tooltip";

/** The wire value for going back to the provider's standard speed. */
export const STANDARD_SPEED = "standard";

/** The faster tiers a catalog lists for a model; `null` is the adapter's default. */
export function speedTiersOf(adapters: readonly Pick<AdapterDescriptor, "id" | "models" | "defaultModel">[], harness: string, model: string | null): SpeedTier[] {
  const adapter = adapters.find(item => item.id === harness);
  return adapter?.models.find(item => item.id === (model ?? adapter.defaultModel))?.speedTiers ?? [];
}

export type SpeedControlProps = {
  tiers: readonly SpeedTier[];
  /** The chat's speed. Null, `standard` or an id the model no longer lists
   *  all read as Standard. */
  value: string | null | undefined;
  onChange?: (speed: string) => void;
  disabled?: boolean;
};

export function SpeedControl({ tiers, value, onChange, disabled }: SpeedControlProps) {
  if (tiers.length === 0) return null;
  const options = [{ id: STANDARD_SPEED, label: "Standard", description: "The provider's normal speed and rate." as string | null | undefined }, ...tiers];
  const current = tiers.some(tier => tier.id === value) ? value : STANDARD_SPEED;
  const active = options.find(option => option.id === current);
  return <div className="flex min-w-0 flex-col gap-1" data-testid="speed-control">
    <div className="flex items-center">
      <SegmentedRadio
        label="Speed"
        value={current ?? undefined}
        options={options.map(option => ({
          value: option.id,
          label: <>{option.id !== STANDARD_SPEED && <Zap size={11} aria-hidden="true" className="shrink-0" />}{option.label}</>,
          hint: option.description ?? undefined,
          data: { "data-speed": option.id },
        }))}
        onChange={onChange}
        disabled={disabled}
      />
    </div>
    {active?.description && current !== STANDARD_SPEED && <Tip label={active.description}><p className="truncate text-caption leading-4 text-muted-foreground">{active.description}</p></Tip>}
  </div>;
}
