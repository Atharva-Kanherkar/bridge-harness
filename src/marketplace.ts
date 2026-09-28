import type { MarketplaceActionResult, MarketplaceAppAuthState, MarketplaceCatalog, MarketplaceProvider, MarketplaceVariant } from "./types";

export interface MarketplaceService {
  id: string;
  name: string;
  description: string | null;
  variants: MarketplaceVariant[];
  matchReason: "alias" | "repository" | "endpoint" | "package" | "single";
}

export type MarketplaceAliases = Record<string, string>;

// Provider IDs are intentionally explicit. Matching by the shared display name
// would merge unrelated community plugins with the same label.
export const MARKETPLACE_ALIASES: MarketplaceAliases = {
  "codex:vercel@openai-curated": "vercel",
  "claude:vercel@claude-plugins-official": "vercel",
  "codex:github@openai-curated": "github",
  "claude:github@claude-plugins-official": "github",
  "codex:linear@openai-curated": "linear",
  "claude:linear@claude-plugins-official": "linear",
  "codex:notion@openai-curated": "notion",
  "claude:notion@claude-plugins-official": "notion",
  "codex:slack@openai-curated": "slack",
  "claude:slack@claude-plugins-official": "slack",
  "codex:stripe@openai-curated": "stripe",
  "claude:stripe@claude-plugins-official": "stripe",
};

export function authenticationLabel(state: string): "Connected" | "Needs login" | null {
  const normalized = state.trim().toLowerCase();
  if (normalized === "connected") return "Connected";
  if (normalized === "required") return "Needs login";
  return null;
}

function connectorKey(provider: MarketplaceProvider, connectorId: string): string {
  return `${provider}:${connectorId}`;
}

/** The `<hex>` of a Codex app connector plugin id, `app-<hex>@marketplace`. */
function codexPluginHex(variant: MarketplaceVariant): string | null {
  return /^app-([0-9a-f]+)$/i.exec(variant.pluginId.split("@")[0])?.[1]?.toLowerCase() ?? null;
}

/** The `<hex>` of a Codex app directory id, `asdk_app_<hex>`. */
function codexStateHex(state: MarketplaceAppAuthState): string | null {
  return /asdk_app_([0-9a-f]+)$/i.exec(state.connectorId)?.[1]?.toLowerCase() ?? null;
}

export function isUnnamedVariant(variant: MarketplaceVariant): boolean {
  return variant.nameIsFallback === true || !variant.name.trim();
}

/** A service is unnamed only while every variant in it is. */
export function isUnnamedService(service: MarketplaceService): boolean {
  return service.variants.every(isUnnamedVariant);
}

export function serviceCategory(service: MarketplaceService): string | null {
  for (const variant of service.variants) {
    const category = variant.category?.trim();
    if (category) return category;
  }
  return null;
}

export function categoryLabel(raw: string): string {
  const words = raw.trim().toLowerCase().replace(/[_-]+/g, " ").split(/\s+/).filter(Boolean);
  if (!words.length) return "";
  return [words[0][0].toUpperCase() + words[0].slice(1), ...words.slice(1)].join(" ");
}

export function servicePopularity(service: MarketplaceService): number {
  return service.variants.reduce((best, variant) => {
    const value = variant.providerMetadata?.installCount;
    return typeof value === "number" && Number.isFinite(value) ? Math.max(best, value) : best;
  }, 0);
}

function applyIdentity(variant: MarketplaceVariant, identity: MarketplaceAppAuthState | undefined): MarketplaceVariant {
  if (!identity) return variant;
  const displayName = identity.displayName?.trim();
  const named = !!displayName && isUnnamedVariant(variant);
  const description = variant.description ?? identity.description ?? null;
  const iconDataUrl = variant.iconDataUrl ?? identity.iconUrl ?? null;
  const category = variant.category ?? identity.category ?? null;
  if (!named && description === variant.description && iconDataUrl === variant.iconDataUrl && (variant.category ?? null) === category) return variant;
  return {
    ...variant,
    name: named ? displayName : variant.name,
    nameIsFallback: named ? false : variant.nameIsFallback,
    description,
    iconDataUrl,
    category,
  };
}

