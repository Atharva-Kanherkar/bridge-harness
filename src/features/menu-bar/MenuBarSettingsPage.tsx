import { useEffect, useState } from "react";
import { RefreshCw } from "lucide-react";
import { bridgeApi } from "../../api";
import type { MenuBarSettings, ProviderUsageOverviews } from "../../protocol/generated/protocol";
import { SettingsPage, SettingsGroup, SettingsRow, Select, Switch } from "../../components/settings/kit";

const providers = [
  { id: "codex", name: "Codex", key: "codexEnabled", description: "Uses Codex sign-in. Manage the account in Harnesses." },
  { id: "claude", name: "Claude", key: "claudeEnabled", description: "Uses Claude Code sign-in for session and weekly limits." },
  { id: "cursor", name: "Cursor", key: "cursorEnabled", description: "Sign in to Cursor desktop to read billing cycle usage." },
  { id: "opencode", name: "OpenCode", key: "opencodeEnabled", description: "Local tokens and costs. Connect a Zen workspace for account billing and limits." },
] as const;

export function MenuBarSettingsPage() {
  const [settings, setSettings] = useState<MenuBarSettings | null>(null);
  const [usage, setUsage] = useState<ProviderUsageOverviews | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const [connection, setConnection] = useState<string | null>(null);
  const enabled = providers.filter(provider => settings?.[provider.key]);

  useEffect(() => {
    let active = true;
    const subscriptions: (() => void)[] = [];
    const subscribe = (promise: Promise<() => void>) => { void promise.then(off => { if (active) subscriptions.push(off); else off(); }).catch(error => { if (active) setError(String(error)); }); };
    // A failed usage read must not prevent disabling or changing the menu.
    void bridgeApi.getMenuBarSettings().then(settings => {
      if (active) setSettings(settings);
    }).catch(error => { if (active) setError(String(error)); });
    void bridgeApi.getProviderUsageOverviews().then(usage => {
      if (active) setUsage(usage);
    }).catch(error => { if (active) setError(String(error)); });
    subscribe(bridgeApi.onProviderUsageOverviews(value => { if (active) setUsage(value); }));
    subscribe(bridgeApi.onMenuBarConnection(message => { if (active) setConnection(message); }));
    subscribe(bridgeApi.onMenuBarSettingsChanged(() => {
      void bridgeApi.getMenuBarSettings().then(value => { if (active) setSettings(value); }).catch(error => { if (active) setError(String(error)); });
    }));
    return () => { active = false; subscriptions.forEach(off => off()); };
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
    try { setUsage(await bridgeApi.refreshProviderUsageOverviews()); }
    catch (error) { setError(String(error)); }
    finally { setBusy(false); }
  }

  return <SettingsPage title="Menu Bar" description="Account limits, tokens, and spend at a glance."
    action={<button type="button" disabled={busy || enabled.length === 0} onClick={() => void refresh()}
      className="flex items-center gap-2 rounded-lg border border-border px-3 py-1.5 text-xs text-foreground hover:bg-accent disabled:opacity-40">
      <RefreshCw size={12} aria-hidden="true" className={busy ? "animate-spin" : ""} />Refresh usage
    </button>}>
    {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
    {connection && <p role="status" className="text-xs text-muted-foreground">{connection}</p>}
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
        <SettingsRow label="Quota window" description="Automatic uses the provider’s first available limit, including billing cycles."
          control={<Select label="Quota window" value={settings.quotaWindow} disabled={busy}
          options={[{ value: "auto", label: "Automatic" }, { value: "session", label: "Session" }, { value: "weekly", label: "Weekly" }]}
          onChange={quotaWindow => void save({ quotaWindow: quotaWindow as MenuBarSettings["quotaWindow"] })} />} />
      </SettingsGroup>
      <SettingsGroup label="Providers & accounts">
        {providers.map(provider => {
          const account = usage?.providers.find(value => value.provider === provider.id);
          return <SettingsRow key={provider.id} label={provider.name}
            description={account?.account ? `${account.account}${account.plan ? ` · ${account.plan}` : ""}` : account?.error ?? provider.description}
            control={<Switch label={`Show ${provider.name}`} checked={settings[provider.key] ?? false} disabled={busy}
              onChange={value => void save({ [provider.key]: value })} />} />;
        })}
        <SettingsRow label="Provider beside the icon" description="Also changes when you switch providers in the menu."
          control={<Select label="Provider beside the icon" value={enabled.some(p => p.id === settings.selectedProvider) ? settings.selectedProvider ?? "codex" : enabled[0]?.id ?? "codex"}
            disabled={busy || enabled.length === 0} options={enabled.map(p => ({ value: p.id, label: p.name }))}
            onChange={value => void save({ selectedProvider: value as MenuBarSettings["selectedProvider"] })} />} />
        <SettingsRow label="OpenCode Zen account" description="Sign in and open a workspace. Bridge saves that session in macOS Keychain."
          control={<button type="button" disabled={busy} className="rounded-lg border border-border px-3 py-1.5 text-xs hover:bg-accent disabled:opacity-40"
            onClick={() => { setConnection("Complete sign-in in the OpenCode window and open your workspace."); void bridgeApi.connectMenuBarOpenCode().catch(error => setError(String(error))); }}>Connect OpenCode</button>} />
        <SettingsRow label="OpenCode workspace" description="Optional workspace ID (wrk_…). Leave blank to use the connected workspace."
          control={<input key={settings.opencodeWorkspace ?? "connected"} aria-label="OpenCode workspace" defaultValue={settings.opencodeWorkspace ?? ""}
            placeholder="Connected workspace" disabled={busy} maxLength={132}
            className="w-44 rounded-lg border border-border bg-background px-2 py-1.5 text-xs text-foreground"
            onBlur={event => { const value = event.currentTarget.value.trim() || null; if (value !== (settings.opencodeWorkspace ?? null)) void save({ opencodeWorkspace: value }); }} />} />
        <SettingsRow label="Show account" control={<Switch label="Show account" checked={settings.showAccount} disabled={busy}
          onChange={showAccount => void save({ showAccount })} />} />
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
      </SettingsGroup>
    </>}
  </SettingsPage>;
}
