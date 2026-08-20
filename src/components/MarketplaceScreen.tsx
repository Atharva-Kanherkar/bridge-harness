import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AlertCircle, Check, ChevronLeft, ChevronRight, ExternalLink, LoaderCircle, Package, RefreshCw, Search, ShieldCheck, SlidersHorizontal, Unplug, X } from "lucide-react";
import { bridgeApi } from "../api";
import { applyAppAuthStates, authenticationLabel, compatibilityLabels, failedVariants, groupMarketplaceServices, installVariants, MARKETPLACE_ALIASES, verifiedBrandLogoUrl, type MarketplaceService } from "../marketplace";
import type { MarketplaceAction, MarketplaceActionResult, MarketplaceCatalog, MarketplaceProvider, MarketplaceVariant } from "../types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { AgentMarketplace } from "./AgentMarketplace";
import { AutomationsPanel } from "./AutomationsPanel";
import { SkillMarketplace } from "./SkillMarketplace";

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

function serviceSourceUrl(service: MarketplaceService): string | null {
  const source = service.variants.find(variant => variant.source || variant.repository);
  return source?.repository?.startsWith("http") ? source.repository : source?.source?.startsWith("http") ? source.source : null;
}

// One restrained tile for every plugin. Identity comes from the vendor's own
// logo — or its initials — not from a rotating set of brand gradients.
function ServiceIcon({ service, size = "md" }: { service: MarketplaceService; size?: "sm" | "md" | "lg" }) {
  const [failedIcons, setFailedIcons] = useState<string[]>([]);
  const initials = service.name.trim().split(/\s+/).slice(0, 2).map(word => word[0]).join("").toUpperCase() || "P";
  const icon = [service.variants.find(variant => variant.iconDataUrl)?.iconDataUrl, verifiedBrandLogoUrl(service)]
    .find(candidate => candidate && !failedIcons.includes(candidate));
  const dimensions = size === "sm" ? "h-8 w-8" : size === "lg" ? "h-16 w-16" : "h-11 w-11";
  const rounding = size === "lg" ? "rounded-[20px]" : "rounded-[14px]";
  const text = size === "sm" ? "text-[10px]" : size === "lg" ? "text-lg" : "text-[13px]";
  if (icon) return <span className={`inline-flex shrink-0 items-center justify-center overflow-hidden ${rounding} border border-border bg-muted p-1.5 ${dimensions}`} aria-hidden="true"><img src={icon} alt="" referrerPolicy="no-referrer" onError={() => setFailedIcons(current => current.includes(icon) ? current : [...current, icon])} className="h-full w-full object-contain" /></span>;
  return <span className={`inline-flex shrink-0 items-center justify-center ${rounding} border border-border bg-accent font-display font-semibold text-foreground ${dimensions} ${text}`} aria-hidden="true">{initials}</span>;
}

function Status({ variant }: { variant: MarketplaceVariant }) {
  const auth = authenticationLabel(variant.authenticationState);
  return <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[10.5px] text-muted-foreground">
    <span className="inline-flex items-center gap-1.5">
      <span className={`h-1.5 w-1.5 rounded-full ${variant.installed ? "bg-success" : "bg-muted-foreground/40"}`} />
      {variant.installed ? "Installed" : "Available"}
    </span>
    <span>{variant.enabled ? "Enabled" : "Disabled"}</span>
    {auth && <span className={auth === "Connected" ? "text-success" : "text-warning"}>{auth}</span>}
  </div>;
}

function actionLabel(action: MarketplaceAction): string {
  if (action === "authenticate") return "Connect";
  return action[0].toUpperCase() + action.slice(1);
}

