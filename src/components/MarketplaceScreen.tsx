import { useCallback, useEffect, useMemo, useState } from "react";
import { AlertCircle, Check, ExternalLink, LoaderCircle, Package, RefreshCw, Search, ShieldCheck, Unplug, X } from "lucide-react";
import { bridgeApi } from "../api";
import { compatibilityLabels, failedVariants, groupMarketplaceServices, installVariants, MARKETPLACE_ALIASES, type MarketplaceService } from "../marketplace";
import type { MarketplaceAction, MarketplaceActionResult, MarketplaceCatalog, MarketplaceProvider, MarketplaceVariant } from "../types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

type InstallTarget = MarketplaceProvider | "both";

function providerLabel(provider: MarketplaceProvider): string {
  return provider === "codex" ? "Codex" : "Claude Code";
}

function Status({ variant }: { variant: MarketplaceVariant }) {
  const auth = variant.authenticationState.toLowerCase();
  return <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] text-neutral-500">
    <span className="inline-flex items-center gap-1.5">
      <span className={`h-1.5 w-1.5 rounded-full ${variant.installed ? "bg-emerald-400" : "bg-neutral-600"}`} />
      {variant.installed ? "Installed" : "Available"}
    </span>
    <span>{variant.enabled ? "Enabled" : "Disabled"}</span>
    <span className={auth === "connected" ? "text-emerald-400" : auth === "required" ? "text-amber-400" : ""}>
      {auth === "connected" ? "Connected" : auth === "required" ? "Needs login" : "Auth unknown"}
    </span>
  </div>;
}

function actionLabel(action: MarketplaceAction): string {
  if (action === "authenticate") return "Connect";
  return action[0].toUpperCase() + action.slice(1);
}