export function applyAppAuthStates(
  catalog: MarketplaceCatalog,
  states: MarketplaceAppAuthState[],
): MarketplaceCatalog {
  const byConnector = new Map(states.map(state => [connectorKey(state.provider, state.connectorId), state]));
  const codexByHex = new Map<string, MarketplaceAppAuthState>();
  for (const state of states) {
    if (state.provider !== "codex") continue;
    const hex = codexStateHex(state);
    if (hex && !codexByHex.has(hex)) codexByHex.set(hex, state);
  }
  const identityFor = (variant: MarketplaceVariant): MarketplaceAppAuthState | undefined => {
    for (const id of variant.appConnectorIds) {
      const match = byConnector.get(connectorKey(variant.provider, id));
      if (match) return match;
    }
    const hex = variant.provider === "codex" ? codexPluginHex(variant) : null;
    return hex ? codexByHex.get(hex) : undefined;
  };
  return {
    providers: catalog.providers.map(provider => ({
      ...provider,
      variants: (() => {
        const variants = provider.variants.map(variant => {
          const explicit = variant.appConnectorIds.map(id => byConnector.get(connectorKey(variant.provider, id))?.authenticationState).filter((state): state is "connected" | "required" => !!state);
          const authenticationState = explicit.includes("required") ? "required" : explicit.includes("connected") ? "connected" : variant.authenticationState;
          const withAuth = authenticationState === variant.authenticationState ? variant : { ...variant, authenticationState };
          return applyIdentity(withAuth, identityFor(variant));
        });
        const represented = new Set(variants.flatMap(variant => variant.appConnectorIds));
        if (provider.provider === "claude") {
          for (const state of states.filter(item => item.provider === "claude" && item.nativeConnector && !represented.has(item.connectorId))) {
            variants.push({
              provider: "claude", pluginId: state.connectorId, name: state.displayName ?? state.connectorId,
              description: "Claude connector", marketplace: null, version: null, source: "claude.ai", repository: null, iconDataUrl: null,
              publisher: "Anthropic", capabilities: [], mcpEndpoint: null, connectorType: "connector",
              appConnectorIds: [state.connectorId], installed: true, enabled: true,
              authenticationState: state.authenticationState, sharedAuthMechanism: null, portableMcp: false,
              compatibilityNotes: ["Claude-native connector"], supportedActions: ["authenticate"], providerMetadata: {},
            });
            represented.add(state.connectorId);
          }
        }
        return variants;
      })(),
    })),
  };
}

function clean(value?: string | null): string | null {
  const normalized = value?.trim().toLowerCase().replace(/\/$/, "") ?? "";
  return normalized || null;
}

function variantKey(variant: MarketplaceVariant): string {
  return `${variant.provider}:${variant.pluginId}`.toLowerCase();
}

function verifiedPackage(variant: MarketplaceVariant): string | null {
  const metadata = variant.providerMetadata;
  if (metadata.verified !== true && metadata.packageVerified !== true) return null;
  const value = metadata.package ?? metadata.packageName;
  return typeof value === "string" ? clean(value) : null;
}

function matchingReason(
  candidate: MarketplaceVariant,
  existing: MarketplaceVariant,
  aliases: MarketplaceAliases,
): MarketplaceService["matchReason"] | null {
  const candidateAlias = aliases[variantKey(candidate)];
  const existingAlias = aliases[variantKey(existing)];
  if (candidateAlias && existingAlias && candidateAlias === existingAlias) return "alias";
  const candidateRepo = clean(candidate.repository);
  const existingRepo = clean(existing.repository);
  if (candidateRepo && candidateRepo === existingRepo) {
    const candidatePublisher = clean(candidate.publisher);
    const existingPublisher = clean(existing.publisher);
    if (candidatePublisher && candidatePublisher === existingPublisher) return "repository";
  }
  const candidateEndpoint = clean(candidate.mcpEndpoint);
  if (candidateEndpoint && candidateEndpoint === clean(existing.mcpEndpoint)) return "endpoint";
  const candidatePackage = verifiedPackage(candidate);
  if (candidatePackage && candidatePackage === verifiedPackage(existing)) return "package";
  return null;
}

export function groupMarketplaceServices(
  variants: MarketplaceVariant[],
  aliases: MarketplaceAliases = {},
): MarketplaceService[] {
  const services: MarketplaceService[] = [];
  for (const variant of variants) {
    let target: MarketplaceService | undefined;
    let reason: MarketplaceService["matchReason"] | null = null;
    for (const service of services) {
      reason = service.variants.map(existing => matchingReason(variant, existing, aliases)).find(Boolean) ?? null;
      if (reason) { target = service; break; }
    }
    if (target && reason) {
      if (target.variants.every(isUnnamedVariant) && !isUnnamedVariant(variant)) {
        target.name = variant.name;
        target.description = variant.description;
      }
      target.variants.push(variant);
      target.matchReason = reason;
      continue;
    }
    services.push({
      id: aliases[variantKey(variant)] ?? `${variant.provider}:${variant.pluginId}`,
      name: variant.name,
      description: variant.description,
      variants: [variant],
      matchReason: "single",
    });
  }
  return services.sort((a, b) => a.name.localeCompare(b.name));
}

