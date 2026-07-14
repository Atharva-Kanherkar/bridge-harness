import { describe, expect, it, vi } from "vitest";
import { applyAppAuthStates, authenticationLabel, compatibilityLabels, failedVariants, groupMarketplaceServices, installVariants, MARKETPLACE_ALIASES, verifiedBrandLogoUrl } from "./marketplace";
import type { MarketplaceCatalog, MarketplaceVariant } from "./types";

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
