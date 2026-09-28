import { describe, expect, it, vi } from "vitest";
import { applyAppAuthStates, authenticationLabel, categoryLabel, compatibilityLabels, failedVariants, groupMarketplaceServices, installVariants, isUnnamedService, isUnnamedVariant, MARKETPLACE_ALIASES, serviceCategory, servicePopularity, verifiedBrandLogoUrl } from "./marketplace";
import type { MarketplaceAppAuthState, MarketplaceCatalog, MarketplaceVariant } from "./types";

const codexAppState = (overrides: Partial<MarketplaceAppAuthState> = {}): MarketplaceAppAuthState => ({
  provider: "codex", connectorId: "asdk_app_6a057d268ebc81919918d37eec718425", displayName: "Remote Desktop Commander",
  description: "Build and automate, anywhere", iconUrl: "https://images.example.test/remote.png", category: "DEVELOPER_TOOLS",
  nativeConnector: false, authenticationState: "required", ...overrides,
});

function variant(provider: "codex" | "claude", pluginId: string, overrides: Partial<MarketplaceVariant> = {}): MarketplaceVariant {
  return {
    provider, pluginId, name: "Vercel", description: null, marketplace: "official", version: null, iconDataUrl: null,
    source: null, repository: null, publisher: null, capabilities: [], mcpEndpoint: null,
    connectorType: null, appConnectorIds: [], installed: false, enabled: false, authenticationState: "unknown",
    sharedAuthMechanism: null, portableMcp: false, compatibilityNotes: [], supportedActions: ["install", "enable", "disable", "update", "uninstall", "authenticate"], providerMetadata: {}, ...overrides,
  };
}

describe("marketplace service grouping", () => {
  it("never merges variants by display name alone", () => {
    expect(groupMarketplaceServices([variant("codex", "one"), variant("claude", "two")])).toHaveLength(2);
  });

  it("groups matching repository and publisher variants", () => {
    const services = groupMarketplaceServices([
      variant("codex", "one", { repository: "https://github.com/vercel/mcp", publisher: "Vercel" }),
      variant("claude", "two", { repository: "https://github.com/vercel/mcp/", publisher: "vercel" }),
    ]);
    expect(services).toHaveLength(1);
    expect(services[0].matchReason).toBe("repository");
  });

  it("uses explicit aliases before metadata matches", () => {
    const services = groupMarketplaceServices(
      [variant("codex", "one"), variant("claude", "two")],
      { "codex:one": "vercel", "claude:two": "vercel" },
    );
    expect(services).toHaveLength(1);
    expect(services[0].matchReason).toBe("alias");
  });

  it("ships explicit aliases for verified cross-provider services", () => {
    const services = groupMarketplaceServices([
      variant("codex", "vercel@openai-curated"),
      variant("claude", "vercel@claude-plugins-official"),
    ], MARKETPLACE_ALIASES);
    expect(services).toHaveLength(1);
    expect(services[0].id).toBe("vercel");
  });
});

describe("marketplace compatibility", () => {
  it("defaults remote MCP variants to separate login", () => {
    const service = groupMarketplaceServices([
      variant("codex", "one", { mcpEndpoint: "https://mcp.example.test", portableMcp: true }),
      variant("claude", "two", { mcpEndpoint: "https://mcp.example.test", portableMcp: true }),
    ])[0];
    expect(compatibilityLabels(service)).toContain("Separate login required");
  });

  it("shows shared auth only for one explicit supported mechanism", () => {
    const service = groupMarketplaceServices([
      variant("codex", "one", { mcpEndpoint: "https://mcp.example.test", sharedAuthMechanism: "gh_cli" }),
      variant("claude", "two", { mcpEndpoint: "https://mcp.example.test", sharedAuthMechanism: "gh_cli" }),
    ])[0];
    expect(compatibilityLabels(service)).toContain("Shared auth compatible");
  });
});

