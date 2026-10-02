// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import type { AdapterDescriptor } from "../types";
import type { ManagedAgentStatus } from "../protocol/generated/protocol";
import { ModelSetupWizard } from "./ModelSetupWizard";

const codex: AdapterDescriptor = {
  id: "codex", label: "Codex", available: true, authState: "signed_in", capabilities: [],
  models: (["fast", "standard", "strong"] as const).map(tier => ({ id: tier, label: tier, tier, defaultForTier: true })),
};
const installed: ManagedAgentStatus = { agentId: "codex", label: "Codex", backing: "external", state: "external", pinnedVersion: "1", removable: false, consecutiveFailures: 0, updateAvailable: false };
const missing: ManagedAgentStatus = { ...installed, agentId: "opencode", label: "OpenCode", backing: "none", state: "not_installed" };
const flush = () => new Promise(resolve => setTimeout(resolve, 0));

describe("first-run choose and connect", () => {
  let host: HTMLDivElement;
  let root: Root;
  let agents: ManagedAgentStatus[];
  let adapters: AdapterDescriptor[];
  const complete = vi.fn();
  const errors = vi.fn();
  const button = (label: string) => [...host.querySelectorAll("button")].find(item => item.textContent?.trim() === label)!;
  const click = async (label: string) => { await act(async () => { button(label).click(); await flush(); }); };
  const select = async (label: string) => { await act(async () => { [...host.querySelectorAll("label")].find(item => item.textContent?.includes(label))!.querySelector<HTMLInputElement>("input")!.click(); await flush(); }); };
  const render = async () => { await act(async () => { root.render(<ModelSetupWizard adapters={adapters} onComplete={complete} onError={errors} />); await flush(); }); };

  beforeEach(async () => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    agents = [installed, missing]; adapters = [codex]; complete.mockReset(); errors.mockReset();
    const health = await bridgeApi.health();
    vi.spyOn(bridgeApi, "listManagedAgents").mockImplementation(async () => ({ agents }));
    vi.spyOn(bridgeApi, "refreshModelCatalogs").mockImplementation(async () => ({ ...health, adapters }));
    vi.spyOn(bridgeApi, "health").mockImplementation(async () => ({ ...health, adapters }));
    vi.spyOn(bridgeApi, "saveModelProfiles").mockImplementation(async profiles => ({ complete: true, activeVersion: 1, profiles: profiles.map(profile => ({ ...profile, version: 1, schemaVersion: 1, profileId: profile.purpose, canonicalRole: "implementation" as const, createdAt: "now" })) }));
    vi.spyOn(bridgeApi, "saveHarnessConfig");
    host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  });
  afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); });

  it("explains ownership and requires a choice with no skip path", async () => {
    await render();
    expect(host.textContent).toContain("You manage its updates");
    expect(host.textContent).toContain("a separate copy Bridge can update and remove");
    expect(host.textContent).toContain("Your own install");
    expect(host.textContent).toContain("Not installed");
    expect(button("Continue").disabled).toBe(true);
    expect(host.textContent).not.toContain("Skip");
    await select("Codex"); await click("Continue");
    expect(button("Start using Bridge").disabled).toBe(false);
    expect(host.textContent).not.toContain("OpenCode");
  });
  it("creates recommended profiles only for the selected agent and persists enabled choices", async () => {
    adapters = [codex, { ...codex, id: "claude", label: "Claude Code" }];
    await render(); await select("Codex"); await click("Continue"); await click("Start using Bridge");
    expect(complete).toHaveBeenCalledOnce();
    const profiles = vi.mocked(bridgeApi.saveModelProfiles).mock.calls[0][0];
    expect(profiles).toHaveLength(9);
    expect(profiles.every(profile => profile.provider === "codex")).toBe(true);
    expect(vi.mocked(bridgeApi.saveHarnessConfig).mock.calls.some(([config]) => config.id === "claude" && !config.enabled)).toBe(true);
  });
  it("can finish with one real model without inventing missing model tiers", async () => {
    adapters = [{ ...codex, models: [{ id: "only", label: "Only model", tier: "standard", defaultForTier: true, supportedEffortLevels: ["low"] }] }];
    await render(); await select("Codex"); await click("Continue"); await click("Start using Bridge");
    expect(complete).toHaveBeenCalledOnce();
    const profiles = vi.mocked(bridgeApi.saveModelProfiles).mock.calls[0][0];
    expect(profiles.every(profile => profile.model === "only" && profile.effort === "low")).toBe(true);
    expect(profiles.find(profile => profile.purpose === "planner")?.selectionMode).toBe("pinned");
  });
  it("does not mark setup complete when no usable model is reported", async () => {
    adapters = [{ ...codex, models: [] }];
    await render(); await select("Codex"); await click("Continue"); await click("Start using Bridge");
    expect(complete).not.toHaveBeenCalled();
    expect(bridgeApi.saveModelProfiles).not.toHaveBeenCalled();
    expect(host.querySelector('[role="alert"]')).toBeTruthy();
  });
  it.each(["signed_out", "unknown"] as const)("blocks %s sign-in instead of calling it ready", async authState => {
    adapters = [{ ...codex, authState }];
    await render(); await select("Codex"); await click("Continue");
    expect(button("Start using Bridge").disabled).toBe(true);
    expect(button("Sign in to Codex")).toBeTruthy();
    expect(host.textContent).not.toContain("Sign-in found");
  });
  it("cancelling sign-in rechecks state and does not complete setup", async () => {
    adapters = [{ ...codex, authState: "signed_out" }];
    await render(); await select("Codex"); await click("Continue"); await click("Sign in to Codex");
    await click("Cancel");
    expect(bridgeApi.refreshModelCatalogs).toHaveBeenCalled();
    expect(button("Start using Bridge").disabled).toBe(true);
    expect(complete).not.toHaveBeenCalled();
  });
  it("installing a selected missing agent still requires its sign-in", async () => {
    vi.spyOn(bridgeApi, "installManagedAgent").mockImplementation(async agentId => {
      const status: ManagedAgentStatus = { ...missing, backing: "managed", state: "ready", removable: true };
      agents = [installed, status]; adapters = [codex, { ...codex, id: "opencode", label: "OpenCode", authState: "signed_out" }];
      return { agentId, kind: "install", outcome: "installed", status };
    });
    await render(); await select("OpenCode"); await click("Continue");
    expect(button("Start using Bridge").disabled).toBe(true);
    await click("Install with Bridge");
    expect(bridgeApi.installManagedAgent).toHaveBeenCalledWith("opencode");
    expect(host.textContent).toContain("Bridge-managed");
    expect(button("Start using Bridge").disabled).toBe(true);
    adapters = [codex, { ...codex, id: "opencode", label: "OpenCode" }];
    await click("Check again"); await click("Start using Bridge");
    expect(complete).toHaveBeenCalledOnce();
  });
  it("keeps installation failures actionable and lets the user deselect an unfinished agent", async () => {
    vi.spyOn(bridgeApi, "installManagedAgent").mockRejectedValue(new Error("Download failed"));
    await render(); await select("Codex"); await select("OpenCode"); await click("Continue"); await click("Install with Bridge");
    expect(host.textContent).toContain("Download failed");
    expect(button("Start using Bridge").disabled).toBe(true);
    await click("Change agents"); await select("OpenCode"); await click("Continue");
    expect(button("Start using Bridge").disabled).toBe(false);
  });
  it("does not offer installation without a native package recipe", async () => {
    agents = [{ ...missing, pinnedVersion: null }]; adapters = [];
    await render(); await select("OpenCode"); await click("Continue");
    expect(button("Install with Bridge")).toBeUndefined();
    expect(host.textContent).toContain("Bridge cannot install this agent");
    expect(button("Start using Bridge").disabled).toBe(true);
  });
  it("rechecks readiness immediately before completion", async () => {
    await render(); await select("Codex"); await click("Continue");
    adapters = [{ ...codex, authState: "unknown" }];
    await click("Start using Bridge");
    expect(complete).not.toHaveBeenCalled();
    expect(bridgeApi.saveModelProfiles).not.toHaveBeenCalled();
    expect(host.textContent).toContain("One of your selected agents is not ready");
  });
  it("shows save errors and retries instead of entering the interface", async () => {
    vi.mocked(bridgeApi.saveModelProfiles).mockRejectedValueOnce(new Error("Save failed"));
    await render(); await select("Codex"); await click("Continue"); await click("Start using Bridge");
    expect(host.textContent).toContain("Save failed"); expect(complete).not.toHaveBeenCalled();
    await click("Start using Bridge"); expect(complete).toHaveBeenCalledOnce();
  });
  it("offers a retry when discovery fails", async () => {
    vi.mocked(bridgeApi.listManagedAgents).mockRejectedValueOnce(new Error("Detection failed"));
    await render(); expect(host.textContent).toContain("Detection failed"); expect(button("Continue").disabled).toBe(true);
    await click("Check again"); await select("Codex"); expect(button("Continue").disabled).toBe(false);
  });
  it.each(["cursor", "grok"])("completes %s-only setup from unknown auth and an empty discovery catalog", async id => {
    const label = id === "cursor" ? "Cursor" : "Grok";
    adapters = [{ ...codex, id, label, authState: "unknown", models: [] }];
    agents = [{ ...installed, agentId: id, label }];
    const prepare = vi.spyOn(bridgeApi, "prepareAgentSetup").mockImplementation(async () => {
      adapters = [{ ...codex, id, label, models: [{ id: "real-model", label: "Real model", tier: "standard", defaultForTier: true }] }];
      return adapters[0];
    });
    await render(); await select(label); await click("Continue");
    expect(prepare).toHaveBeenCalledWith(id);
    expect(button("Start using Bridge").disabled).toBe(false);
    await click("Start using Bridge");
    expect(prepare).toHaveBeenCalledTimes(2); // Final completion must recheck.
    expect(complete).toHaveBeenCalledOnce();
    expect(vi.mocked(bridgeApi.saveModelProfiles).mock.calls[0][0].every(profile => profile.provider === id && profile.model === "real-model")).toBe(true);
  });
  it("keeps a failed Cursor setup check unresolved and offers sign-in", async () => {
    agents = [{ ...installed, agentId: "cursor", label: "Cursor" }];
    adapters = [{ ...codex, id: "cursor", label: "Cursor", authState: "unknown", models: [] }];
    vi.spyOn(bridgeApi, "prepareAgentSetup").mockImplementation(async () => {
      adapters = [{ ...adapters[0], available: false, authState: "signed_out" }];
      throw new Error("Cursor needs sign-in");
    });
    await render(); await select("Cursor"); await click("Continue");
    expect(button("Start using Bridge").disabled).toBe(true);
    expect(button("Sign in to Cursor")).toBeTruthy();
    expect(host.textContent).toContain("Cursor needs sign-in");
    expect(complete).not.toHaveBeenCalled();
    expect(bridgeApi.saveModelProfiles).not.toHaveBeenCalled();
  });
  it("rechecks Cursor sign-in at final submission and clears stale readiness", async () => {
    agents = [{ ...installed, agentId: "cursor", label: "Cursor" }];
    adapters = [{ ...codex, id: "cursor", label: "Cursor", authState: "unknown", models: [] }];
    vi.spyOn(bridgeApi, "prepareAgentSetup")
      .mockImplementationOnce(async () => {
        adapters = [{ ...codex, id: "cursor", label: "Cursor" }];
        return adapters[0];
      })
      .mockImplementationOnce(async () => {
        adapters = [{ ...adapters[0], available: false, authState: "signed_out", models: [] }];
        throw new Error("Cursor needs sign-in");
      });
    await render(); await select("Cursor"); await click("Continue");
    expect(button("Start using Bridge").disabled).toBe(false);
    await click("Start using Bridge");
    expect(button("Start using Bridge").disabled).toBe(true);
    expect(button("Sign in to Cursor")).toBeTruthy();
    expect(host.textContent).toContain("Cursor needs sign-in");
    expect(complete).not.toHaveBeenCalled();
    expect(bridgeApi.saveModelProfiles).not.toHaveBeenCalled();
  });
  it("can use health-verified agents when installation detection keeps failing", async () => {
    vi.mocked(bridgeApi.listManagedAgents).mockRejectedValue(new Error("Detection failed"));
    await render(); await select("Codex");
    expect(button("Continue").disabled).toBe(true);
    await click("Use detected agents"); await click("Continue");
    expect(host.textContent).toContain("Installation found; source could not be checked");
    expect(button("Install with Bridge")).toBeUndefined();
    await click("Start using Bridge");
    expect(complete).toHaveBeenCalledOnce();
  });
  it("health fallback cannot bypass unknown sign-in", async () => {
    vi.mocked(bridgeApi.listManagedAgents).mockRejectedValue(new Error("Detection failed"));
    adapters = [{ ...codex, authState: "unknown" }];
    await render(); await select("Codex"); await click("Use detected agents"); await click("Continue");
    expect(button("Start using Bridge").disabled).toBe(true);
    expect(complete).not.toHaveBeenCalled();
  });
});
