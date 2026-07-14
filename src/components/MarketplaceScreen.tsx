import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AlertCircle, Check, ChevronDown, ExternalLink, LoaderCircle, Package, RefreshCw, Search, ShieldCheck, SlidersHorizontal, Unplug, X } from "lucide-react";
import { bridgeApi } from "../api";
import { applyAppAuthStates, authenticationLabel, compatibilityLabels, failedVariants, groupMarketplaceServices, installVariants, MARKETPLACE_ALIASES, type MarketplaceService } from "../marketplace";
import type { MarketplaceAction, MarketplaceActionResult, MarketplaceCatalog, MarketplaceProvider, MarketplaceVariant } from "../types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

type InstallTarget = MarketplaceProvider | "both";
type CatalogScope = "public" | "personal";

function providerLabel(provider: MarketplaceProvider): string {
  return provider === "codex" ? "Codex" : "Claude Code";
}

function serviceScope(service: MarketplaceService): CatalogScope {
  const values = service.variants.flatMap(variant => [
    variant.marketplace,
    typeof variant.providerMetadata.visibility === "string" ? variant.providerMetadata.visibility : null,
    typeof variant.providerMetadata.scope === "string" ? variant.providerMetadata.scope : null,
  ]).filter((value): value is string => !!value).map(value => value.toLowerCase());
  return values.some(value => value === "personal" || value === "private" || value === "local") ? "personal" : "public";
}

const iconThemes = [
  "from-blue-400 to-blue-600 text-white",
  "from-violet-400 to-fuchsia-600 text-white",
  "from-emerald-400 to-teal-600 text-white",
  "from-amber-300 to-orange-500 text-neutral-950",
  "from-rose-400 to-red-600 text-white",
  "from-cyan-300 to-sky-600 text-neutral-950",
];

function ServiceIcon({ service, size = "md" }: { service: MarketplaceService; size?: "sm" | "md" }) {
  const seed = [...service.name].reduce((total, letter) => total + letter.charCodeAt(0), 0);
  const initials = service.name.trim().split(/\s+/).slice(0, 2).map(word => word[0]).join("").toUpperCase() || "P";
  return <span className={`inline-flex shrink-0 items-center justify-center rounded-xl border border-white/10 bg-gradient-to-br font-display font-semibold shadow-[inset_0_1px_0_rgba(255,255,255,0.25)] ${iconThemes[seed % iconThemes.length]} ${size === "sm" ? "h-8 w-8 text-[10px]" : "h-10 w-10 text-xs"}`} aria-hidden="true">{initials}</span>;
}

function Status({ variant }: { variant: MarketplaceVariant }) {
  const auth = authenticationLabel(variant.authenticationState);
  return <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[10.5px] text-neutral-500">
    <span className="inline-flex items-center gap-1.5">
      <span className={`h-1.5 w-1.5 rounded-full ${variant.installed ? "bg-emerald-400" : "bg-neutral-600"}`} />
      {variant.installed ? "Installed" : "Available"}
    </span>
    <span>{variant.enabled ? "Enabled" : "Disabled"}</span>
    {auth && <span className={auth === "Connected" ? "text-emerald-400" : "text-amber-400"}>{auth}</span>}
  </div>;
}

function actionLabel(action: MarketplaceAction): string {
  if (action === "authenticate") return "Connect";
  return action[0].toUpperCase() + action.slice(1);
}