describe("verified brand logos", () => {
  it("prefers an official website favicon over repository metadata", () => {
    const service = groupMarketplaceServices([variant("claude", "vercel", {
      repository: "https://github.com/vercel/vercel-plugin",
      providerMetadata: { homepage: "https://vercel.com/products" },
    })])[0];
    expect(verifiedBrandLogoUrl(service)).toBe("https://vercel.com/favicon.ico");
  });

  it("derives a GitHub organization avatar from the official repository", () => {
    const service = groupMarketplaceServices([variant("claude", "adobe", { repository: "https://github.com/adobe/skills" })])[0];
    expect(verifiedBrandLogoUrl(service)).toBe("https://github.com/adobe.png?size=128");
  });

  it("rejects unsafe logo origins", () => {
    for (const repository of ["http://example.com/plugin", "https://user:pass@example.com/plugin", "https://localhost/plugin", "https://127.0.0.1/plugin", "https://example.com:8443/plugin"]) {
      const service = groupMarketplaceServices([variant("claude", "unsafe", { repository })])[0];
      expect(verifiedBrandLogoUrl(service)).toBeNull();
    }
  });
});

describe("authentication presentation", () => {
  it("shows only explicit provider-reported authentication states", () => {
    expect(authenticationLabel("connected")).toBe("Connected");
    expect(authenticationLabel("required")).toBe("Needs login");
    expect(authenticationLabel("unknown")).toBeNull();
    expect(authenticationLabel("unrecognized-provider-state")).toBeNull();
  });

  it("merges explicit connector accessibility without guessing missing states", () => {
    const catalog: MarketplaceCatalog = { providers: [{
      provider: "codex", available: true, error: null, variants: [
        variant("codex", "vercel", { appConnectorIds: ["connector_vercel"], authenticationState: "unknown" }),
        variant("codex", "missing", { appConnectorIds: ["connector_missing"], authenticationState: "unknown" }),
      ],
    }] };

    const merged = applyAppAuthStates(catalog, [{ provider: "codex", connectorId: "connector_vercel", displayName: null, nativeConnector: false, authenticationState: "connected" }]);

    expect(merged.providers[0].variants[0].authenticationState).toBe("connected");
    expect(merged.providers[0].variants[1].authenticationState).toBe("unknown");
  });

  it("adds native Claude connectors and keeps required precedence for plugin bundles", () => {
    const catalog: MarketplaceCatalog = { providers: [{
      provider: "claude", available: true, error: null, variants: [
        variant("claude", "vercel", { appConnectorIds: ["plugin:vercel:docs", "plugin:vercel:vercel"], authenticationState: "unknown" }),
      ],
    }] };

    const merged = applyAppAuthStates(catalog, [
      { provider: "claude", connectorId: "plugin:vercel:docs", displayName: null, nativeConnector: false, authenticationState: "connected" },
      { provider: "claude", connectorId: "plugin:vercel:vercel", displayName: null, nativeConnector: false, authenticationState: "required" },
      { provider: "claude", connectorId: "claude.ai Notion", displayName: "Notion", nativeConnector: true, authenticationState: "connected" },
    ]);

    expect(merged.providers[0].variants[0].authenticationState).toBe("required");
    expect(merged.providers[0].variants[1]).toMatchObject({
      pluginId: "claude.ai Notion", name: "Notion", connectorType: "connector", authenticationState: "connected",
    });
  });
});

