// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import type { ManagedAgentStatus } from "../protocol/generated/protocol";
import type { MarketplaceAppAuthState, MarketplaceCatalog, MarketplaceVariant } from "../types";
import { MarketplaceScreen } from "./MarketplaceScreen";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function agent(overrides: Partial<ManagedAgentStatus> = {}): ManagedAgentStatus {
  return {
    agentId: "codex",
    label: "Codex",
    state: "ready",
    backing: "managed",
    removable: true,
    executable: "/managed-runtimes/agents/codex/installations/abc/payload/bin/codex",
    version: "0.147.0",
    consecutiveFailures: 0,
    ...overrides,
  } as ManagedAgentStatus;
}

function variant(pluginId: string, overrides: Partial<MarketplaceVariant> = {}): MarketplaceVariant {
  return {
    provider: "claude", pluginId, name: pluginId, description: null, marketplace: "official", version: null, iconDataUrl: null,
    source: null, repository: null, publisher: null, capabilities: [], mcpEndpoint: null, connectorType: null,
    appConnectorIds: [], installed: false, enabled: false, authenticationState: "unknown", sharedAuthMechanism: null,
    portableMcp: false, compatibilityNotes: [], supportedActions: ["install"], providerMetadata: {}, ...overrides,
  };
}

function catalog(variants: MarketplaceVariant[]): MarketplaceCatalog {
  return {
    providers: (["codex", "claude"] as const).map(provider => ({
      provider, available: true, error: null,
      variants: variants.filter(item => item.provider === provider),
    })),
  };
}

async function render() {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const flush = () => act(async () => { await Promise.resolve(); });
  const view = {
    host,
    flush,
    text: () => host.textContent ?? "",
    articles: () => [...host.querySelectorAll("article")],
    button: (label: string) => [...host.querySelectorAll("button")].find(item => item.textContent?.trim() === label) ?? null,
    viewButton: (label: string) => [...host.querySelectorAll('[aria-label="Plugin views"] button')].find(item => item.textContent?.trim() === label) ?? null,
    click: async (item: Element | null) => {
      expect(item, "control must exist to be clicked").not.toBeNull();
      await act(async () => { (item as HTMLButtonElement).click(); await Promise.resolve(); });
    },
    type: async (value: string) => {
      const input = host.querySelector<HTMLInputElement>('input[aria-label="Search plugins"]');
      expect(input, "search input must exist").not.toBeNull();
      const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value")!.set!;
      await act(async () => {
        setter.call(input, value);
        input!.dispatchEvent(new Event("input", { bubbles: true }));
        await Promise.resolve();
      });
    },
    unmount: () => act(async () => { root.unmount(); host.remove(); }),
  };
  await act(async () => { root.render(<MarketplaceScreen/>); });
  return view;
}

async function openPlugins(catalogValue: MarketplaceCatalog, states: MarketplaceAppAuthState[] = []) {
  vi.spyOn(bridgeApi, "listManagedAgents").mockResolvedValue({ agents: [agent()] } as Awaited<ReturnType<typeof bridgeApi.listManagedAgents>>);
  vi.spyOn(bridgeApi, "marketplaceCatalog").mockResolvedValue(catalogValue);
  vi.spyOn(bridgeApi, "marketplaceAppAuthStates").mockResolvedValue(states);
  const view = await render();
  await view.click(view.button("plugins"));
  await view.flush();
  return view;
}

const popularFixtures = Array.from({ length: 12 }, (_, index) => variant(`popular-${index}`, {
  name: `Popular ${String.fromCharCode(65 + index)}`,
  providerMetadata: { installCount: 1000 - index },
}));

afterEach(() => { vi.restoreAllMocks(); document.body.innerHTML = ""; });

