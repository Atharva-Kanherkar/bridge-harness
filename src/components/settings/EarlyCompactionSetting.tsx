import { EARLY_COMPACTION_DESCRIPTION, useEarlyCompaction } from "../../earlyCompaction";
import { SettingsGroup, SettingsRow, Switch } from "./kit";

/**
 * The global early compaction switch. A chat can opt out from its menu, but
 * no chat can turn the feature on. A chat cannot change this setting either.
 */
export function EarlyCompactionSetting() {
  const { view, error, busy, set } = useEarlyCompaction();
  return <SettingsGroup label="Context" note="Off by default">
    <SettingsRow
      anchor="early-compaction"
      label="Early compaction"
      description={EARLY_COMPACTION_DESCRIPTION}
      control={<Switch
        label="Early compaction"
        checked={view?.globalEnabled === true}
        disabled={busy || !view}
        onChange={next => set(next)}
      />}
    />
    {error && <SettingsRow label={<span role="alert" className="text-destructive">{error}</span>} />}
  </SettingsGroup>;
}
