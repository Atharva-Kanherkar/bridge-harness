// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import type { OpenCodeCatalog } from "../types";
import { OpenCodeHarnessSettings } from "./OpenCodeHarnessSettings";

const catalog: OpenCodeCatalog = {
  executablePath: "/managed/opencode",
  version: "1.18.3",
  providers: [
    {
      id: "opencode-go", name: "OpenCode Go", connected: true, source: "api",
      environmentVariables: [], defaultModel: "opencode-go/kimi-k2.5", authMethods: [{ kind: "api", label: "API key" }],
      models: [{ id: "opencode-go/kimi-k2.5", providerId: "opencode-go", modelId: "kimi-k2.5", label: "Kimi K2.5", reasoning: true, toolCall: true, attachment: true, contextWindow: 262144, outputLimit: 65536, inputCost: null, outputCost: null }],
    },
    {
      id: "anthropic", name: "Anthropic", connected: false, source: "env",
      environmentVariables: ["ANTHROPIC_API_KEY"], defaultModel: null, authMethods: [{ kind: "api", label: "API key" }], models: [],
    },
  ],
};

describe("OpenCodeHarnessSettings", () => {
  it("shows provider status, redacted key entry, executable override, and qualified model controls", () => {
    const html = renderToStaticMarkup(<OpenCodeHarnessSettings
      value={{ executablePath: "/custom/opencode", visibleModels: ["opencode-go/kimi-k2.5"] }}
      catalog={catalog}
      disabled={false}
      onChange={() => undefined}
      onCatalog={() => undefined}
      onError={() => undefined}
    />);
    expect(html).toContain("OpenCode Go");
    expect(html).toContain("Connected");
    expect(html).toContain("Anthropic");
    expect(html).toContain('type="password"');
    expect(html).toContain("never saved by Bridge");
    expect(html).toContain("/custom/opencode");
    expect(html).toContain("opencode-go/kimi-k2.5");
    expect(html).toContain("Disconnect");
  });

  it("submits an API key once and clears it from component state", async () => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    const updated = { ...catalog, providers: catalog.providers.map(provider => provider.id === "anthropic" ? { ...provider, connected: true } : provider) };
    const setKey = vi.spyOn(bridgeApi, "setOpenCodeProviderApiKey").mockResolvedValue(updated);
    const container = document.createElement("div");
    const root = createRoot(container);
    await act(async () => root.render(<OpenCodeHarnessSettings value={{}} catalog={catalog} disabled={false} onChange={() => undefined} onCatalog={() => undefined} onError={error => { throw new Error(error); }}/>));

    const input = container.querySelector<HTMLInputElement>('#opencode-key-anthropic')!;
    await act(async () => {
      const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      setValue?.call(input, "disposable-provider-key");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    const connect = [...container.querySelectorAll("button")].find(button => button.textContent?.includes("Connect"))!;
    await act(async () => { connect.click(); await Promise.resolve(); });

    expect(setKey).toHaveBeenCalledOnce();
    expect(setKey).toHaveBeenCalledWith("anthropic", "disposable-provider-key");
    expect(input.value).toBe("");
    expect(container.innerHTML).not.toContain("disposable-provider-key");
    await act(async () => root.unmount());
    vi.restoreAllMocks();
  });
});
