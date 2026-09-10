import { useEffect, useState } from "react";
import { RefreshCw } from "lucide-react";
import { bridgeApi } from "../../api";
import type { MenuBarSettings, UsageOverviewSnapshot } from "../../protocol/generated/protocol";
import { SettingsPage, SettingsGroup, SettingsRow, Select, Switch } from "../../components/settings/kit";

export function MenuBarSettingsPage() {
  const [settings, setSettings] = useState<MenuBarSettings | null>(null);
  const [usage, setUsage] = useState<UsageOverviewSnapshot | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    let active = true;
    let off: (() => void) | undefined;
    // A failed usage read must not prevent disabling or changing the menu.
    void bridgeApi.getMenuBarSettings().then(settings => {
      if (active) setSettings(settings);
    }).catch(error => { if (active) setError(String(error)); });
    void bridgeApi.getUsageOverview().then(usage => {
      if (active) setUsage(usage);
    }).catch(error => { if (active) setError(String(error)); });
    void bridgeApi.onUsageOverview(value => { if (active) setUsage(value); }).then(unlisten => {
      if (active) off = unlisten; else unlisten();
    }).catch(error => { if (active) setError(String(error)); });
    return () => { active = false; off?.(); };
  }, []);

  async function save(patch: Partial<MenuBarSettings>) {
    if (!settings || busy) return;
    setBusy(true); setError(null); setSaved(false);
    try { setSettings(await bridgeApi.saveMenuBarSettings({ ...settings, ...patch })); setSaved(true); }
    catch (error) { setError(String(error)); }
    finally { setBusy(false); }
  }

  async function refresh() {
    setBusy(true); setError(null); setSaved(false);
    try { setUsage(await bridgeApi.refreshUsageOverview()); }
    catch (error) { setError(String(error)); }
    finally { setBusy(false); }
  }

  return <SettingsPage title="Menu Bar" description="Account limits, tokens, and spend at a glance."
    action={<button type="button" disabled={busy || !settings?.codexEnabled} onClick={() => void refresh()}
      className="flex items-center gap-2 rounded-lg border border-border px-3 py-1.5 text-xs text-foreground hover:bg-accent disabled:opacity-40">
      <RefreshCw size={12} aria-hidden="true" className={busy ? "animate-spin" : ""} />Refresh usage
    </button>}>
    {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
    {saved && <p role="status" className="text-xs text-muted-foreground">Menu Bar settings saved.</p>}
    {!settings && !error && <p role="status" className="text-sm text-muted-foreground">Loading Menu Bar settings…</p>}
    {settings && <>
      <SettingsGroup label="Display">
        <SettingsRow label="Show in menu bar" description="The Bridge symbol follows your macOS appearance."
          control={<Switch label="Show in menu bar" checked={settings.enabled} disabled={busy} onChange={enabled => void save({ enabled })} />} />
        <SettingsRow label="Beside the icon" description="Choose the information that stays visible."
          control={<Select label="Beside the icon" value={settings.displayMode} disabled={busy} options={[
            { value: "icon", label: "Icon only" }, { value: "remaining", label: "Quota remaining" },
            { value: "used", label: "Quota used" }, { value: "cost", label: "Today's spend" },
          ]} onChange={displayMode => void save({ displayMode: displayMode as MenuBarSettings["displayMode"] })} />} />
        <SettingsRow label="Quota window" control={<Select label="Quota window" value={settings.quotaWindow} disabled={busy}
          options={[{ value: "session", label: "Session" }, { value: "weekly", label: "Weekly" }]}
          onChange={quotaWindow => void save({ quotaWindow: quotaWindow as MenuBarSettings["quotaWindow"] })} />} />
      </SettingsGroup>
      <SettingsGroup label="Providers & accounts" note="Codex is available first">
        <SettingsRow label="Codex" description={usage?.account ? `${usage.account}${usage.plan ? ` · ${usage.plan}` : ""}` : "Uses the account signed in through Codex. Manage sign-in in Harnesses."}
          control={<Switch label="Show Codex" checked={settings.codexEnabled} disabled={busy} onChange={codexEnabled => void save({ codexEnabled })} />} />
        <SettingsRow label="Show account" control={<Switch label="Show account" checked={settings.showAccount} disabled={busy}
          onChange={showAccount => void save({ showAccount })} />} />
        <SettingsRow label="More providers" description="Claude, Cursor, and OpenCode are next." />
      </SettingsGroup>
      <SettingsGroup label="Usage & spend">
        <SettingsRow label="Tokens and models" description="Recorded input, output, and cache tokens on this Mac."
          control={<Switch label="Tokens and models" checked={settings.showTokens} disabled={busy} onChange={showTokens => void save({ showTokens })} />} />
        <SettingsRow label="Show spend" description="Today and the last 30 days. Estimated costs are labelled; missing prices stay unavailable."
          control={<Switch label="Show spend" checked={settings.showCost} disabled={busy} onChange={showCost => void save({ showCost })} />} />
      </SettingsGroup>
      <SettingsGroup label="Refresh">
        <SettingsRow label="Refresh account usage" description="Shared with Bridge. Closing the main window keeps the menu available."
          control={<Select label="Refresh account usage" value={String(settings.refreshSeconds)} disabled={busy} options={[
            { value: "0", label: "Manually" }, { value: "60", label: "Every minute" }, { value: "300", label: "Every 5 minutes" },
            { value: "900", label: "Every 15 minutes" }, { value: "1800", label: "Every 30 minutes" },
          ]} onChange={seconds => void save({ refreshSeconds: Number(seconds) })} />} />
        {usage?.error && <SettingsRow label="Account status" description={usage.error} />}
      </SettingsGroup>
    </>}
  </SettingsPage>;
}
