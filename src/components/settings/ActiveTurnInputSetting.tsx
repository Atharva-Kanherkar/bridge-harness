import { useActiveTurnInput, writeActiveTurnInput } from "../../activeTurnSettings";
import { SettingsGroup, SettingsRow } from "./kit";

export function ActiveTurnInputSetting({ onError }: { onError: (message: string) => void }) {
  const value = useActiveTurnInput();
  return <SettingsGroup label="Messages during a turn">
    <SettingsRow label="Active-turn behavior" description="Applies to all chats. Steer sends guidance into the live turn. Queue holds your message until the current step finishes." control={
      <div className="u-segmented flex" role="group" aria-label="Active-turn behavior">
        {(["steer", "queue"] as const).map(mode => <button key={mode} type="button" className="u-segmented-item" data-active={value === mode} aria-pressed={value === mode} onClick={() => {
          try { writeActiveTurnInput(mode); } catch (error) { onError(String(error)); }
        }}>{mode === "steer" ? "Steer" : "Queue"}</button>)}
      </div>
    } />
    <p className="px-4 py-2 text-xs text-muted-foreground">If a provider cannot steer a live turn, Bridge queues the message instead. Images cannot be queued; send them after the turn finishes.</p>
  </SettingsGroup>;
}
