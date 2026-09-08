import { useState } from "react";
import { RefreshCw as ArrowsClockwise, LoaderCircle as CircleNotch, Key, Plug as Plugs } from "lucide-react";
import { bridgeApi } from "../api";
import type { OpenCodeCatalog } from "../types";
import {
  GhostButton, PrimaryButton, SettingsGroup, SettingsRow, StatusPill, Switch, TextButton,
} from "./settings/kit";
import { cn } from "@/lib/utils";

// The two groups only OpenCode's harness detail page shows: which providers it
// has credentials for, and which of their models Bridge is allowed to offer.
//
// Provider keys are sent straight to OpenCode's own credential store. Nothing
// here writes a credential into Bridge's configuration, which is why the key
// field clears itself the moment the call returns.

export interface OpenCodeAdvancedSettings {
  executablePath?: string;
  visibleModels?: string[];
}

export function OpenCodeHarnessSettings({
  value,
  catalog,
  discoveryError,
  disabled,
  onChange,
  onCatalog,
  onError,
}: {
  value: OpenCodeAdvancedSettings;
  catalog?: OpenCodeCatalog;
  discoveryError?: string;
  disabled: boolean;
  onChange: (value: OpenCodeAdvancedSettings) => void;
  onCatalog: (catalog: OpenCodeCatalog) => void;
  onError: (message: string) => void;
}) {
  const [providerKeys, setProviderKeys] = useState<Record<string, string>>({});
  const [workingProvider, setWorkingProvider] = useState<string>();
  const [refreshing, setRefreshing] = useState(false);
  const visible = new Set(value.visibleModels ?? []);
  const connectedModels = catalog?.providers.flatMap(provider => provider.connected ? provider.models : []) ?? [];

  const refresh = async () => {
    setRefreshing(true);
    try { onCatalog(await bridgeApi.refreshOpenCodeCatalog()); }
    catch (error) { onError(error instanceof Error ? error.message : String(error)); }
    finally { setRefreshing(false); }
  };

  const connect = async (providerId: string) => {
    const apiKey = providerKeys[providerId]?.trim();
    if (!apiKey) return;
    setWorkingProvider(providerId);
    try {
      onCatalog(await bridgeApi.setOpenCodeProviderApiKey(providerId, apiKey));
      setProviderKeys(current => ({ ...current, [providerId]: "" }));
    } catch (error) { onError(error instanceof Error ? error.message : String(error)); }
    finally { setWorkingProvider(undefined); }
  };

  const disconnect = async (providerId: string) => {
    setWorkingProvider(providerId);
    try { onCatalog(await bridgeApi.removeOpenCodeProviderAuth(providerId)); }
    catch (error) { onError(error instanceof Error ? error.message : String(error)); }
    finally { setWorkingProvider(undefined); }
  };

  const toggleModel = (modelId: string) => {
    const allModelIds = connectedModels.map(model => model.id);
    const next = visible.size === 0 ? new Set(allModelIds) : new Set(visible);
    if (next.has(modelId)) next.delete(modelId); else next.add(modelId);
    onChange({
      ...value,
      visibleModels: next.size === allModelIds.length ? [] : [...next].sort(),
    });
  };

  return <div className="space-y-[26px]" data-testid="opencode-provider-settings">
    <SettingsGroup
      label="Providers"
      note={catalog ? `OpenCode ${catalog.version}` : undefined}
    >
      <SettingsRow
        label="Provider credentials"
        description="Uses OpenCode's own credential store, environment, and config. Keys are sent directly to OpenCode and are never saved by Bridge."
        control={<GhostButton disabled={disabled || refreshing} onClick={() => void refresh()}>
          <ArrowsClockwise size={12} strokeWidth={1.7} aria-hidden="true" className={cn(refreshing && "animate-spin")} />Refresh
        </GhostButton>}
      />
      {discoveryError && <SettingsRow label={<span role="alert" className="text-destructive">{discoveryError}</span>} />}
      {catalog?.providers.map(provider => <SettingsRow
        key={provider.id}
        label={provider.name}
        description={provider.connected
          ? `${provider.models.length} model${provider.models.length === 1 ? "" : "s"}${provider.source ? ` · ${provider.source}` : ""}`
          : provider.id}
        control={provider.connected
          ? <>
              <StatusPill tone="success">Connected</StatusPill>
              <TextButton
                tone="destructive"
                disabled={disabled || workingProvider === provider.id}
                onClick={() => void disconnect(provider.id)}
              >
                {workingProvider === provider.id
                  ? <CircleNotch size={12} strokeWidth={1.7} className="animate-spin" aria-hidden="true" />
                  : <Plugs size={12} strokeWidth={1.7} aria-hidden="true" />}
                Disconnect
              </TextButton>
            </>
          : provider.authMethods.some(method => method.kind === "api")
            ? <>
                <label className="sr-only" htmlFor={`opencode-key-${provider.id}`}>{provider.name} API key</label>
                <input
                  id={`opencode-key-${provider.id}`}
                  type="password"
                  autoComplete="off"
                  spellCheck={false}
                  placeholder={`${provider.name} API key`}
                  value={providerKeys[provider.id] ?? ""}
                  onChange={event => setProviderKeys(current => ({ ...current, [provider.id]: event.target.value }))}
                  className="h-7 w-44 shrink-0 rounded-lg border border-border-card bg-popover px-2.5 text-xs text-foreground outline-none transition-colors placeholder:text-muted-foreground focus:border-foreground/25"
                />
                <PrimaryButton
                  disabled={disabled || workingProvider === provider.id || !(providerKeys[provider.id]?.trim())}
                  onClick={() => void connect(provider.id)}
                >
                  {workingProvider === provider.id
                    ? <CircleNotch size={12} strokeWidth={1.7} className="animate-spin" aria-hidden="true" />
                    : <Key size={12} strokeWidth={1.7} aria-hidden="true" />}
                  Connect
                </PrimaryButton>
              </>
            : <StatusPill>Connect in OpenCode, then refresh</StatusPill>}
      />)}
      {catalog && catalog.providers.length === 0 && <SettingsRow label="OpenCode reported no providers." />}
    </SettingsGroup>

    {connectedModels.length > 0 && <SettingsGroup
      label="Visible models"
      note="All on means every connected model"
    >
      {connectedModels.map(model => <SettingsRow
        key={model.id}
        label={model.label}
        description={model.id}
        mono
        control={<Switch
          label={model.label}
          checked={visible.size === 0 || visible.has(model.id)}
          disabled={disabled}
          onChange={() => toggleModel(model.id)}
        />}
      />)}
    </SettingsGroup>}
  </div>;
}