function ServiceCard({
  service,
  busyKey,
  target,
  results,
  onTarget,
  onInstall,
  onRetry,
  onAction,
}: {
  service: MarketplaceService;
  busyKey: string | null;
  target: InstallTarget;
  results: MarketplaceActionResult[];
  onTarget: (target: InstallTarget) => void;
  onInstall: () => void;
  onRetry: () => void;
  onAction: (variant: MarketplaceVariant, action: MarketplaceAction) => void;
}) {
  const labels = compatibilityLabels(service);
  const failed = results.filter(result => !result.success);
  const source = service.variants.find(variant => variant.source || variant.repository);
  const sourceUrl = source?.repository?.startsWith("http") ? source.repository : source?.source?.startsWith("http") ? source.source : null;
  const marketplaceName = service.variants.find(variant => variant.marketplace)?.marketplace;
  const hasCodex = service.variants.some(variant => variant.provider === "codex");
  const hasClaude = service.variants.some(variant => variant.provider === "claude");
  const installable = service.variants.some(variant => !variant.installed);

  return <article className="rounded-3xl border border-white/[0.08] bg-white/[0.035] p-4 shadow-[inset_0_1px_0_rgba(255,255,255,0.025)] sm:p-5">
    <div className="flex items-start gap-3">
      <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-2xl border border-white/[0.08] bg-white/[0.05] text-neutral-300">
        <Package size={18} strokeWidth={1.6} aria-hidden="true" />
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <h2 className="font-display text-[15px] font-semibold tracking-tight text-white">{service.name}</h2>
          {hasCodex && <Badge variant="secondary" size="sm">Codex</Badge>}
          {hasClaude && <Badge variant="secondary" size="sm">Claude Code</Badge>}
        </div>
        {service.description && <p className="mt-1 max-w-2xl text-xs leading-relaxed text-neutral-500">{service.description}</p>}
        <div className="mt-2 flex flex-wrap items-center gap-1.5">
          {labels.map(label => <span key={label} className={`rounded-full border px-2 py-0.5 text-[9.5px] ${label === "Separate login required" ? "border-amber-400/20 bg-amber-400/[0.06] text-amber-300" : "border-white/[0.07] bg-white/[0.025] text-neutral-500"}`}>{label}</span>)}
          {sourceUrl ? <a className="inline-flex items-center gap-1 rounded-full px-1.5 py-0.5 text-[9.5px] text-neutral-500 transition-colors hover:text-neutral-200" href={sourceUrl} target="_blank" rel="noreferrer">Source <ExternalLink size={9} aria-hidden="true" /></a> : marketplaceName && <span className="rounded-full px-1.5 py-0.5 text-[9.5px] text-neutral-600">Source: {marketplaceName}</span>}
        </div>
      </div>
    </div>

    <div className="mt-4 divide-y divide-white/[0.06] rounded-2xl border border-white/[0.06] bg-black/10">
      {service.variants.map(variant => {
        const key = `${service.id}:${variant.provider}`;
        const working = busyKey === key;
        const nextToggle: MarketplaceAction = variant.enabled ? "disable" : "enable";
        const canToggle = variant.supportedActions.includes(nextToggle);
        return <div key={`${variant.provider}:${variant.pluginId}`} className="flex flex-col gap-3 p-3 sm:flex-row sm:items-center">
          <div className="min-w-0 flex-1">
            <div className="mb-1 flex items-center gap-2 text-xs font-medium text-neutral-200">
              {providerLabel(variant.provider)} variant
              {variant.version && <span className="font-mono text-[9.5px] font-normal text-neutral-600">v{variant.version}</span>}
            </div>
            <Status variant={variant} />
          </div>
          <div className="flex flex-wrap items-center gap-1.5">
            {variant.installed && <>
              {canToggle && <Button size="xs" variant="ghost" disabled={!!busyKey} onClick={() => onAction(variant, nextToggle)}>{working ? <LoaderCircle className="animate-spin" size={12} /> : actionLabel(nextToggle)}</Button>}
              {variant.supportedActions.includes("update") && <Button size="xs" variant="ghost" disabled={!!busyKey} onClick={() => onAction(variant, "update")}>Update</Button>}
              {variant.supportedActions.includes("uninstall") && <Button size="xs" variant="ghost" disabled={!!busyKey} onClick={() => onAction(variant, "uninstall")}>Uninstall</Button>}
            </>}
            {variant.installed && variant.supportedActions.includes("authenticate") && variant.authenticationState.toLowerCase() === "required" && <Button size="xs" variant="secondary" disabled={!!busyKey} onClick={() => onAction(variant, "authenticate")}><ShieldCheck size={12} aria-hidden="true" /> Connect {providerLabel(variant.provider)}</Button>}
          </div>
        </div>;
      })}
    </div>

    {installable && <div className="mt-4 flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
      <div className="flex items-center gap-1 rounded-xl border border-white/[0.06] bg-black/10 p-1">
        {(["codex", "claude", "both"] as InstallTarget[]).map(value => {
          const disabled = (value === "codex" && !hasCodex) || (value === "claude" && !hasClaude) || (value === "both" && (!hasCodex || !hasClaude));
          return <button key={value} type="button" disabled={disabled} onClick={() => onTarget(value)} className={`rounded-lg px-2.5 py-1 text-[10.5px] transition-colors disabled:cursor-not-allowed disabled:opacity-30 ${target === value ? "bg-white/[0.1] text-neutral-100" : "text-neutral-500 hover:text-neutral-300"}`}>{value === "both" ? "Both" : providerLabel(value)}</button>;
        })}
      </div>
      <Button size="sm" disabled={!!busyKey} onClick={onInstall}>{busyKey === `${service.id}:install` ? <LoaderCircle className="animate-spin" size={13} /> : <Package size={13} />} Install for {target === "both" ? "both" : providerLabel(target)}</Button>
    </div>}

    {!!results.length && <div className="mt-3 space-y-1 rounded-2xl border border-white/[0.06] bg-black/10 p-3 text-[11px]">
      {results.map(result => <div key={`${result.provider}:${result.pluginId}`} className="flex items-start gap-2">
        {result.success ? <Check className="mt-0.5 shrink-0 text-emerald-400" size={12} /> : <X className="mt-0.5 shrink-0 text-red-400" size={12} />}
        <span className="text-neutral-400"><b className="font-medium text-neutral-300">{providerLabel(result.provider)}:</b> {result.error ?? result.message}</span>
      </div>)}
      {!!failed.length && <Button size="xs" variant="secondary" className="mt-2" disabled={!!busyKey} onClick={onRetry}><RefreshCw size={11} /> Retry failed provider</Button>}
    </div>}
  </article>;
}

