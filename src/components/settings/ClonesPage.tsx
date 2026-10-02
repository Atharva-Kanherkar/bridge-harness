import { useEffect, useState } from "react";
import { LoaderCircle as CircleNotch } from "lucide-react";
import { bridgeApi } from "../../api";
import type { CloneSettings, CloneSettingsSnapshot, CloneSignInPath } from "../../types";
import { Select, SettingsGroup, SettingsPage, SettingsRow, Switch, useSavedFlash, type SelectOption } from "./kit";

// Clones: the defaults a new throwaway browser starts from. Every control
// persists on the spot, so the page has no save bar, like Work briefing. The
// values are read and written through the native API. A build without clone
// support disables the controls.

const SIGN_IN_OPTIONS: SelectOption[] = [
  { value: "import", label: "Signed in as you", description: "Copy the approved site's sign-in from Chrome into the browser copy" },
  { value: "sign_in_inside", label: "Blank browser", description: "Start without copied sign-in; sign in to the browser copy yourself" },
];

/** Lifetime choices. A stored value outside the list is shown, not dropped. */
const TTL_MINUTES = [10, 15, 30, 60, 120, 240];
const ttlLabel = (minutes: number) => minutes < 60 ? `${minutes} minutes` : minutes === 60 ? "1 hour" : `${minutes / 60} hours`;

export function ClonesPage({ onError }: { onError: (message: string) => void }) {
  const [snapshot, setSnapshot] = useState<CloneSettingsSnapshot>();
  const [busy, setBusy] = useState(false);
  const [isFlashed, flash] = useSavedFlash();

  useEffect(() => {
    let active = true;
    bridgeApi.readCloneSettings()
      .then(stored => { if (active) setSnapshot(stored); })
      .catch(error => onError(error instanceof Error ? error.message : String(error)));
    return () => { active = false; };
  }, [onError]);

  if (!snapshot) {
    return <SettingsPage title="Browser copies">
      <SettingsGroup>
        <SettingsRow label="Loading…" control={<CircleNotch size={12} strokeWidth={1.7} className="animate-spin text-muted-foreground" aria-hidden="true" />} />
      </SettingsGroup>
    </SettingsPage>;
  }

  const { settings, connected } = snapshot;
  const persist = async (next: CloneSettings, key: string) => {
    setBusy(true);
    try {
      setSnapshot(await bridgeApi.writeCloneSettings(next));
      flash(key);
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };
  const ttlOptions: SelectOption[] = [...new Set([...TTL_MINUTES, settings.ttlMinutes])]
    .sort((a, b) => a - b)
    .map(minutes => ({ value: String(minutes), label: ttlLabel(minutes) }));

  return <SettingsPage
    title="Browser copies"
    description="Agents can use a separate browser for signed-in work. You approve which sites it can access; your own tabs stay separate."
  >
    <SettingsGroup label="New browser copies" note={connected ? undefined : "Unavailable in this build"}>
      {!connected && <SettingsRow label="Browser copies are unavailable in this build." description="Bridge cannot create a browser copy in this build. These controls become available when browser copies are supported." />}
      <SettingsRow
        label="How to sign in"
        description="Choose whether to copy an approved site's sign-in or sign in yourself."
        saved={isFlashed("signIn")}
        control={<Select
          label="How to sign in"
          value={settings.defaultSignInPath}
          disabled={busy || !connected}
          options={SIGN_IN_OPTIONS}
          onChange={value => void persist({ ...settings, defaultSignInPath: value as CloneSignInPath }, "signIn")}
        />}
      />
      <SettingsRow
        label="Agent sees screenshots"
        description="The agent can look at the page, not only read its text. You can still turn it off for any one request."
        saved={isFlashed("vision")}
        control={<Switch
          label="Agent sees screenshots"
          checked={settings.agentVision ?? true}
          disabled={busy || !connected}
          onChange={agentVision => void persist({ ...settings, agentVision }, "vision")}
        />}
      />
      <SettingsRow
        label="Close after"
        description="The browser copy closes and deletes its temporary profile and cookies after this time."
        saved={isFlashed("ttl")}
        control={<Select
          label="Close browser copy after"
          value={String(settings.ttlMinutes)}
          disabled={busy || !connected}
          width="w-40"
          options={ttlOptions}
          onChange={value => void persist({ ...settings, ttlMinutes: Number(value) }, "ttl")}
        />}
      />
    </SettingsGroup>
  </SettingsPage>;
}
