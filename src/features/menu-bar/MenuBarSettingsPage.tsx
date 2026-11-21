import { Fragment, useEffect, useRef, useState } from "react";
import { SkeletonRows } from "@/components/ui/skeleton";
import { RefreshCw } from "lucide-react";
import { bridgeApi } from "../../api";
import type { MenuBarSettings, ProviderUsageOverviews } from "../../protocol/generated/protocol";
import { PAGES } from "../../components/settings/sections";
import { Field, GhostButton, PrimaryButton, Segmented, SettingsPage, SettingsGroup, SettingsRow, Select, Switch, useSavedFlash } from "../../components/settings/kit";
import { MenuBarLayoutEditor } from "./MenuBarLayoutEditor";
import { chooseFavorite, defaultFavorites } from "./favorites";

const providers = [
  { id: "codex", name: "Codex", key: "codexEnabled", description: "Uses Codex sign-in. Manage the account in Harnesses." },
  { id: "claude", name: "Claude", key: "claudeEnabled", description: "Uses Claude Code sign-in for session and weekly limits." },
  { id: "cursor", name: "Cursor", key: "cursorEnabled", description: "Uses your Cursor desktop sign-in to read account usage, token/model history, and cost from cursor.com." },
  { id: "opencode", name: "OpenCode", key: "opencodeEnabled", description: "Local tokens and costs, plus five-hour, weekly, and monthly limits from your OpenCode Go account." },
] as const;