function metadataString(metadata: Record<string, unknown>, key: string): string | null {
  const value = metadata[key];
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function safeOfficialUrl(value: string | null | undefined): URL | null {
  if (!value) return null;
  try {
    const url = new URL(value);
    const hostname = url.hostname.toLowerCase();
    const isIpv4 = /^\d{1,3}(?:\.\d{1,3}){3}$/.test(hostname);
    const isIpv6 = hostname.includes(":");
    if (url.protocol !== "https:" || url.username || url.password || (url.port && url.port !== "443")) return null;
    if (!hostname.includes(".") || hostname === "localhost" || hostname.endsWith(".localhost") || hostname.endsWith(".local") || isIpv4 || isIpv6) return null;
    return url;
  } catch {
    return null;
  }
}

function githubAvatar(url: URL): string | null {
  if (url.hostname.toLowerCase() !== "github.com") return null;
  const owner = url.pathname.split("/").filter(Boolean)[0];
  return owner ? `https://github.com/${encodeURIComponent(owner)}.png?size=128` : null;
}

export function verifiedBrandLogoUrl(service: MarketplaceService): string | null {
  const websiteCandidates = service.variants.flatMap(variant => {
    const metadata = variant.providerMetadata;
    const interfaceMetadata = metadata.interface && typeof metadata.interface === "object" && !Array.isArray(metadata.interface)
      ? metadata.interface as Record<string, unknown>
      : {};
    return [
      metadataString(interfaceMetadata, "websiteURL"),
      metadataString(metadata, "homepage"),
      metadataString(metadata, "websiteURL"),
      metadataString(metadata, "website"),
    ];
  });
  for (const candidate of websiteCandidates) {
    const url = safeOfficialUrl(candidate);
    if (!url) continue;
    return githubAvatar(url) ?? `${url.origin}/favicon.ico`;
  }

  const repositoryCandidates = service.variants.flatMap(variant => [variant.repository, variant.source]);
  for (const candidate of repositoryCandidates) {
    const url = safeOfficialUrl(candidate);
    if (!url) continue;
    return githubAvatar(url) ?? `${url.origin}/favicon.ico`;
  }
  return null;
}

const sharedMechanisms = new Set(["mcp_server", "environment_token", "gh_cli", "credential_helper"]);

export function compatibilityLabels(service: MarketplaceService): string[] {
  const labels: string[] = [];
  const providers = new Set(service.variants.map(variant => variant.provider));
  if (providers.size > 1) labels.push("Native variants");
  if (service.variants.some(variant => variant.portableMcp) && providers.size === 1) labels.push("Convertible MCP variant");
  if (service.variants.some(variant => !variant.portableMcp && variant.connectorType?.toLowerCase().includes("connector"))) labels.push("Provider only");
  const mechanisms = service.variants.map(variant => clean(variant.sharedAuthMechanism)).filter((value): value is string => !!value);
  if (providers.size > 1 && mechanisms.length === service.variants.length && new Set(mechanisms).size === 1 && sharedMechanisms.has(mechanisms[0])) {
    labels.push("Shared auth compatible");
  } else if (service.variants.some(variant => !!variant.mcpEndpoint)) {
    labels.push("Separate login required");
  }
  return labels.length ? labels : ["Native variant"];
}

export type MarketplaceActionInvoker = (
  provider: MarketplaceProvider,
  pluginId: string,
  marketplace: string | null,
  action: "install",
) => Promise<MarketplaceActionResult>;

export async function installVariants(
  variants: MarketplaceVariant[],
  invoke: MarketplaceActionInvoker,
): Promise<MarketplaceActionResult[]> {
  return Promise.all(variants.map(async variant => {
    try {
      return await invoke(variant.provider, variant.pluginId, variant.marketplace, "install");
    } catch (error) {
      return {
        provider: variant.provider,
        pluginId: variant.pluginId,
        action: "install" as const,
        success: false,
        message: "install failed",
        error: error instanceof Error ? error.message : String(error),
      };
    }
  }));
}

export function failedVariants(variants: MarketplaceVariant[], results: MarketplaceActionResult[]): MarketplaceVariant[] {
  const failed = new Set(results.filter(result => !result.success).map(result => `${result.provider}:${result.pluginId}`));
  return variants.filter(variant => failed.has(`${variant.provider}:${variant.pluginId}`));
}
