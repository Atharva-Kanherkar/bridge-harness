import { useState } from "react";
import { KeyRound, LoaderCircle, RefreshCw, Unplug } from "lucide-react";
import { bridgeApi } from "../api";
import type { OpenCodeCatalog } from "../types";
import { cn } from "@/lib/utils";

export interface OpenCodeAdvancedSettings {
  executablePath?: string;
  visibleModels?: string[];
}

const field = "h-10 w-full rounded-xl border border-border bg-background/60 px-3 text-sm text-foreground outline-none transition-colors focus:border-foreground/25 disabled:opacity-45";

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

  return <div className="mt-4 space-y-4 border-t border-border/60 pt-4" data-testid="opencode-provider-settings">
    <div className="flex items-start justify-between gap-4">
      <div><h4 className="text-xs font-semibold text-foreground">OpenCode providers</h4><p className="mt-1 text-[10px] leading-relaxed text-muted-foreground">Uses OpenCode’s credential store, environment, and config. Provider keys are sent directly to OpenCode and are never saved by Bridge.</p></div>
      <button type="button" disabled={disabled || refreshing} onClick={() => void refresh()} className="inline-flex h-8 shrink-0 items-center gap-1.5 rounded-xl border border-border px-2.5 text-[10px] text-muted-foreground hover:bg-foreground/[0.05] hover:text-foreground disabled:opacity-40"><RefreshCw size={12} className={cn(refreshing && "animate-spin")}/>Refresh</button>
    </div>

    {discoveryError && <p role="alert" className="rounded-xl border border-red-400/20 bg-red-400/[0.06] px-3 py-2 text-[11px] text-red-300">{discoveryError}</p>}
    {catalog && <p className="text-[10px] text-muted-foreground">OpenCode {catalog.version} · <span className="font-mono">{catalog.executablePath}</span></p>}

    <div className="space-y-2">
      {catalog?.providers.map(provider => <div key={provider.id} className="rounded-2xl border border-border/70 bg-background/35 p-3">
        <div className="flex items-center gap-2"><span className={cn("h-2 w-2 rounded-full", provider.connected ? "bg-emerald-400" : "bg-muted-foreground/30")}/><span className="text-xs font-medium">{provider.name}</span><span className="text-[9px] text-muted-foreground">{provider.id}</span><span className={cn("ml-auto text-[9px] font-medium uppercase tracking-wider", provider.connected ? "text-emerald-300" : "text-muted-foreground")}>{provider.connected ? "Connected" : "Not connected"}</span></div>
        {provider.connected ? <div className="mt-2 flex items-center justify-between gap-3"><p className="text-[10px] text-muted-foreground">{provider.models.length} model{provider.models.length === 1 ? "" : "s"}{provider.source ? ` · ${provider.source}` : ""}</p><button type="button" disabled={disabled || workingProvider === provider.id} onClick={() => void disconnect(provider.id)} className="inline-flex h-7 items-center gap-1.5 rounded-lg px-2 text-[10px] text-red-300 hover:bg-red-400/[0.07] disabled:opacity-40">{workingProvider === provider.id ? <LoaderCircle size={11} className="animate-spin"/> : <Unplug size={11}/>}Disconnect</button></div> : provider.authMethods.some(method => method.kind === "api") ? <div className="mt-3 flex gap-2"><label className="sr-only" htmlFor={`opencode-key-${provider.id}`}>{provider.name} API key</label><input id={`opencode-key-${provider.id}`} type="password" autoComplete="off" className={cn(field, "h-8 min-w-0 text-xs")} placeholder={`${provider.name} API key`} value={providerKeys[provider.id] ?? ""} onChange={event => setProviderKeys(current => ({ ...current, [provider.id]: event.target.value }))}/><button type="button" disabled={disabled || workingProvider === provider.id || !(providerKeys[provider.id]?.trim())} onClick={() => void connect(provider.id)} className="inline-flex h-8 shrink-0 items-center gap-1.5 rounded-lg bg-foreground px-2.5 text-[10px] font-medium text-background disabled:opacity-40">{workingProvider === provider.id ? <LoaderCircle size={11} className="animate-spin"/> : <KeyRound size={11}/>}Connect</button></div> : <p className="mt-2 text-[10px] text-muted-foreground">Connect this provider with OpenCode’s supported authentication flow, then refresh.</p>}
      </div>)}
      {catalog && catalog.providers.length === 0 && <p className="rounded-xl border border-border/60 px-3 py-2 text-[11px] text-muted-foreground">OpenCode reported no providers.</p>}
    </div>

    <label className="block space-y-1.5 text-[11px] font-medium text-muted-foreground">OpenCode executable <span className="font-normal text-muted-foreground/60">optional; managed executable and PATH are fallback locations</span><input className={field} value={value.executablePath ?? ""} placeholder="/path/to/opencode" onChange={event => onChange({ ...value, executablePath: event.target.value || undefined })}/></label>

    <fieldset className="space-y-2"><legend className="text-[11px] font-medium text-muted-foreground">Visible models <span className="font-normal text-muted-foreground/60">none selected means all connected models</span></legend>{connectedModels.map(model => <label key={model.id} className="flex items-center gap-2 rounded-xl border border-border/60 px-3 py-2 text-[11px] text-foreground"><input type="checkbox" checked={visible.size === 0 || visible.has(model.id)} onChange={() => toggleModel(model.id)}/><span className="flex-1">{model.label}</span><span className="font-mono text-[9px] text-muted-foreground">{model.id}</span></label>)}</fieldset>
  </div>;
}