describe("app identity resolution", () => {
  const fallbackCatalog = (overrides: Partial<MarketplaceVariant> = {}): MarketplaceCatalog => ({
    providers: [{
      provider: "codex", available: true, error: null, variants: [
        variant("codex", "app-6A057D268EBC81919918D37EEC718425@openai-curated-remote", {
          name: "app-6A057D268EBC81919918D37EEC718425@openai-curated-remote", nameIsFallback: true, ...overrides,
        }),
      ],
    }],
  });

  it("renames a fallback app id from its app directory identity", () => {
    const [resolved] = applyAppAuthStates(fallbackCatalog(), [codexAppState()]).providers[0].variants;
    expect(resolved.name).toBe("Remote Desktop Commander");
    expect(resolved.nameIsFallback).toBe(false);
    expect(resolved.description).toBe("Build and automate, anywhere");
    expect(resolved.iconDataUrl).toBe("https://images.example.test/remote.png");
    expect(resolved.category).toBe("DEVELOPER_TOOLS");
    expect(isUnnamedVariant(resolved)).toBe(false);
  });

  it("keeps provider-supplied detail and never renames a human name", () => {
    const catalog: MarketplaceCatalog = { providers: [{
      provider: "codex", available: true, error: null, variants: [
        variant("codex", "gmail@openai-curated-remote", { name: "Gmail", description: "Bridge description", appConnectorIds: ["connector_gmail"] }),
      ],
    }] };
    const [resolved] = applyAppAuthStates(catalog, [
      codexAppState({ connectorId: "connector_gmail", displayName: "Gmail (Work)", description: "Directory description", iconUrl: null, category: "PRODUCTIVITY" }),
    ]).providers[0].variants;
    expect(resolved.name).toBe("Gmail");
    expect(resolved.description).toBe("Bridge description");
    expect(resolved.category).toBe("PRODUCTIVITY");
  });

  it("treats an id-shaped label as unnamed even without the backend flag", () => {
    const unmarked = variant("codex", "app-6a057d268ebc81919918d37eec718425@openai-curated-remote", {
      name: "app-6a057d268ebc81919918d37eec718425@openai-curated-remote",
    });
    expect(isUnnamedVariant(unmarked)).toBe(true);
    const slug = variant("codex", "browser@openai-bundled", { name: "browser" });
    expect(isUnnamedVariant(slug)).toBe(false);
  });

  it("leaves unmatched fallbacks unnamed so the UI can hide them", () => {
    const [unmatched] = applyAppAuthStates(fallbackCatalog(), []).providers[0].variants;
    expect(isUnnamedVariant(unmatched)).toBe(true);
    expect(isUnnamedService(groupMarketplaceServices([unmatched])[0])).toBe(true);
    const [other] = applyAppAuthStates(fallbackCatalog(), [codexAppState({ connectorId: "asdk_app_deadbeefdeadbeefdeadbeefdeadbeef" })]).providers[0].variants;
    expect(other.nameIsFallback).toBe(true);
  });

  it("upgrades a grouped service name once a named variant arrives", () => {
    const services = groupMarketplaceServices([
      variant("codex", "one", { name: "one", nameIsFallback: true }),
      variant("claude", "two", { name: "Two" }),
    ], { "codex:one": "shared", "claude:two": "shared" });
    expect(services).toHaveLength(1);
    expect(services[0].name).toBe("Two");
    expect(isUnnamedService(services[0])).toBe(false);
  });
});

describe("catalog categories and popularity", () => {
  it("formats provider categories for people", () => {
    expect(categoryLabel("DEVELOPER_TOOLS")).toBe("Developer tools");
    expect(categoryLabel("productivity")).toBe("Productivity");
    expect(categoryLabel("Collaboration")).toBe("Collaboration");
    expect(categoryLabel("   ")).toBe("");
  });

  it("reads category and install count from variants", () => {
    const service = groupMarketplaceServices([
      variant("claude", "one", { category: null, repository: "https://github.com/example/tool", publisher: "example", providerMetadata: { installCount: 120 } }),
      variant("claude", "two", { category: "BUSINESS", repository: "https://github.com/example/tool", publisher: "example", providerMetadata: { installCount: 40 } }),
    ])[0];
    expect(serviceCategory(service)).toBe("BUSINESS");
    expect(servicePopularity(service)).toBe(120);
  });

  it("reports no popularity when the provider never counted installs", () => {
    const service = groupMarketplaceServices([variant("codex", "one", { providerMetadata: {} })])[0];
    expect(servicePopularity(service)).toBe(0);
    expect(serviceCategory(service)).toBeNull();
  });
});

describe("dual-provider installation", () => {
  it("preserves partial success and selects only failures for retry", async () => {
    const variants = [variant("codex", "one"), variant("claude", "two")];
    const invoke = vi.fn(async (provider: "codex" | "claude", pluginId: string) => ({
      provider, pluginId, action: "install" as const, success: provider === "codex",
      message: provider === "codex" ? "installed" : "failed", error: provider === "codex" ? null : "login required",
    }));
    const results = await installVariants(variants, invoke);
    expect(results.map(result => result.success)).toEqual([true, false]);
    expect(failedVariants(variants, results).map(item => item.provider)).toEqual(["claude"]);
  });
});
