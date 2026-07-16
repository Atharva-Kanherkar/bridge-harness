import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { ModelSetupWizard } from "./ModelSetupWizard";
import { RouterSettingsDialog } from "./RouterSettingsDialog";
import type { AdapterDescriptor } from "../types";

const adapters: AdapterDescriptor[] = [{
  id: "catalog", label: "Catalog", available: true, version: "1", capabilities: [], unavailableReason: null, defaultModel: "balanced",
  models: [
    { id: "quick", label: "Quick", tier: "fast", defaultForTier: true },
    { id: "balanced", label: "Balanced", tier: "standard", defaultForTier: true },
    { id: "deep", label: "Deep", tier: "strong", defaultForTier: true },
  ],
}];

describe("adaptive setup surfaces", () => {
  it("puts one-click recommended setup before advanced disclosure", () => {
    const html = renderToStaticMarkup(<ModelSetupWizard adapters={adapters} onComplete={() => undefined} onError={() => undefined} />);
    expect(html).toContain("Use recommended defaults");
    expect(html).toContain("Customize role profiles");
    expect(html).not.toContain("Advanced role profiles");
  });

  it("explains the single local runner and truthful provider limitations", () => {
    const html = renderToStaticMarkup(<RouterSettingsDialog open workspaceId="workspace" adapters={adapters} onClose={() => undefined} onError={() => undefined} />);
    expect(html).toContain("Run learning now");
    expect(html).toContain("Bridge cannot create or enumerate schedules");
    expect(html).toContain("Cloud Routines are experimental");
    expect(html).toContain("Role model profiles");
  });
});