export function MenuBarSettingsPage() {
  const [settings, setSettings] = useState<MenuBarSettings | null>(null);
  const [usage, setUsage] = useState<ProviderUsageOverviews | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [isFlashed, flash] = useSavedFlash();
  const [connection, setConnection] = useState<string | null>(null);
  const [openCodeKey, setOpenCodeKey] = useState("");
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
    setBusy(true); setError(null);
    const operation = saves.current.then(async () => {
      try {
        const value = await bridgeApi.saveMenuBarSettings({ ...confirmedSettings.current!, ...patch });
        confirmedSettings.current = value;
        setSettings(value);
        for (const key of Object.keys(patch)) flash(key);
      } catch (error) { setError(String(error)); }
      finally { pendingSaves.current -= 1; if (pendingSaves.current === 0) setBusy(false); }
    });
    saves.current = operation;
    await operation;
  }

  async function refresh() {
    setBusy(true); setError(null);
    try { setUsage(await bridgeApi.refreshProviderUsageOverviews()); }
    catch (error) { setError(String(error)); }
    finally { setBusy(false); }
  }

  async function connectOpenCodeGo() {
    if (!openCodeKey.trim() || busy) return;
    const key = openCodeKey.trim();
    setOpenCodeKey(""); setBusy(true); setError(null); setConnection(null);
    try {
      await bridgeApi.setOpenCodeProviderApiKey("opencode-go", key);
      setConnection("OpenCode Go API key saved. Enable OpenCode usage and add it to favorites to show its limits in the menu.");
      if (settings?.opencodeEnabled) setUsage(await bridgeApi.refreshProviderUsageOverviews());
    } catch (error) {
      setError(String(error).includes("OPENCODE_AUTH_CONTENT")
        ? "OpenCode authentication is managed by OPENCODE_AUTH_CONTENT. Update that environment setting instead."
        : "Could not save the OpenCode Go API key. Check that OpenCode is installed, then try again.");
    } finally { setBusy(false); }
  }

  return <SettingsPage title={PAGES.menuBar.label} description={PAGES.menuBar.description}
    action={<GhostButton disabled={busy || enabled.length === 0} onClick={() => void refresh()}>
      <RefreshCw size={12} aria-hidden="true" />Refresh usage
    </GhostButton>}>
    {error && <p role="alert" className="text-ui text-destructive">{error}</p>}
    {connection && <p role="status" className="text-ui text-muted-foreground">{connection}</p>}
    {!settings && !error && <SkeletonRows label="Loading Menu Bar settings" rows={4} trailing />}
    {settings && <>
      <SettingsGroup anchor="menu-display" label="Display">
        <SettingsRow saved={isFlashed("enabled")} label="Show in menu bar" description="The Bridge symbol follows your macOS appearance."
          control={<Switch label="Show in menu bar" checked={settings.enabled} disabled={busy} onChange={enabled => void save({ enabled })} />} />
        <SettingsRow saved={isFlashed("separateProviderIcons")} label="Separate provider icons" description="Show each favorite with account usage enabled as its own icon in the macOS menu bar. Each icon opens that provider."
          control={<Switch label="Separate provider icons" checked={settings.separateProviderIcons ?? false} disabled={busy} onChange={separateProviderIcons => void save({ separateProviderIcons })} />} />
        <SettingsRow saved={isFlashed("openToOverview")} label="Open to Overview" description="See your favorite providers’ current quotas together. Details stay in provider tabs."
          control={<Switch label="Open to Overview" checked={settings.openToOverview ?? true} disabled={busy} onChange={openToOverview => void save({ openToOverview })} />} />
        <SettingsRow saved={isFlashed("quotaDisplayMode")} label="Quota bars" description="Used fills the bar as you consume your allowance. The opposite percentage appears below."
          control={<Segmented label="Quota bars" value={settings.quotaDisplayMode ?? "used"} disabled={busy}
            options={[{ value: "used", label: "Used" }, { value: "remaining", label: "Remaining" }]}
            onChange={value => void save({ quotaDisplayMode: value as MenuBarSettings["quotaDisplayMode"] })} />} />
        <SettingsRow saved={isFlashed("iconStyle")} label="Menu icon" description="Single-icon mode uses this mark. Separate icons use each provider’s logo. All follow macOS appearance."
          control={<Segmented label="Menu icon" value={settings.iconStyle ?? "bridge"} disabled={busy || settings.separateProviderIcons}
            options={[{ value: "bridge", label: "Bridge logo" }, { value: "meter", label: "Quota meters" }]}
            onChange={value => void save({ iconStyle: value as MenuBarSettings["iconStyle"] })} />} />
        <SettingsRow saved={isFlashed("displayMode")} label="Beside the icon" description={customDisplay ? "Custom layout is active. Choose a standard display to replace it." : "Choose what appears beside each menu icon."}
          control={<Select label="Beside the icon" value={customDisplay ? "custom" : settings.displayMode} disabled={busy} options={[
            ...(customDisplay ? [{ value: "custom", label: "Custom layout", disabled: true }] : []),
            { value: "icon", label: "Icon only" }, { value: "remaining", label: "Quota remaining" },
            { value: "used", label: "Quota used" }, { value: "cost", label: "Today's spend" },
          ]} onChange={displayMode => { if (displayMode !== "custom") void save({ displayMode: displayMode as MenuBarSettings["displayMode"], statusLayout: [] }); }} />} />
        <SettingsRow saved={isFlashed("quotaWindow")} label="Quota window" description="Automatic uses the provider’s first available limit, including billing cycles."
          control={<Segmented label="Quota window" value={settings.quotaWindow} disabled={busy}
          options={[{ value: "auto", label: "Automatic" }, { value: "session", label: "5-hour" }, { value: "weekly", label: "Weekly" }]}
          onChange={quotaWindow => void save({ quotaWindow: quotaWindow as MenuBarSettings["quotaWindow"] })} />} />
      </SettingsGroup>
      <SettingsGroup label="Menu icon layout">
        {settings.separateProviderIcons && <p className="px-3.5 py-2.5 text-meta leading-relaxed text-muted-foreground">Custom layouts apply to the single Bridge icon. Separate provider icons use the standard display selected above.</p>}
        <MenuBarLayoutEditor key={JSON.stringify(settings.statusLayout ?? [])} layout={settings.statusLayout ?? []} busy={busy || !!settings.separateProviderIcons}
          onSave={statusLayout => save({ statusLayout })} />
      </SettingsGroup>
      <SettingsGroup anchor="menu-favorites" label="Favorite providers">
        <p className="px-3.5 py-2.5 text-meta text-muted-foreground">In single-icon mode, your first favorite supplies the usage beside the menu icon. Switching tabs only changes the open menu. Favorites control the tabs, overview totals, and separate icons. Add more favorites below, then scroll or use the arrows. Disconnected favorites keep their tabs.</p>
        {favorites.map((_, position) => <SettingsRow saved={isFlashed("pinnedProviders")} key={position} label={`Favorite ${position + 1}`}
          control={<Select label={`Favorite provider ${position + 1}`} value={favorites[position] ?? ""} disabled={busy}
            options={[{ value: "", label: "Remove favorite" }, ...providers.map(provider => ({ value: provider.id, label: provider.name }))]}
            onChange={value => void save({ pinnedProviders: chooseFavorite(favorites, position, value as (typeof favorites)[number] | "") })} />} />)}
        <div className="px-3.5 py-2.5">
          <GhostButton disabled={busy || !nextFavorite}
            onClick={() => { if (nextFavorite) void save({ pinnedProviders: [...favorites, nextFavorite.id] }); }}>
            Add favorite
          </GhostButton>
        </div>
      </SettingsGroup>
      <SettingsGroup anchor="menu-providers" label="Providers & accounts">
        <p className="px-3.5 py-2.5 text-meta text-muted-foreground">Enable account usage for each provider. Choosing a favorite does not connect its account.</p>
        {providers.map(provider => {
          // The provider's own login; extra accounts follow it, read with the
          // same switch, so a second subscription is never invisible here.
          const account = usage?.providers.find(value => value.provider === provider.id && !value.accountId);
          const extra = (usage?.providers ?? []).filter(value => value.provider === provider.id && !!value.accountId);
          return <Fragment key={provider.id}>
            <SettingsRow label={provider.name}
              description={!settings[provider.key] ? provider.description : account?.account ? `${account.account}${account.plan ? ` · ${account.plan}` : ""}` : account?.error ?? (account?.quotaSource ? `Usage via ${account.quotaSource}` : provider.description)}
              control={<Switch label={`Read ${provider.name} usage`} checked={settings[provider.key] ?? false} disabled={busy}
                onChange={value => void save({ [provider.key]: value })} />} />
            {extra.map(row => <SettingsRow key={row.accountId} label={`${provider.name} · ${row.accountLabel || row.accountId}`}
              description={!settings[provider.key] ? `Shown when ${provider.name} usage is on.` : row.account ? `${row.account}${row.plan ? ` · ${row.plan}` : ""}` : row.error ?? "Not read yet."}
              control={<span className="text-caption text-muted-foreground">{settings[provider.key] ? "Included" : "Off"}</span>} />)}
          </Fragment>;
        })}
        {settings.claudeEnabled && <p className="px-3.5 py-2.5 text-meta text-muted-foreground">Claude limits come from your Claude Code subscription sign-in. Claude Code handles authentication and renewal. If limits are unavailable, open Claude Code, check your account, then refresh usage.</p>}
        <SettingsRow label="Provider beside the icon" description={settings.separateProviderIcons ? "Each favorite with account usage enabled has its own icon and usage." : "Uses your first favorite, or the first enabled provider when no favorites are set."}
          control={<span className="text-ui text-muted-foreground" aria-label="Provider beside the icon">{settings.separateProviderIcons ? enabledFavorites.map(provider => provider.name).join(", ") || "None" : visible[0]?.name ?? enabled[0]?.name ?? "None"}</span>} />
        <SettingsRow label="OpenCode Go account" description="Sign in using your default browser, then copy your API key below. Already connected through OpenCode? Enable usage and click Refresh usage."
          control={<GhostButton disabled={busy}
            onClick={() => { setConnection(null); void bridgeApi.connectMenuBarOpenCode().catch(() => setError("Could not open your default browser. Visit opencode.ai/auth to sign in.")); }}>Connect OpenCode</GhostButton>} />
        <SettingsRow label="OpenCode Go API key" description="Saved by OpenCode in its own authentication store. Requires OpenCode to be installed."
          control={<form className="flex flex-wrap items-center gap-2" onSubmit={event => { event.preventDefault(); void connectOpenCodeGo(); }}>
            <Field label="OpenCode Go API key" type="password" mono width="w-44"
              value={openCodeKey} onChange={setOpenCodeKey} disabled={busy}
              placeholder="Paste API key" />
            <PrimaryButton type="submit" disabled={busy || !openCodeKey.trim()}>Save key</PrimaryButton>
          </form>} />
        <SettingsRow label="OpenCode workspace" description="For previously connected Zen browser sessions only. OpenCode Go uses your API key and does not need a workspace ID."
          saved={isFlashed("opencodeWorkspace")}
          control={<WorkspaceField key={settings.opencodeWorkspace ?? "connected"} stored={settings.opencodeWorkspace ?? null} disabled={busy}
            onSave={value => void save({ opencodeWorkspace: value })} />} />
        <SettingsRow saved={isFlashed("showAccount")} label="Show account" control={<Switch label="Show account" checked={settings.showAccount} disabled={busy}
          onChange={showAccount => void save({ showAccount })} />} />
      </SettingsGroup>
      <SettingsGroup anchor="menu-usage" label="Usage & spend">
        <SettingsRow saved={isFlashed("showOverviewSummary")} label="Overview usage & spend" description="Show a 30-day summary across favorites with account usage enabled above the Overview quota bars."
          control={<Switch label="Overview usage & spend" checked={settings.showOverviewSummary ?? true} disabled={busy} onChange={showOverviewSummary => void save({ showOverviewSummary })} />} />
        <SettingsRow saved={isFlashed("showHistory")} label="Daily history" description="A 30-day chart in provider tabs. Overview stays compact without charts."
          control={<Switch label="Daily history" checked={settings.showHistory ?? true} disabled={busy} onChange={showHistory => void save({ showHistory })} />} />
        <SettingsRow saved={isFlashed("showTokens")} label="Tokens and models" description="Show token totals. Model details are available in the breakdown submenu."
          control={<Switch label="Tokens and models" checked={settings.showTokens} disabled={busy} onChange={showTokens => void save({ showTokens })} />} />
        <SettingsRow saved={isFlashed("showCost")} label="Show spend" description="Today and the last 30 days. Estimated costs are labelled; missing prices stay unavailable."
          control={<Switch label="Show spend" checked={settings.showCost} disabled={busy} onChange={showCost => void save({ showCost })} />} />
      </SettingsGroup>
      <SettingsGroup anchor="menu-refresh" label="Refresh">
        <SettingsRow saved={isFlashed("refreshSeconds")} label="Refresh account usage" description="Shared with Bridge. Closing the main window keeps the menu available."
          control={<Select label="Refresh account usage" value={String(settings.refreshSeconds)} disabled={busy} options={[
            { value: "0", label: "Manually" }, { value: "60", label: "Every minute" }, { value: "300", label: "Every 5 minutes" },
            { value: "900", label: "Every 15 minutes" }, { value: "1800", label: "Every 30 minutes" },
          ]} onChange={seconds => void save({ refreshSeconds: Number(seconds) })} />} />
      </SettingsGroup>
    </>}
  </SettingsPage>;
}

/** Free text saved when you leave the field or press Return, the way the
 *  menu bar helper expects one workspace id rather than every keystroke. */
function WorkspaceField({ stored, disabled, onSave }: { stored: string | null; disabled: boolean; onSave: (value: string | null) => void }) {
  const [value, setValue] = useState(stored ?? "");
  const commit = () => { const next = value.trim() || null; if (next !== stored) onSave(next); };
  return <span onBlur={commit}>
    <Field label="OpenCode workspace" value={value} disabled={disabled} width="w-44" placeholder="Connected workspace"
      onChange={next => setValue(next.slice(0, 132))}
      onKeyDown={event => { if (event.key === "Enter") commit(); }} />
  </span>;
}
