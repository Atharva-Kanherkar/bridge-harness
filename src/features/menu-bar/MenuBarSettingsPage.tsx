import { useEffect, useRef, useState } from "react";
import { RefreshCw } from "lucide-react";
import { bridgeApi } from "../../api";
import type { MenuBarSettings, ProviderUsageOverviews } from "../../protocol/generated/protocol";
import { SettingsPage, SettingsGroup, SettingsRow, Select, Switch } from "../../components/settings/kit";
import { MenuBarLayoutEditor } from "./MenuBarLayoutEditor";
import { chooseFavorite, defaultFavorites } from "./favorites";

const providers = [
  { id: "codex", name: "Codex", key: "codexEnabled", description: "Uses Codex sign-in. Manage the account in Harnesses." },
  { id: "claude", name: "Claude", key: "claudeEnabled", description: "Uses Claude Code sign-in for session and weekly limits." },
  { id: "cursor", name: "Cursor", key: "cursorEnabled", description: "Uses your Cursor desktop sign-in to read account usage, token/model history, and cost from cursor.com." },
  { id: "opencode", name: "OpenCode", key: "opencodeEnabled", description: "Local tokens and costs. Connect a Zen workspace for account billing and limits." },
] as const;

export function MenuBarSettingsPage() {
  const [settings, setSettings] = useState<MenuBarSettings | null>(null);
  const [usage, setUsage] = useState<ProviderUsageOverviews | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const [connection, setConnection] = useState<string | null>(null);
  const confirmedSettings = useRef<MenuBarSettings | null>(null);
  const saves = useRef<Promise<void>>(Promise.resolve());
  const pendingSaves = useRef(0);
  const enabled = providers.filter(provider => settings?.[provider.key]);
  const favorites = settings?.pinnedProviders ?? defaultFavorites;
  const visibleIds = favorites;
  const nextFavorite = providers.find(provider => !favorites.includes(provider.id));
  const visible = visibleIds.flatMap(id => providers.filter(provider => provider.id === id));
  const enabledFavorites = visible.filter(provider => settings?.[provider.key]);
  const customDisplay = !settings?.separateProviderIcons && (settings?.statusLayout?.flat().some(token => token !== "space" && token !== "dot") ?? false);

  useEffect(() => {
    let active = true;
    const subscriptions: (() => void)[] = [];
    const subscribe = (promise: Promise<() => void>) => { void promise.then(off => { if (active) subscriptions.push(off); else off(); }).catch(error => { if (active) setError(String(error)); }); };
    // A failed usage read must not prevent disabling or changing the menu.
    void bridgeApi.getMenuBarSettings().then(settings => {
      if (active) { confirmedSettings.current = settings; setSettings(settings); }
    }).catch(error => { if (active) setError(String(error)); });
    void bridgeApi.getProviderUsageOverviews().then(usage => {
      if (active) setUsage(usage);
    }).catch(error => { if (active) setError(String(error)); });
    subscribe(bridgeApi.onProviderUsageOverviews(value => { if (active) setUsage(value); }));
    subscribe(bridgeApi.onMenuBarConnection(message => { if (active) setConnection(message); }));
    subscribe(bridgeApi.onMenuBarSettingsChanged(() => {
      void bridgeApi.getMenuBarSettings().then(value => { if (active && pendingSaves.current === 0) { confirmedSettings.current = value; setSettings(value); } }).catch(error => { if (active) setError(String(error)); });
    }));
    return () => { active = false; subscriptions.forEach(off => off()); };
  }, []);

  async function save(patch: Partial<MenuBarSettings>) {
    if (!confirmedSettings.current) return;
    pendingSaves.current += 1;
    setBusy(true); setError(null); setSaved(false);
    const operation = saves.current.then(async () => {
      try {
        const value = await bridgeApi.saveMenuBarSettings({ ...confirmedSettings.current!, ...patch });
        confirmedSettings.current = value;
        setSettings(value);
        if (pendingSaves.current === 1) setSaved(true);
      } catch (error) { setError(String(error)); setSaved(false); }
      finally { pendingSaves.current -= 1; if (pendingSaves.current === 0) setBusy(false); }
    });
    saves.current = operation;
    await operation;
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
      <RefreshCw size={12} aria-hidden="true" />Refresh usage
    </button>}>
    {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
    {connection && <p role="status" className="text-xs text-muted-foreground">{connection}</p>}
    {saved && <p role="status" className="text-xs text-muted-foreground">Menu Bar settings saved.</p>}
    {!settings && !error && <p role="status" className="text-sm text-muted-foreground">Loading Menu Bar settings…</p>}
    {settings && <>
      <SettingsGroup label="Display">
        <SettingsRow label="Show in menu bar" description="The Bridge symbol follows your macOS appearance."
          control={<Switch label="Show in menu bar" checked={settings.enabled} disabled={busy} onChange={enabled => void save({ enabled })} />} />
        <SettingsRow label="Separate provider icons" description="Show each favorite with account usage enabled as its own icon in the macOS menu bar. Each icon opens that provider."
          control={<Switch label="Separate provider icons" checked={settings.separateProviderIcons ?? false} disabled={busy} onChange={separateProviderIcons => void save({ separateProviderIcons })} />} />
        <SettingsRow label="Open to Overview" description="See your favorite providers’ current quotas together. Details stay in provider tabs."
          control={<Switch label="Open to Overview" checked={settings.openToOverview ?? true} disabled={busy} onChange={openToOverview => void save({ openToOverview })} />} />
        <SettingsRow label="Quota bars" description="Used fills the bar as you consume your allowance. The opposite percentage appears below."
          control={<Select label="Quota bars" value={settings.quotaDisplayMode ?? "used"} disabled={busy}
            options={[{ value: "used", label: "Show used" }, { value: "remaining", label: "Show remaining" }]}
            onChange={value => void save({ quotaDisplayMode: value as MenuBarSettings["quotaDisplayMode"] })} />} />
        <SettingsRow label="Menu icon" description="Single-icon mode uses this mark. Separate icons use each provider’s logo. All follow macOS appearance."
          control={<Select label="Menu icon" value={settings.iconStyle ?? "bridge"} disabled={busy || settings.separateProviderIcons}
            options={[{ value: "bridge", label: "Bridge logo" }, { value: "meter", label: "Quota meters" }]}
            onChange={value => void save({ iconStyle: value as MenuBarSettings["iconStyle"] })} />} />
        <SettingsRow label="Beside the icon" description={customDisplay ? "Custom layout is active. Choose a standard display to replace it." : "Choose what appears beside each menu icon."}
          control={<Select label="Beside the icon" value={customDisplay ? "custom" : settings.displayMode} disabled={busy} options={[
            ...(customDisplay ? [{ value: "custom", label: "Custom layout", disabled: true }] : []),
            { value: "icon", label: "Icon only" }, { value: "remaining", label: "Quota remaining" },
            { value: "used", label: "Quota used" }, { value: "cost", label: "Today's spend" },
          ]} onChange={displayMode => { if (displayMode !== "custom") void save({ displayMode: displayMode as MenuBarSettings["displayMode"], statusLayout: [] }); }} />} />
        <SettingsRow label="Quota window" description="Automatic uses the provider’s first available limit, including billing cycles."
          control={<Select label="Quota window" value={settings.quotaWindow} disabled={busy}
          options={[{ value: "auto", label: "Automatic" }, { value: "session", label: "5-hour / rolling" }, { value: "weekly", label: "Weekly" }]}
          onChange={quotaWindow => void save({ quotaWindow: quotaWindow as MenuBarSettings["quotaWindow"] })} />} />
      </SettingsGroup>
      <SettingsGroup label="Menu icon layout">
        {settings.separateProviderIcons && <p className="px-4 py-3 text-xs text-muted-foreground">Custom layouts apply to the single Bridge icon. Separate provider icons use the standard display selected above.</p>}
        <MenuBarLayoutEditor key={JSON.stringify(settings.statusLayout ?? [])} layout={settings.statusLayout ?? []} busy={busy || !!settings.separateProviderIcons}
          onSave={statusLayout => save({ statusLayout })} />
      </SettingsGroup>
      <SettingsGroup label="Favorite providers">
        <p className="px-4 py-3 text-xs text-muted-foreground">In single-icon mode, your first favorite supplies the usage beside the menu icon. Switching tabs only changes the open menu. Favorites control the tabs, overview totals, and separate icons. Add more favorites below, then scroll or use the arrows. Disconnected favorites keep their tabs.</p>
        {favorites.map((_, position) => <SettingsRow key={position} label={`Favorite ${position + 1}`}
          control={<Select label={`Favorite provider ${position + 1}`} value={favorites[position] ?? ""} disabled={busy}
            options={[{ value: "", label: "Remove favorite" }, ...providers.map(provider => ({ value: provider.id, label: provider.name }))]}
            onChange={value => void save({ pinnedProviders: chooseFavorite(favorites, position, value as (typeof favorites)[number] | "") })} />} />)}
        <div className="px-4 py-3">
          <button type="button" disabled={busy || !nextFavorite}
            className="rounded-lg border border-border px-3 py-1.5 text-xs hover:bg-accent disabled:opacity-40"
            onClick={() => { if (nextFavorite) void save({ pinnedProviders: [...favorites, nextFavorite.id] }); }}>
            Add favorite
          </button>
        </div>
      </SettingsGroup>
      <SettingsGroup label="Providers & accounts">
        <p className="px-4 py-3 text-xs text-muted-foreground">Enable account usage for each provider. Choosing a favorite does not connect its account.</p>
        {providers.map(provider => {
          const account = usage?.providers.find(value => value.provider === provider.id);
          return <SettingsRow key={provider.id} label={provider.name}
            description={!settings[provider.key] ? provider.description : account?.account ? `${account.account}${account.plan ? ` · ${account.plan}` : ""}` : account?.error ?? (account?.quotaSource ? `Usage via ${account.quotaSource}` : provider.description)}
            control={<Switch label={`Read ${provider.name} usage`} checked={settings[provider.key] ?? false} disabled={busy}
              onChange={value => void save({ [provider.key]: value })} />} />;
        })}
        {settings.claudeEnabled && <p className="px-4 py-3 text-xs text-muted-foreground">Claude limits use your Claude Code subscription sign-in, not an API key. Refresh usage can request access to “Claude Code-credentials” in macOS Keychain; choose Always Allow to enable automatic reads. Refresh can also ask Claude Code to renew an expired sign-in. Background updates never request Keychain permission.</p>}
        <SettingsRow label="Provider beside the icon" description={settings.separateProviderIcons ? "Each favorite with account usage enabled has its own icon and usage." : "Uses your first favorite, or the first enabled provider when no favorites are set."}
          control={<span className="text-sm text-muted-foreground" aria-label="Provider beside the icon">{settings.separateProviderIcons ? enabledFavorites.map(provider => provider.name).join(", ") || "None" : visible[0]?.name ?? enabled[0]?.name ?? "None"}</span>} />
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
        <SettingsRow label="Overview usage & spend" description="Show a 30-day summary across favorites with account usage enabled above the Overview quota bars."
          control={<Switch label="Overview usage & spend" checked={settings.showOverviewSummary ?? true} disabled={busy} onChange={showOverviewSummary => void save({ showOverviewSummary })} />} />
        <SettingsRow label="Daily history" description="A 30-day chart in provider tabs. Overview stays compact without charts."
          control={<Switch label="Daily history" checked={settings.showHistory ?? true} disabled={busy} onChange={showHistory => void save({ showHistory })} />} />
        <SettingsRow label="Tokens and models" description="Show token totals. Model details are available in the breakdown submenu."
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
