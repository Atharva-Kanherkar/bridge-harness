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

export function applyAppAuthStates(
  catalog: MarketplaceCatalog,
  states: MarketplaceAppAuthState[],
): MarketplaceCatalog {
  const byConnector = new Map(states.map(state => [`${state.provider}:${state.connectorId}`, state.authenticationState]));
  return {
    providers: catalog.providers.map(provider => ({
      ...provider,
      variants: (() => {
        const variants = provider.variants.map(variant => {
          const explicit = variant.appConnectorIds.map(id => byConnector.get(`${variant.provider}:${id}`)).filter((state): state is "connected" | "required" => !!state);
          const authenticationState = explicit.includes("required") ? "required" : explicit.includes("connected") ? "connected" : variant.authenticationState;
          return authenticationState === variant.authenticationState ? variant : { ...variant, authenticationState };
        });
        const represented = new Set(variants.flatMap(variant => variant.appConnectorIds));
        if (provider.provider === "claude") {
          for (const state of states.filter(item => item.provider === "claude" && item.nativeConnector && !represented.has(item.connectorId))) {
            variants.push({
              provider: "claude", pluginId: state.connectorId, name: state.displayName ?? state.connectorId,
              description: "Claude connector", marketplace: null, version: null, source: "claude.ai", repository: null,
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