function ServiceRow({ service, busyKey, onOpen, onInstall }: {
  service: MarketplaceService;
  busyKey: string | null;
  onOpen: () => void;
  onInstall: () => void;
}) {
  const installable = service.variants.some(variant => !variant.installed);
  const working = busyKey === `${service.id}:install`;
  return <article className="group rounded-2xl border border-border bg-card transition-colors duration-200 hover:border-input hover:bg-accent">
    <div className="flex min-h-[76px] items-center gap-3.5 px-4 py-3.5">
      <button type="button" className="flex min-w-0 flex-1 items-center gap-3.5 rounded-xl text-left" onClick={onOpen}>
        <ServiceIcon service={service} />
        <span className="min-w-0 flex-1">
          <span className="flex items-center gap-2">
            <span className="truncate text-[13.5px] font-semibold tracking-[-0.008em] text-foreground">{service.name}</span>
            {service.variants.every(variant => variant.installed) && <Check size={12} className="shrink-0 text-success" aria-label="Installed" />}
          </span>
          <span className="mt-1 truncate block text-[11.5px] leading-[1.5] text-muted-foreground">{service.description || "Provider plugin for Bridge agents"}</span>
        </span>
      </button>
      {installable
        ? <Button size="xs" variant="secondary" disabled={!!busyKey} onClick={onInstall}>{working ? <LoaderCircle className="animate-spin" size={12} /> : "Install"}</Button>
        : <ChevronRight size={15} className="shrink-0 text-muted-foreground/70 transition-colors group-hover:text-foreground" aria-hidden="true" />}
    </div>
  </article>;
}

