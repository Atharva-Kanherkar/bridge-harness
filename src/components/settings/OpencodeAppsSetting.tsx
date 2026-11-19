import { useEffect, useState } from "react";
import { TriangleAlert } from "lucide-react";
import { bridgeApi } from "../../api";
import { Button } from "../ui/button";
import { SettingsGroup, SettingsRow, Switch } from "./kit";

export const OPENCODE_APPS_WARNING = "OpenCode reaches MCP servers only through its code mode, which can also make network requests Bridge cannot see or approve. Bridge still lets OpenCode make only the exact MCP call you approved, but it cannot stop other network traffic from that hidden session. Turn this on only for servers you trust, at your own discretion.";

/**
 * The person's own opt-in for personal apps through OpenCode. Off by default;
 * turning it on takes a second, explicit confirmation. A chat cannot change it.
 */
export function OpencodeAppsSetting() {
  const [allowed, setAllowed] = useState<boolean>();
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    let current = true;
    void bridgeApi.getAppPreferences().then(prefs => { if (current) setAllowed(prefs.values.allowOpencodeAppCourier === true); }, () => { if (current) setAllowed(false); });
    return () => { current = false; };
  }, []);
  const save = async (next: boolean) => {
    setBusy(true); setError("");
    try {
      const saved = await bridgeApi.saveAppPreferences({ allowOpencodeAppCourier: next });
      setAllowed(saved.values.allowOpencodeAppCourier === true);
      setConfirming(false);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };
  return <SettingsGroup label="Personal apps" note="Off by default">
    <SettingsRow
      label="Allow personal apps through OpenCode (less safe)"
      description={OPENCODE_APPS_WARNING}
      control={<Switch
        label="Allow personal apps through OpenCode"
        checked={allowed === true}
        disabled={busy || allowed === undefined}
        onChange={next => { if (next) setConfirming(true); else void save(false); }}
      />}
    />
    {confirming && <SettingsRow
      lead={<TriangleAlert size={12} aria-hidden="true" />}
      label="Turn on personal apps through OpenCode?"
      description="You accept that an app running through OpenCode may make network requests Bridge cannot approve."
      control={<div className="flex gap-2">
        <Button size="sm" variant="ghost" disabled={busy} onClick={() => setConfirming(false)}>Cancel</Button>
        <Button size="sm" disabled={busy} onClick={() => void save(true)}>I understand, turn on</Button>
      </div>}
    />}
    {error && <SettingsRow label={<span role="alert" className="text-destructive">{error}</span>} />}
  </SettingsGroup>;
}
