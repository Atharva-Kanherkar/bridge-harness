import { NOTIFY_LEVELS, useDelegationNotify } from "../../delegationNotify";
import { SettingsGroup, SettingsRow } from "./kit";

export function DelegationNotifySetting() {
  const { view, error, set } = useDelegationNotify();
  const level = view?.globalLevel ?? "all";
  return <SettingsGroup label="Orchestrator notices" note="Choose which worker notices cost an orchestrator a model turn. Approval cards and transcript entries for you are unchanged. A chat can override this from its menu.">
    <SettingsRow label="Notify the orchestrator about" description={NOTIFY_LEVELS.find(item => item.value === level)?.description} control={
      <div className="u-segmented flex" role="group" aria-label="Orchestrator notices">
        {NOTIFY_LEVELS.map(item => <button key={item.value} type="button" className="u-segmented-item" disabled={!view} data-active={level === item.value} aria-pressed={level === item.value} onClick={() => set(item.value)}>{item.label}</button>)}
      </div>
    } />
    {error && <p role="alert" className="px-4 py-2 text-xs text-destructive">{error}</p>}
  </SettingsGroup>;
}
