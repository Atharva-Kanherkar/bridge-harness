import { useEffect, useState } from "react";
import { LoaderCircle as CircleNotch } from "lucide-react";
import { bridgeApi } from "../../api";
import type { CloneSettings, CloneSettingsSnapshot, CloneSignInPath } from "../../types";
import { Select, SettingsGroup, SettingsPage, SettingsRow, useSavedFlash, type SelectOption } from "./kit";

// Clones: the two defaults a new throwaway browser starts from. Both controls
// are selects, so each change persists on the spot and the page has no save
// bar, like Work briefing. The values are read and written through the mock
// api until the protocol slice lands; a build with no clone backend says so and
// disables the controls rather than accepting a write nothing would honour.

const SIGN_IN_OPTIONS: SelectOption[] = [
  { value: "import", label: "Import from my browser", description: "Copy the cookies for an approved domain into the clone" },
  { value: "sign_in_inside", label: "Sign in inside the clone", description: "Start blank and sign in yourself, so nothing is copied" },
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
    return <SettingsPage title="Clones">
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
    title="Clones"
    description="Throwaway copies of your browser an agent uses for work that needs a signed-in site. A clone never touches your own tabs."
  >
    <SettingsGroup label="New clones" note={connected ? undefined : "Not connected"}>
      {!connected && <SettingsRow label="Browser clones are not connected to the runtime in this build yet." description="These settings apply once they are." />}
      <SettingsRow
        label="Default sign-in path"
        description="How a new clone gets signed in."
        saved={isFlashed("signIn")}
        control={<Select
          label="Default sign-in path"
          value={settings.defaultSignInPath}
          disabled={busy || !connected}
          options={SIGN_IN_OPTIONS}
          onChange={value => void persist({ ...settings, defaultSignInPath: value as CloneSignInPath }, "signIn")}
        />}
      />
      <SettingsRow
        label="Time to live"
        description="A clone destroys itself, profile and cookies included, when this runs out."
        saved={isFlashed("ttl")}
        control={<Select
          label="Clone time to live"
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