describe("Plugins catalog", () => {
  it("shows a list skeleton while the catalog is in flight", async () => {
    vi.spyOn(bridgeApi, "listManagedAgents").mockResolvedValue({ agents: [agent()] } as Awaited<ReturnType<typeof bridgeApi.listManagedAgents>>);
    vi.spyOn(bridgeApi, "marketplaceCatalog").mockReturnValue(new Promise<MarketplaceCatalog>(() => {}));
    vi.spyOn(bridgeApi, "marketplaceAppAuthStates").mockResolvedValue([]);

    const view = await render();
    await view.click(view.button("plugins"));

    expect(view.host.querySelector('[data-testid="plugin-catalog-skeleton"]')).not.toBeNull();
    await view.unmount();
  });

  it("opens on installed plus a short popular set, never the long tail", async () => {
    const view = await openPlugins(catalog([
      variant("vercel", { name: "Vercel", installed: true }),
      ...popularFixtures,
      variant("obscure", { name: "Obscure Tool", providerMetadata: { installCount: 5 } }),
      variant("zero", { name: "Zero Count Tool" }),
    ]));

    expect(view.text()).toContain("Installed");
    expect(view.text()).toContain("Vercel");
    expect(view.text()).toContain("Popular A");
    expect(view.text()).not.toContain("Obscure Tool");
    expect(view.text()).not.toContain("Zero Count Tool");
    // installed + FEATURED_LIMIT, not the whole catalog
    expect(view.articles().length).toBe(13);
    await view.unmount();
  });

  it("never renders a fallback app id and hides it until an identity arrives", async () => {
    const fallback = variant("app-6a057d268ebc81919918d37eec718425@openai-curated-remote", {
      provider: "codex", name: "app-6a057d268ebc81919918d37eec718425@openai-curated-remote", nameIsFallback: true,
    });
    const view = await openPlugins(catalog([fallback]));

    await view.type("remote");
    expect(view.text()).not.toContain("app-6a057d268ebc81919918d37eec718425");
    expect(view.articles()).toHaveLength(0);
    expect(view.text()).toContain("No matching plugins");
    await view.unmount();
  });

  it("resolves an app-<hex> row to its app directory name and category", async () => {
    const fallback = variant("app-6a057d268ebc81919918d37eec718425@openai-curated-remote", {
      provider: "codex", name: "app-6a057d268ebc81919918d37eec718425@openai-curated-remote", nameIsFallback: true,
    });
    const view = await openPlugins(catalog([fallback]), [{
      provider: "codex", connectorId: "asdk_app_6a057d268ebc81919918d37eec718425",
      displayName: "Remote Desktop Commander", description: "Build and automate, anywhere",
      iconUrl: null, category: "DEVELOPER_TOOLS", nativeConnector: false, authenticationState: "required",
    }]);

    await view.type("remote");
    expect(view.text()).toContain("Remote Desktop Commander");
    expect(view.text()).not.toContain("app-6a057d268ebc81919918d37eec718425");
    expect(view.articles()).toHaveLength(1);
    await view.unmount();
  });

  it("search reaches the long tail by name, description, and capability", async () => {
    const view = await openPlugins(catalog([
      ...popularFixtures,
      variant("obscure", { name: "Zebra Analyzer", description: "Inspects release pipelines", capabilities: ["pipeline-linting"] }),
    ]));

    expect(view.text()).not.toContain("Zebra Analyzer");
    await view.type("zebra");
    expect(view.text()).toContain("Zebra Analyzer");
    await view.type("pipelines");
    expect(view.text()).toContain("Zebra Analyzer");
    await view.type("pipeline-linting");
    expect(view.text()).toContain("Zebra Analyzer");
    await view.unmount();
  });

  it("browses all in pages instead of mounting the catalog", async () => {
    const variants = Array.from({ length: 30 }, (_, index) => variant(`plugin-${index}`, { name: `Plugin ${String(index + 1).padStart(2, "0")}` }));
    const view = await openPlugins(catalog(variants));

    await view.click(view.button("Browse all 30 plugins"));
    expect(view.articles()).toHaveLength(24);

    await view.click(view.host.querySelector('[data-testid="plugin-show-more"]'));
    expect(view.articles()).toHaveLength(30);
    expect(view.host.querySelector('[data-testid="plugin-show-more"]')).toBeNull();
    await view.unmount();
  });

  it("divides the catalog into provider categories", async () => {
    const view = await openPlugins(catalog([
      variant("github", { provider: "codex", name: "GitHub", category: "DEVELOPER_TOOLS" }),
      variant("gmail", { provider: "codex", name: "Gmail", category: "PRODUCTIVITY" }),
      variant("misc", { provider: "codex", name: "Misc Tool" }),
    ]));

    await view.click(view.viewButton("All"));
    expect(view.host.querySelector('[data-category="DEVELOPER_TOOLS"]')?.textContent).toContain("Developer tools");
    expect(view.host.querySelector('[data-category="PRODUCTIVITY"]')?.textContent).toContain("Productivity");
    expect(view.host.querySelector('[data-category="other"]')?.textContent).toContain("Other");

    await view.click(view.host.querySelector('[data-category="DEVELOPER_TOOLS"]'));
    expect(view.articles()).toHaveLength(1);
    expect(view.text()).toContain("GitHub");
    expect(view.text()).not.toContain("Gmail");
    expect(view.text()).not.toContain("Misc Tool");
    await view.unmount();
  });
});