function PluginDetailPage({ service, busyKey, target, results, onBack, onTarget, onInstall, onRetry, onAction }: {
  service: MarketplaceService;
  busyKey: string | null;
  target: InstallTarget;
  results: MarketplaceActionResult[];
  onBack: () => void;
  onTarget: (target: InstallTarget) => void;
  onInstall: () => void;
  onRetry: () => void;
  onAction: (variant: MarketplaceVariant, action: MarketplaceAction) => void;
}) {
  const labels = compatibilityLabels(service);
  const failed = results.filter(result => !result.success);
  const sourceUrl = serviceSourceUrl(service);
  const hasCodex = service.variants.some(variant => variant.provider === "codex");
  const hasClaude = service.variants.some(variant => variant.provider === "claude");
  const installable = service.variants.some(variant => !variant.installed);
  const working = busyKey === `${service.id}:install`;
  const capabilities = [...new Set(service.variants.flatMap(variant => variant.capabilities))];

  return <div className="h-full min-h-0 overflow-y-auto">
    <main className="animate-page-enter mx-auto w-full max-w-3xl px-3 pb-16 pt-6 sm:px-6 sm:pt-8">
      <button type="button" onClick={onBack} className="inline-flex items-center gap-0.5 rounded-full py-1 pl-1 pr-3 text-[12px] font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">
        <ChevronLeft size={14} aria-hidden="true" /> Plugins
      </button>

      <header className="mt-6 flex flex-wrap items-start gap-4">
        <ServiceIcon service={service} size="lg" />
        <div className="min-w-0 flex-1 basis-48 pt-1">
          <h1 className="font-display text-[22px] font-semibold tracking-[-0.02em] text-foreground break-words sm:text-[26px]">{service.name}</h1>
          {sourceUrl && <a className="mt-1.5 flex max-w-full items-center gap-1 truncate text-[11.5px] text-muted-foreground transition-colors hover:text-foreground" href={sourceUrl} target="_blank" rel="noreferrer"><span className="truncate">{sourceUrl.replace(/^https?:\/\//, "")}</span> <ExternalLink size={10} className="shrink-0" /></a>}
        </div>
        <div className="shrink-0 pt-2">
          {installable
            ? <Button size="sm" disabled={!!busyKey} onClick={onInstall}>{working ? <LoaderCircle className="animate-spin" size={13} /> : <Package size={13} />} Install</Button>
            : <span className="inline-flex items-center gap-1.5 rounded-full border border-success/30 bg-success/10 px-3 py-1.5 text-[11px] font-medium text-success"><Check size={12} aria-hidden="true" /> Installed</span>}
        </div>
      </header>

      <div className="mt-5 flex flex-wrap items-center gap-1.5">
        {hasCodex && <Badge variant="secondary" size="sm">Codex</Badge>}
        {hasClaude && <Badge variant="secondary" size="sm">Claude Code</Badge>}
        {labels.map(label => <span key={label} className={`rounded-full border px-2 py-0.5 text-[9.5px] ${label === "Separate login required" ? "border-warning/30 bg-warning/10 text-warning" : "border-border text-muted-foreground"}`}>{label}</span>)}
      </div>

      <section className="mt-8">
        <h2 className="text-[11px] font-semibold uppercase tracking-[0.14em] text-muted-foreground">About</h2>
        <p className="mt-3 text-[13.5px] leading-[1.7] text-foreground">{service.description || "Provider plugin for Bridge agents"}</p>
        {capabilities.length > 0 && <div className="mt-4 flex flex-wrap gap-1.5">
          {capabilities.map(capability => <span key={capability} className="rounded-full border border-border bg-muted px-2.5 py-1 text-[10.5px] text-muted-foreground">{capability}</span>)}
        </div>}
      </section>

      <section className="mt-8">
        <h2 className="text-[11px] font-semibold uppercase tracking-[0.14em] text-muted-foreground">Providers</h2>
        <div className="mt-3 grid gap-3">
          {service.variants.map(variant => {
            const key = `${service.id}:${variant.provider}`;
            const variantWorking = busyKey === key;
            const nextToggle: MarketplaceAction = variant.enabled ? "disable" : "enable";
            const canToggle = variant.supportedActions.includes(nextToggle);
            const isCodexApp = variant.provider === "codex" && variant.connectorType === "app";
            const canAuthenticate = variant.supportedActions.includes("authenticate") && ((isCodexApp && variant.authenticationState.toLowerCase() !== "connected") || variant.authenticationState.toLowerCase() === "required");
            return <div key={`${variant.provider}:${variant.pluginId}`} className="u-surface rounded-2xl p-4">
              <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
                <div className="min-w-0 flex-1">
                  <div className="mb-1 text-[12.5px] font-semibold text-foreground">{providerLabel(variant.provider)} {variant.version && <span className="ml-1 font-mono text-[10px] font-normal text-muted-foreground">v{variant.version}</span>}</div>
                  <Status variant={variant} />
                </div>
                <div className="flex flex-wrap items-center gap-1.5">
                  {variant.installed && canToggle && <Button size="xs" variant="ghost" disabled={!!busyKey} onClick={() => onAction(variant, nextToggle)}>{variantWorking ? <LoaderCircle className="animate-spin" size={11} /> : actionLabel(nextToggle)}</Button>}
                  {variant.installed && variant.supportedActions.includes("update") && <Button size="xs" variant="ghost" disabled={!!busyKey} onClick={() => onAction(variant, "update")}>Update</Button>}
                  {variant.installed && variant.supportedActions.includes("uninstall") && <Button size="xs" variant="ghost" disabled={!!busyKey} onClick={() => onAction(variant, "uninstall")}>Uninstall</Button>}
                  {variant.installed && canAuthenticate && <Button size="xs" variant="secondary" disabled={!!busyKey} onClick={() => onAction(variant, "authenticate")}><ShieldCheck size={11} /> {isCodexApp ? "Connect" : `Connect ${providerLabel(variant.provider)}`}</Button>}
                </div>
              </div>
            </div>;
          })}
        </div>
      </section>

      {installable && <section className="mt-6">
        <div className="u-surface flex flex-wrap items-center justify-between gap-3 rounded-2xl p-4">
          <div className="u-segmented">
            {(["codex", "claude", "both"] as InstallTarget[]).map(value => {
              const disabled = (value === "codex" && !hasCodex) || (value === "claude" && !hasClaude) || (value === "both" && (!hasCodex || !hasClaude));
              return <button key={value} type="button" data-active={target === value} disabled={disabled} onClick={() => onTarget(value)} className="u-segmented-item disabled:opacity-25">{value === "both" ? "Both" : providerLabel(value)}</button>;
            })}
          </div>
          <Button size="sm" disabled={!!busyKey} onClick={onInstall}>{working ? <LoaderCircle className="animate-spin" size={12} /> : <Package size={12} />} Install for {target === "both" ? "both" : providerLabel(target)}</Button>
        </div>
      </section>}

      {!!results.length && <section className="mt-6">
        <div className="u-surface rounded-2xl p-4 text-[11px]">
          {results.map(result => <div key={`${result.provider}:${result.pluginId}`} className="flex items-start gap-2 py-1">{result.success ? <Check className="mt-0.5 shrink-0 text-success" size={12} /> : <X className="mt-0.5 shrink-0 text-destructive" size={12} />}<span className="min-w-0 break-words text-muted-foreground"><b className="font-medium text-foreground">{providerLabel(result.provider)}:</b> {result.error ?? result.message}</span></div>)}
          {!!failed.length && <Button size="xs" variant="secondary" className="mt-2" disabled={!!busyKey} onClick={onRetry}><RefreshCw size={10} /> Retry failed provider</Button>}
        </div>
      </section>}
    </main>
  </div>;
}

function PluginMarketplace() {
  const [catalog, setCatalog] = useState<MarketplaceCatalog>();
  const [loading, setLoading] = useState(true);
  const [query, setQuery] = useState("");
  const [provider, setProvider] = useState<MarketplaceProvider | "all">("all");
  const [scope, setScope] = useState<CatalogScope>("public");
  const [selectedId, setSelectedId] = useState<string | null>(null);
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
    try { const next = await installVariants(selected, bridgeApi.marketplaceAction); setResults(current => ({ ...current, [service.id]: retry ? [...previous.filter(item => item.success), ...next] : next })); setSelectedId(service.id); await refresh(); void refreshAuth(); }
    finally { setBusyKey(null); }
  };
  const act = async (service: MarketplaceService, variant: MarketplaceVariant, action: MarketplaceAction) => {
    setBusyKey(`${service.id}:${variant.provider}`);
    try { const result = await bridgeApi.marketplaceAction(variant.provider, variant.pluginId, variant.marketplace, action); setResults(current => ({ ...current, [service.id]: [result] })); await refresh(); void refreshAuth(); }
    finally { setBusyKey(null); }
  };

  const selected = selectedId ? allServices.find(service => service.id === selectedId) : undefined;
  if (selected) {
    return <PluginDetailPage
      service={selected}
      busyKey={busyKey}
      target={targets[selected.id] ?? (selected.variants.length > 1 ? "both" : selected.variants[0].provider)}
      results={results[selected.id] ?? []}
      onBack={() => setSelectedId(null)}
      onTarget={target => setTargets(current => ({ ...current, [selected.id]: target }))}
      onInstall={() => void install(selected)}
      onRetry={() => void install(selected, true)}
      onAction={(variant, action) => void act(selected, variant, action)}
    />;
  }

  return <div className="h-full min-h-0 overflow-y-auto">
    <main className="mx-auto w-full max-w-5xl px-3 pb-16 pt-8 sm:px-6 sm:pt-12">
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div className="min-w-0"><h1 className="font-display text-[26px] font-semibold tracking-[-0.025em] text-foreground sm:text-[32px]">Plugins</h1><p className="mt-1.5 text-[13.5px] leading-relaxed text-muted-foreground">Work with your favorite tools across Codex and Claude Code</p></div>
        <button type="button" onClick={() => void refresh()} disabled={loading} aria-label="Refresh plugins" className="mt-1.5 inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-40"><RefreshCw size={14} className={loading ? "animate-spin" : ""} /></button>
      </div>
      <div className="relative mt-6"><Search className="pointer-events-none absolute left-3.5 top-1/2 z-10 -translate-y-1/2 text-muted-foreground" size={14} /><Input value={query} onChange={event => setQuery(event.target.value)} placeholder="Search plugins" className="h-10 rounded-full pl-10 text-[13px]" /></div>

      {catalog?.providers.map(item => item.error && <div key={item.provider} className="mt-3 flex items-start gap-2 rounded-xl border border-warning/30 bg-warning/10 px-3 py-2 text-[10.5px] text-warning"><AlertCircle className="mt-0.5 shrink-0" size={12} /><span className="min-w-0 break-words"><b>{providerLabel(item.provider)}:</b> {item.error}</span></div>)}
      {loading && !catalog && <div className="flex min-h-56 items-center justify-center gap-2 text-xs text-muted-foreground"><LoaderCircle className="animate-spin" size={15} /> Discovering provider marketplaces…</div>}

      {!!catalog && <>
        <section className="mt-8" aria-labelledby="installed-heading">
          <div className="flex items-center justify-between gap-3"><h2 id="installed-heading" className="text-[11px] font-semibold uppercase tracking-[0.14em] text-muted-foreground">Installed</h2><span className="shrink-0 text-[10px] text-muted-foreground/70">{installed.length} plugins</span></div>
          {installed.length ? <div className="mt-3.5 flex flex-wrap gap-2.5">{installed.map(service => <button key={service.id} type="button" title={service.name} aria-label={`Open ${service.name}`} onClick={() => setSelectedId(service.id)} className="rounded-[14px] transition-transform hover:-translate-y-0.5"><ServiceIcon service={service} size="sm" /></button>)}</div> : <p className="mt-3 text-[11px] text-muted-foreground/70">No plugins installed yet.</p>}
        </section>

        <section className="mt-9" aria-labelledby="catalog-heading">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div className="u-segmented">{(["public", "personal"] as CatalogScope[]).map(value => <button key={value} type="button" data-active={scope === value} onClick={() => setScope(value)} className="u-segmented-item capitalize">{value}</button>)}</div>
            <div className="flex items-center gap-2 text-muted-foreground"><SlidersHorizontal size={12} aria-hidden="true" /><div className="u-segmented">{(["all", "codex", "claude"] as const).map(value => <button key={value} type="button" data-active={provider === value} onClick={() => setProvider(value)} className="u-segmented-item">{value === "all" ? "All" : providerLabel(value)}</button>)}</div></div>
          </div>
          <h2 id="catalog-heading" className="mt-7 text-[11px] font-semibold uppercase tracking-[0.14em] text-muted-foreground">{query ? "Search results" : scope === "public" ? "Featured" : "Personal plugins"}</h2>
          {services.length ? <div className="mt-3.5 grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-3">{services.map(service => <ServiceRow key={service.id} service={service} busyKey={busyKey} onOpen={() => setSelectedId(service.id)} onInstall={() => void install(service)} />)}</div> : <div className="u-surface mt-3.5 flex min-h-44 flex-col items-center justify-center rounded-2xl px-4 text-center"><Unplug className="mb-2.5 text-muted-foreground/70" size={22} /><p className="text-[13px] font-medium text-foreground">No matching plugins</p><p className="mt-1 text-[11px] text-muted-foreground">Try another search, scope, or provider.</p></div>}
        </section>
      </>}
    </main>
  </div>;
}

const RESOURCES = ["agents", "plugins", "skills", "automations"] as const;
type Resource = (typeof RESOURCES)[number];

export function MarketplaceScreen() {
  // Agents first, and the default: a runtime is the thing a plugin or a skill
  // runs *inside*. Landing on Plugins asks the user to furnish a room before
  // they have one.
  const [resource, setResource] = useState<Resource>("agents");
  return <div className="flex h-full min-h-0 flex-col">
    <nav className="flex h-14 shrink-0 items-center justify-center border-b border-border px-3" aria-label="Marketplace sections" data-tauri-drag-region>
      <div className="u-segmented">
        {RESOURCES.map(value => <button key={value} type="button" data-active={resource === value} onClick={() => setResource(value)} className="u-segmented-item capitalize">{value}</button>)}
      </div>
    </nav>
    <div className="min-h-0 flex-1">
      {resource === "agents" && <AgentMarketplace/>}
      {resource === "plugins" && <PluginMarketplace/>}
      {resource === "skills" && <SkillMarketplace/>}
      {resource === "automations" && <AutomationsPanel/>}
    </div>
  </div>;
}
