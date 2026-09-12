import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const APP = readFileSync(join(__dirname, "App.tsx"), "utf8");
const SERVER_STATE = readFileSync(join(__dirname, "serverState.ts"), "utf8");

describe("server state wiring", () => {
  it("loads health and model setup through shared queries", () => {
    expect(APP).toContain("useBridgeServerState()");
    expect(SERVER_STATE).toContain("queryKey: queryKeys.health");
    expect(SERVER_STATE).toContain("queryFn: () => bridgeApi.health()");
    expect(SERVER_STATE).toContain("queryKey: queryKeys.modelSetup");
    expect(SERVER_STATE).toContain("queryFn: () => bridgeApi.modelSetup()");
    expect(APP).not.toContain("useState<Health>");
    expect(APP).not.toContain("useState<ModelSetupState>");
  });

  it("invalidates health from backend availability events", () => {
    expect(SERVER_STATE).toContain("queryClient.invalidateQueries({ queryKey: queryKeys.health })");
    expect(APP).toContain("bridgeApi.onAdaptersChanged(reloadHealth)");
    expect(APP).toContain("bridgeApi.onTerminalExited");
  });

  it("writes successful model setup changes to the shared cache", () => {
    expect(SERVER_STATE).toContain("queryClient.setQueryData(queryKeys.modelSetup, setup)");
    expect(APP).toContain("onComplete={finishAgentOnboarding}");
    expect(APP).toContain("if (setup) acceptModelSetup(setup)");
    expect(APP).toContain("onModelSetupChange={acceptModelSetup}");
  });
});
