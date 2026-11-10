import { ATTENTION_KINDS, useAttentionSettings, writeAttentionSettings, type AttentionSettingKind, type Channels } from "../../attentionSettings";
import { playAttentionSound } from "../../attentionSound";
import { Field, GhostButton, SettingsGroup, SettingsPage, SettingsRow, Switch } from "./kit";

const KIND_COPY: Record<AttentionSettingKind, { label: string; description: string }> = {
  needsYou: { label: "Needs you", description: "A chat or one of its agents is waiting for an answer or an approval." },
  posted: { label: "An agent asks for you", description: "A chat's agent needs a decision or is blocked, in its own words." },
  escalation: { label: "A result is waiting", description: "An agent stopped before it read a worker's update." },
  failed: { label: "Turn failed", description: "A chat's turn ended with an error." },
  finished: { label: "Turn completed", description: "A chat finished its turn." },
};

const CHANNELS: { key: keyof Channels; label: string }[] = [
  { key: "toast", label: "Toast" },
  { key: "banner", label: "Banner" },
  { key: "sound", label: "Sound" },
];

/** Which notices reach you, and how. Nothing is shown for the chat you are looking at. */
export function NotificationsPage() {
  const settings = useAttentionSettings();
  const setChannel = (kind: AttentionSettingKind, channel: keyof Channels, value: boolean) => {
    writeAttentionSettings({ ...settings, kinds: { ...settings.kinds, [kind]: { ...settings.kinds[kind], [channel]: value } } });
  };
  const setQuiet = (change: Partial<typeof settings.quietHours>) => {
    writeAttentionSettings({ ...settings, quietHours: { ...settings.quietHours, ...change } });
  };
  return <SettingsPage title="Notifications" description="How Bridge tells you about your chats. A toast appears in the window, a banner from macOS while Bridge is in the background. Nothing is shown for the chat you are looking at.">
    <SettingsGroup label="Notices">
      {ATTENTION_KINDS.map(kind => <SettingsRow
        key={kind}
        label={KIND_COPY[kind].label}
        description={KIND_COPY[kind].description}
        control={<div className="flex items-center gap-3">
          {CHANNELS.map(channel => <span key={channel.key} className="flex items-center gap-1.5 text-xs text-muted-foreground">
            <span aria-hidden="true">{channel.label}</span>
            <Switch
              checked={settings.kinds[kind][channel.key]}
              onChange={value => setChannel(kind, channel.key, value)}
              label={`${channel.label} for ${KIND_COPY[kind].label}`}
            />
          </span>)}
          <GhostButton ariaLabel={`Play the ${KIND_COPY[kind].label} sound`} onClick={() => playAttentionSound(kind)}>Play</GhostButton>
        </div>}
      />)}
    </SettingsGroup>
    <SettingsGroup label="Quiet hours" note="No banners or sounds between these times. Toasts still appear in the window.">
      <SettingsRow
        label="Quiet hours"
        description={settings.quietHours.enabled ? `From ${settings.quietHours.start} to ${settings.quietHours.end}` : "Off"}
        control={<Switch checked={settings.quietHours.enabled} onChange={enabled => setQuiet({ enabled })} label="Quiet hours" />}
      />
      <SettingsRow
        label="From"
        disabled={!settings.quietHours.enabled}
        control={<Field type="time" width="w-28" value={settings.quietHours.start} disabled={!settings.quietHours.enabled} label="Quiet hours start" onChange={start => start && setQuiet({ start })} />}
      />
      <SettingsRow
        label="Until"
        disabled={!settings.quietHours.enabled}
        control={<Field type="time" width="w-28" value={settings.quietHours.end} disabled={!settings.quietHours.enabled} label="Quiet hours end" onChange={end => end && setQuiet({ end })} />}
      />
    </SettingsGroup>
  </SettingsPage>;
}
