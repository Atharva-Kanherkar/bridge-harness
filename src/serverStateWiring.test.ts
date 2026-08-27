import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const APP = readFileSync(join(__dirname, "App.tsx"), "utf8");

describe("server state wiring", () => {
  it("loads health and model setup through shared queries", () => {
    expect(APP).toContain("queryKey: queryKeys.health");
    expect(APP).toContain("queryFn: () => bridgeApi.health()");
    expect(APP).toContain("queryKey: queryKeys.modelSetup");
    expect(APP).toContain("queryFn: () => bridgeApi.modelSetup()");
    expect(APP).not.toContain("useState<Health>");
    expect(APP).not.toContain("useState<ModelSetupState>");
  });

  it("invalidates health from backend availability events", () => {
    expect(APP).toContain("queryClient.invalidateQueries({ queryKey: queryKeys.health })");
    expect(APP).toContain("bridgeApi.onAdaptersChanged(reloadHealth)");
    expect(APP).toContain("bridgeApi.onTerminalExited");
  });

  it("writes successful model setup changes to the shared cache", () => {
    expect(APP).toContain("queryClient.setQueryData(queryKeys.modelSetup, setup)");
    expect(APP).toContain("onComplete={acceptModelSetup}");
    expect(APP).toContain("onModelSetupChange={acceptModelSetup}");
  });
});