function ServiceRow({ service, busyKey, target, results, expanded, onToggle, onTarget, onInstall, onRetry, onAction }: {
  service: MarketplaceService;
  busyKey: string | null;
  target: InstallTarget;
  results: MarketplaceActionResult[];
  expanded: boolean;
  onToggle: () => void;
  onTarget: (target: InstallTarget) => void;
  onInstall: () => void;
  onRetry: () => void;
  onAction: (variant: MarketplaceVariant, action: MarketplaceAction) => void;
}) {
  const labels = compatibilityLabels(service);
  const failed = results.filter(result => !result.success);
  const source = service.variants.find(variant => variant.source || variant.repository);
  const sourceUrl = source?.repository?.startsWith("http") ? source.repository : source?.source?.startsWith("http") ? source.source : null;
  const hasCodex = service.variants.some(variant => variant.provider === "codex");
  const hasClaude = service.variants.some(variant => variant.provider === "claude");
  const installable = service.variants.some(variant => !variant.installed);
  const working = busyKey === `${service.id}:install`;

  return <article className={`group border-b border-white/[0.055] transition-colors ${expanded ? "bg-white/[0.025]" : "hover:bg-white/[0.018]"}`}>
    <div className="flex min-h-[76px] items-center gap-3 px-2 py-3">
      <button type="button" className="flex min-w-0 flex-1 items-center gap-3 rounded-xl text-left focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-white/30" onClick={onToggle} aria-expanded={expanded}>
        <ServiceIcon service={service} />
        <span className="min-w-0 flex-1">
          <span className="flex items-center gap-2">
            <span className="truncate text-[13px] font-medium text-neutral-100">{service.name}</span>
            {service.variants.every(variant => variant.installed) && <Check size={12} className="shrink-0 text-emerald-400" aria-label="Installed" />}
          </span>
          <span className="mt-0.5 line-clamp-1 block text-[11px] leading-relaxed text-neutral-500">{service.description || "Provider plugin for Bridge agents"}</span>
        </span>
      </button>
      {installable ? <Button size="xs" variant="secondary" disabled={!!busyKey} onClick={onInstall}>{working ? <LoaderCircle className="animate-spin" size={12} /> : "Install"}</Button> : <button type="button" onClick={onToggle} className="rounded-lg p-1.5 text-neutral-600 transition-colors hover:bg-white/5 hover:text-neutral-300" aria-label={`${expanded ? "Hide" : "Show"} ${service.name} details`}><ChevronDown size={14} className={`transition-transform ${expanded ? "rotate-180" : ""}`} /></button>}
    </div>

    {expanded && <div className="mx-2 mb-3 rounded-xl border border-white/[0.06] bg-black/10 p-3">
      <div className="mb-3 flex flex-wrap items-center gap-1.5">
        {hasCodex && <Badge variant="secondary" size="sm">Codex</Badge>}
        {hasClaude && <Badge variant="secondary" size="sm">Claude Code</Badge>}
        {labels.map(label => <span key={label} className={`rounded-full border px-2 py-0.5 text-[9px] ${label === "Separate login required" ? "border-amber-400/20 bg-amber-400/[0.06] text-amber-300" : "border-white/[0.07] text-neutral-500"}`}>{label}</span>)}
        {sourceUrl && <a className="ml-auto inline-flex items-center gap-1 text-[9.5px] text-neutral-500 transition-colors hover:text-neutral-200" href={sourceUrl} target="_blank" rel="noreferrer">Source <ExternalLink size={9} /></a>}
      </div>
      <div className="divide-y divide-white/[0.055]">
        {service.variants.map(variant => {
          const key = `${service.id}:${variant.provider}`;
          const variantWorking = busyKey === key;
          const nextToggle: MarketplaceAction = variant.enabled ? "disable" : "enable";
          const canToggle = variant.supportedActions.includes(nextToggle);
          const isCodexApp = variant.provider === "codex" && variant.connectorType === "app";
          const canAuthenticate = variant.supportedActions.includes("authenticate") && ((isCodexApp && variant.authenticationState.toLowerCase() !== "connected") || variant.authenticationState.toLowerCase() === "required");
          return <div key={`${variant.provider}:${variant.pluginId}`} className="flex flex-col gap-2 py-2 first:pt-0 last:pb-0 sm:flex-row sm:items-center">
            <div className="min-w-0 flex-1">
              <div className="mb-0.5 text-[11px] font-medium text-neutral-300">{providerLabel(variant.provider)} {variant.version && <span className="font-mono text-[9px] font-normal text-neutral-600">v{variant.version}</span>}</div>
              <Status variant={variant} />
            </div>
            <div className="flex flex-wrap items-center gap-1">
              {variant.installed && canToggle && <Button size="xs" variant="ghost" disabled={!!busyKey} onClick={() => onAction(variant, nextToggle)}>{variantWorking ? <LoaderCircle className="animate-spin" size={11} /> : actionLabel(nextToggle)}</Button>}
              {variant.installed && variant.supportedActions.includes("update") && <Button size="xs" variant="ghost" disabled={!!busyKey} onClick={() => onAction(variant, "update")}>Update</Button>}
              {variant.installed && variant.supportedActions.includes("uninstall") && <Button size="xs" variant="ghost" disabled={!!busyKey} onClick={() => onAction(variant, "uninstall")}>Uninstall</Button>}
              {variant.installed && canAuthenticate && <Button size="xs" variant="secondary" disabled={!!busyKey} onClick={() => onAction(variant, "authenticate")}><ShieldCheck size={11} /> {isCodexApp ? "Connect" : `Connect ${providerLabel(variant.provider)}`}</Button>}
            </div>
          </div>;
        })}
      </div>
      {installable && <div className="mt-3 flex flex-wrap items-center justify-between gap-2 border-t border-white/[0.055] pt-3">
        <div className="flex items-center gap-1 rounded-lg bg-white/[0.025] p-0.5">
          {(["codex", "claude", "both"] as InstallTarget[]).map(value => {
            const disabled = (value === "codex" && !hasCodex) || (value === "claude" && !hasClaude) || (value === "both" && (!hasCodex || !hasClaude));
            return <button key={value} type="button" disabled={disabled} onClick={() => onTarget(value)} className={`rounded-md px-2 py-1 text-[9.5px] transition-colors disabled:opacity-25 ${target === value ? "bg-white/[0.09] text-neutral-100" : "text-neutral-500 hover:text-neutral-300"}`}>{value === "both" ? "Both" : providerLabel(value)}</button>;
          })}
        </div>
        <Button size="xs" disabled={!!busyKey} onClick={onInstall}>{working ? <LoaderCircle className="animate-spin" size={11} /> : <Package size={11} />} Install for {target === "both" ? "both" : providerLabel(target)}</Button>
      </div>}
      {!!results.length && <div className="mt-3 space-y-1 border-t border-white/[0.055] pt-3 text-[10.5px]">
        {results.map(result => <div key={`${result.provider}:${result.pluginId}`} className="flex items-start gap-2">{result.success ? <Check className="mt-0.5 shrink-0 text-emerald-400" size={11} /> : <X className="mt-0.5 shrink-0 text-red-400" size={11} />}<span className="text-neutral-400"><b className="font-medium text-neutral-300">{providerLabel(result.provider)}:</b> {result.error ?? result.message}</span></div>)}
        {!!failed.length && <Button size="xs" variant="secondary" className="mt-2" disabled={!!busyKey} onClick={onRetry}><RefreshCw size={10} /> Retry failed provider</Button>}
      </div>}
    </div>}
  </article>;
}

