import type { MarketplaceActionResult, MarketplaceProvider, MarketplaceVariant } from "./types";

export interface MarketplaceService {
  id: string;
  name: string;
  description: string | null;
  variants: MarketplaceVariant[];
  matchReason: "alias" | "repository" | "endpoint" | "package" | "single";
}

export type MarketplaceAliases = Record<string, string>;

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
