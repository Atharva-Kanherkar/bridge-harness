import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
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
});