export function MarketplaceScreen() {
  const [catalog, setCatalog] = useState<MarketplaceCatalog>();
  const [loading, setLoading] = useState(true);
  const [query, setQuery] = useState("");
  const [provider, setProvider] = useState<MarketplaceProvider | "all">("all");
  const [targets, setTargets] = useState<Record<string, InstallTarget>>({});
  const [results, setResults] = useState<Record<string, MarketplaceActionResult[]>>({});
  const [busyKey, setBusyKey] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    try { setCatalog(await bridgeApi.marketplaceCatalog()); }
    finally { setLoading(false); }
  }, []);
  useEffect(() => { void refresh(); }, [refresh]);

  const services = useMemo(() => {
    const variants = catalog?.providers.flatMap(item => item.variants) ?? [];
    const needle = query.trim().toLowerCase();
    return groupMarketplaceServices(variants, MARKETPLACE_ALIASES).filter(service => {
      const providerMatch = provider === "all" || service.variants.some(variant => variant.provider === provider);
      const searchMatch = !needle || service.name.toLowerCase().includes(needle) || service.description?.toLowerCase().includes(needle) || service.variants.some(variant => variant.capabilities.some(capability => capability.toLowerCase().includes(needle)));
      return providerMatch && searchMatch;
    });
  }, [catalog, provider, query]);

  const install = async (service: MarketplaceService, retry = false) => {
    const previous = results[service.id] ?? [];
    const target = targets[service.id] ?? (service.variants.length > 1 ? "both" : service.variants[0].provider);
    const selected = retry
      ? failedVariants(service.variants, previous)
      : service.variants.filter(variant => target === "both" || variant.provider === target).filter(variant => !variant.installed);
    if (!selected.length) return;
    setBusyKey(`${service.id}:install`);
    try {
      const next = await installVariants(selected, bridgeApi.marketplaceAction);
      setResults(current => ({ ...current, [service.id]: retry ? [...previous.filter(item => item.success), ...next] : next }));
      await refresh();
    } finally { setBusyKey(null); }
  };

  const act = async (service: MarketplaceService, variant: MarketplaceVariant, action: MarketplaceAction) => {
    setBusyKey(`${service.id}:${variant.provider}`);
    try {
      const result = await bridgeApi.marketplaceAction(variant.provider, variant.pluginId, variant.marketplace, action);
      setResults(current => ({ ...current, [service.id]: [result] }));
      await refresh();
    } finally { setBusyKey(null); }
  };

  return <div className="flex h-full min-h-0 flex-col overflow-hidden">
    <header className="shrink-0 border-b border-white/[0.05] px-5 pb-4 pt-5 sm:px-8 sm:pt-7">
      <div className="mx-auto max-w-5xl">
        <div className="flex items-start justify-between gap-4">
          <div>
            <p className="mb-1 text-[9px] font-semibold uppercase tracking-[0.16em] text-neutral-600">Provider-native plugins</p>
            <h1 className="font-display text-2xl font-semibold tracking-tight text-white">Marketplace</h1>
            <p className="mt-1 max-w-2xl text-xs leading-relaxed text-neutral-500">Install one service for Codex, Claude Code, or both. Each provider keeps its own installation and login.</p>
          </div>
          <Button size="sm" variant="ghost" onClick={() => void refresh()} disabled={loading}><RefreshCw size={13} className={loading ? "animate-spin" : ""} /> Refresh</Button>
        </div>
        <div className="mt-5 flex flex-col gap-2 sm:flex-row">
          <div className="relative flex-1">
            <Search className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-neutral-600" size={14} aria-hidden="true" />
            <Input value={query} onChange={event => setQuery(event.target.value)} placeholder="Search plugins and capabilities" className="h-9 rounded-xl border-white/[0.07] bg-white/[0.035] pl-9 text-xs" />
          </div>
          <div className="flex items-center gap-1 rounded-xl border border-white/[0.06] bg-white/[0.025] p-1">
            {(["all", "codex", "claude"] as const).map(value => <button key={value} type="button" onClick={() => setProvider(value)} className={`rounded-lg px-3 py-1.5 text-[10.5px] transition-colors ${provider === value ? "bg-white/[0.09] text-neutral-100" : "text-neutral-500 hover:text-neutral-300"}`}>{value === "all" ? "All providers" : providerLabel(value)}</button>)}
          </div>
        </div>
      </div>
    </header>

    <div className="min-h-0 flex-1 overflow-y-auto px-5 py-5 scrollbar-thin scrollbar-thumb-white/10 sm:px-8 sm:py-7">
      <div className="mx-auto max-w-5xl space-y-3">
        {catalog?.providers.map(item => item.error && <div key={item.provider} className="flex items-start gap-2 rounded-2xl border border-amber-400/15 bg-amber-400/[0.04] px-3 py-2.5 text-[11px] text-amber-200/80"><AlertCircle className="mt-0.5 shrink-0" size={13} /><span><b>{providerLabel(item.provider)}:</b> {item.error}</span></div>)}
        {loading && !catalog && <div className="flex min-h-56 items-center justify-center gap-2 text-xs text-neutral-500"><LoaderCircle className="animate-spin" size={15} /> Discovering provider marketplaces…</div>}
        {!loading && !services.length && <div className="flex min-h-56 flex-col items-center justify-center text-center"><Unplug className="mb-3 text-neutral-700" size={24} /><p className="text-sm text-neutral-400">No matching plugins</p><p className="mt-1 text-xs text-neutral-600">Try another filter or install the provider CLIs.</p></div>}
        {services.map(service => <ServiceCard
          key={service.id}
          service={service}
          busyKey={busyKey}
          target={targets[service.id] ?? (service.variants.length > 1 ? "both" : service.variants[0].provider)}
          results={results[service.id] ?? []}
          onTarget={target => setTargets(current => ({ ...current, [service.id]: target }))}
          onInstall={() => void install(service)}
          onRetry={() => void install(service, true)}
          onAction={(variant, action) => void act(service, variant, action)}
        />)}
      </div>
    </div>
  </div>;
}
