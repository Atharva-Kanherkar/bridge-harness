import { NOTIFY_LEVELS, useDelegationNotify } from "../../delegationNotify";
import { Segmented, SettingsGroup, SettingsRow } from "./kit";

export function DelegationNotifySetting() {
  const { view, error, set } = useDelegationNotify();
  const level = view?.globalLevel ?? "all";
  return <SettingsGroup anchor="orchestrator-notices" label="Orchestrator notices" note="Choose which worker notices cost an orchestrator a model turn. Approval cards and transcript entries for you are unchanged. A chat can override this from its menu.">
    <SettingsRow label="Notify the orchestrator about" description={NOTIFY_LEVELS.find(item => item.value === level)?.description} control={
      <Segmented label="Orchestrator notices" disabled={!view} value={level} options={NOTIFY_LEVELS} onChange={next => set(next)} />
    } />
    {error && <p role="alert" className="px-3.5 py-2 text-caption text-destructive">{error}</p>}
  </SettingsGroup>;
}