export function MarketplaceScreen() {
  const [catalog, setCatalog] = useState<MarketplaceCatalog>();
  const [loading, setLoading] = useState(true);
  const [query, setQuery] = useState("");
  const [provider, setProvider] = useState<MarketplaceProvider | "all">("all");
  const [scope, setScope] = useState<CatalogScope>("public");
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [targets, setTargets] = useState<Record<string, InstallTarget>>({});
  const [results, setResults] = useState<Record<string, MarketplaceActionResult[]>>({});
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const catalogRef = useRef<MarketplaceCatalog>();
  const authRefreshBusy = useRef(false);

  useEffect(() => { catalogRef.current = catalog; }, [catalog]);
  const refreshAuth = useCallback(async () => {
    if (authRefreshBusy.current) return;
    authRefreshBusy.current = true;
    try { const states = await bridgeApi.marketplaceAppAuthStates(); setCatalog(current => current ? applyAppAuthStates(current, states) : current); }
    catch { /* Connector status is supplementary. */ }
    finally { authRefreshBusy.current = false; }
  }, []);
  const refresh = useCallback(async () => { setLoading(true); try { setCatalog(await bridgeApi.marketplaceCatalog()); } finally { setLoading(false); } }, []);
  useEffect(() => { void refresh(); }, [refresh]);
  useEffect(() => {
    void refreshAuth();
    const timer = window.setInterval(() => {
      const needsRefresh = catalogRef.current?.providers.some(item => item.variants.some(variant => variant.installed && variant.appConnectorIds.length > 0 && variant.authenticationState.toLowerCase() !== "connected"));
      if (needsRefresh) void refreshAuth();
    }, 15_000);
    return () => window.clearInterval(timer);
  }, [refreshAuth]);

  const allServices = useMemo(() => groupMarketplaceServices(catalog?.providers.flatMap(item => item.variants) ?? [], MARKETPLACE_ALIASES), [catalog]);
  const services = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return allServices.filter(service => {
      const providerMatch = provider === "all" || service.variants.some(variant => variant.provider === provider);
      const searchMatch = !needle || service.name.toLowerCase().includes(needle) || service.description?.toLowerCase().includes(needle) || service.variants.some(variant => variant.capabilities.some(capability => capability.toLowerCase().includes(needle)));
      return providerMatch && searchMatch && serviceScope(service) === scope;
    });
  }, [allServices, provider, query, scope]);
  const installed = useMemo(() => allServices.filter(service => service.variants.some(variant => variant.installed)), [allServices]);

  const install = async (service: MarketplaceService, retry = false) => {
    const previous = results[service.id] ?? [];
    const target = targets[service.id] ?? (service.variants.length > 1 ? "both" : service.variants[0].provider);
    const selected = retry ? failedVariants(service.variants, previous) : service.variants.filter(variant => target === "both" || variant.provider === target).filter(variant => !variant.installed);
    if (!selected.length) return;
    setBusyKey(`${service.id}:install`);
    try { const next = await installVariants(selected, bridgeApi.marketplaceAction); setResults(current => ({ ...current, [service.id]: retry ? [...previous.filter(item => item.success), ...next] : next })); setExpandedId(service.id); await refresh(); void refreshAuth(); }
    finally { setBusyKey(null); }
  };
  const act = async (service: MarketplaceService, variant: MarketplaceVariant, action: MarketplaceAction) => {
    setBusyKey(`${service.id}:${variant.provider}`);
    try { const result = await bridgeApi.marketplaceAction(variant.provider, variant.pluginId, variant.marketplace, action); setResults(current => ({ ...current, [service.id]: [result] })); await refresh(); void refreshAuth(); }
    finally { setBusyKey(null); }
  };
  const row = (service: MarketplaceService) => <ServiceRow key={service.id} service={service} busyKey={busyKey} target={targets[service.id] ?? (service.variants.length > 1 ? "both" : service.variants[0].provider)} results={results[service.id] ?? []} expanded={expandedId === service.id} onToggle={() => setExpandedId(current => current === service.id ? null : service.id)} onTarget={target => setTargets(current => ({ ...current, [service.id]: target }))} onInstall={() => void install(service)} onRetry={() => void install(service, true)} onAction={(variant, action) => void act(service, variant, action)} />;

  return <div className="h-full min-h-0 overflow-y-auto scrollbar-thin scrollbar-thumb-white/10">
    <main className="mx-auto w-full max-w-5xl px-5 pb-12 pt-7 sm:px-8 sm:pt-10">
      <div className="flex items-start justify-between gap-4">
        <div><h1 className="font-display text-[26px] font-semibold tracking-tight text-white">Plugins</h1><p className="mt-1 text-[13px] text-neutral-500">Work with your favorite tools across Codex and Claude Code</p></div>
        <Button size="xs" variant="ghost" onClick={() => void refresh()} disabled={loading} aria-label="Refresh plugins"><RefreshCw size={13} className={loading ? "animate-spin" : ""} /></Button>
      </div>
      <div className="relative mt-5"><Search className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-neutral-500" size={14} /><Input value={query} onChange={event => setQuery(event.target.value)} placeholder="Search plugins" className="h-8 rounded-xl border-white/[0.12] bg-white/[0.055] pl-9 text-[11.5px]" /></div>

      {catalog?.providers.map(item => item.error && <div key={item.provider} className="mt-3 flex items-start gap-2 rounded-xl border border-amber-400/15 bg-amber-400/[0.04] px-3 py-2 text-[10.5px] text-amber-200/80"><AlertCircle className="mt-0.5 shrink-0" size={12} /><span><b>{providerLabel(item.provider)}:</b> {item.error}</span></div>)}
      {loading && !catalog && <div className="flex min-h-56 items-center justify-center gap-2 text-xs text-neutral-500"><LoaderCircle className="animate-spin" size={15} /> Discovering provider marketplaces…</div>}

      {!!catalog && <>
        <section className="mt-7 border-b border-white/[0.06] pb-5" aria-labelledby="installed-heading">
          <div className="flex items-center justify-between"><h2 id="installed-heading" className="text-[12px] font-medium text-neutral-200">Installed</h2><span className="text-[9.5px] text-neutral-600">{installed.length} plugins</span></div>
          {installed.length ? <div className="mt-3 flex flex-wrap gap-2">{installed.map(service => <button key={service.id} type="button" title={service.name} aria-label={`Show ${service.name} details`} onClick={() => { setScope(serviceScope(service)); setExpandedId(service.id); }} className={`rounded-xl ring-offset-2 ring-offset-background transition-transform hover:-translate-y-0.5 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-white/40 ${expandedId === service.id ? "ring-1 ring-white/30" : ""}`}><ServiceIcon service={service} size="sm" /></button>)}</div> : <p className="mt-3 text-[10.5px] text-neutral-600">No plugins installed yet.</p>}
        </section>

        <section className="mt-4" aria-labelledby="catalog-heading">
          <div className="flex items-center justify-between border-b border-white/[0.06] pb-2">
            <div className="flex items-center gap-1">{(["public", "personal"] as CatalogScope[]).map(value => <button key={value} type="button" onClick={() => setScope(value)} className={`rounded-lg px-2.5 py-1 text-[10.5px] capitalize transition-colors ${scope === value ? "bg-white/[0.07] text-neutral-100" : "text-neutral-500 hover:text-neutral-300"}`}>{value}</button>)}</div>
            <div className="flex items-center gap-1 text-neutral-500"><SlidersHorizontal size={11} aria-hidden="true" />{(["all", "codex", "claude"] as const).map(value => <button key={value} type="button" onClick={() => setProvider(value)} className={`rounded-md px-1.5 py-1 text-[9.5px] transition-colors ${provider === value ? "text-neutral-100" : "hover:text-neutral-300"}`}>{value === "all" ? "All" : providerLabel(value)}</button>)}</div>
          </div>
          <h2 id="catalog-heading" className="mt-5 text-[12px] font-medium text-neutral-200">{query ? "Search results" : scope === "public" ? "Featured" : "Personal plugins"}</h2>
          {services.length ? <div className="mt-2 grid grid-cols-1 gap-x-7 lg:grid-cols-2">{services.map(row)}</div> : <div className="flex min-h-44 flex-col items-center justify-center text-center"><Unplug className="mb-2 text-neutral-700" size={20} /><p className="text-xs text-neutral-400">No matching plugins</p><p className="mt-1 text-[10.5px] text-neutral-600">Try another search, scope, or provider.</p></div>}
        </section>
      </>}
    </main>
  </div>;
}
