import { useEffect, useRef, useState } from "react";
import { checkForUpdate, getUpdateChannel, setUpdateChannel, UpdateCheckSupersededError, type UpdateInfo } from "../../updater";
import { SettingsGroup, SettingsPage, SettingsRow, Switch } from "./kit";

export function UpdatesPage({ availableUpdate, onUpdate }: { availableUpdate?: UpdateInfo; onUpdate: (update: UpdateInfo | undefined) => void }) {
  const [channel, setChannel] = useState(getUpdateChannel);
  const [checking, setChecking] = useState(false);
  const [message, setMessage] = useState<string>();
  const checkId = useRef(0);

  useEffect(() => () => { ++checkId.current; }, []);
  useEffect(() => { if (availableUpdate) setMessage(undefined); }, [availableUpdate]);

  async function check(next = channel) {
    const id = ++checkId.current;
    setChecking(true);
    setMessage(undefined);
    onUpdate(undefined);
    try {
      const update = await checkForUpdate(next);
      if (id !== checkId.current || getUpdateChannel() !== next) return;
      onUpdate(update ?? undefined);
      if (!update) setMessage("Bridge is up to date on this channel.");
    } catch (error) {
      if (!(error instanceof UpdateCheckSupersededError) && id === checkId.current && getUpdateChannel() === next) setMessage(`Could not check for updates: ${String(error)}`);
    } finally { if (id === checkId.current) setChecking(false); }
  }

  function change(beta: boolean) {
    const next = beta ? "beta" : "stable";
    setUpdateChannel(next);
    setChannel(next);
    void check(next);
  }

  return <SettingsPage title="Updates" description="Choose which Bridge builds you receive and check for a new one.">
    <SettingsGroup label="Channel">
      <SettingsRow label="Beta nightly builds" description="Get the latest signed nightly build when one is available. Nightlies may be less stable. Turn this off to follow published releases again."
        control={<Switch label="Beta nightly builds" checked={channel === "beta"} onChange={change} />} />
    </SettingsGroup>
    <SettingsGroup label="Check">
      <SettingsRow label="Check for updates" description={availableUpdate?.channel === channel
        ? `Bridge ${availableUpdate.version} is available on this channel.`
        : message ?? `Current channel: ${channel === "beta" ? "beta nightly" : "stable"}. Bridge also checks when it opens.`}
        control={<button type="button" disabled={checking} onClick={() => void check()} className="rounded-lg border border-border px-3 py-1.5 text-xs text-foreground hover:bg-accent disabled:opacity-50">{checking ? "Checking…" : "Check now"}</button>} />
    </SettingsGroup>
  </SettingsPage>;
}
