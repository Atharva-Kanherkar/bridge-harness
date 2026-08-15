// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { bridgeApi } from "../api";
import type { ManagedAgentStatus } from "../protocol/generated/protocol";
import { AgentMarketplace } from "./AgentMarketplace";
import { MarketplaceScreen } from "./MarketplaceScreen";

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

const external = (id: string, label: string) =>
  agent({ agentId: id, label, state: "external", backing: "external", removable: false, version: undefined });

async function render(node: React.ReactElement) {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  await act(async () => { root.render(node); });
  return {
    host,
    text: () => host.textContent ?? "",
    button: (label: string) =>
      [...host.querySelectorAll("button")].find(item => item.textContent?.trim() === label) ?? null,
    buttons: () => [...host.querySelectorAll("button")].map(item => item.textContent?.trim()),
    click: async (item: Element | null) => {
      expect(item, "control must exist to be clicked").not.toBeNull();
      await act(async () => { (item as HTMLButtonElement).click(); });
    },
    unmount: () => act(async () => { root.unmount(); host.remove(); }),
  };
}

afterEach(() => { vi.restoreAllMocks(); document.body.innerHTML = ""; });

describe("AgentMarketplace", () => {
  it("shows Install for an agent Bridge has not installed", async () => {
    vi.spyOn(bridgeApi, "listManagedAgents").mockResolvedValue({
      agents: [external("claude", "Claude Code")],
    } as Awaited<ReturnType<typeof bridgeApi.listManagedAgents>>);

    const view = await render(<AgentMarketplace/>);
    expect(view.button("Install")).not.toBeNull();
    expect(view.button("Uninstall")).toBeNull();
    await view.unmount();
  });

  it("shows Uninstall for an agent Bridge installed", async () => {
    vi.spyOn(bridgeApi, "listManagedAgents").mockResolvedValue({
      agents: [agent()],
    } as Awaited<ReturnType<typeof bridgeApi.listManagedAgents>>);

    const view = await render(<AgentMarketplace/>);
    expect(view.button("Uninstall")).not.toBeNull();
    expect(view.button("Install")).toBeNull();
    await view.unmount();
  });

  it("installs, and the card flips to Uninstall", async () => {
    vi.spyOn(bridgeApi, "listManagedAgents").mockResolvedValue({
      agents: [external("claude", "Claude Code")],
    } as Awaited<ReturnType<typeof bridgeApi.listManagedAgents>>);
    const install = vi.spyOn(bridgeApi, "installManagedAgent").mockResolvedValue({
      agentId: "claude",
      status: agent({ agentId: "claude", label: "Claude Code", removable: true }),
    } as Awaited<ReturnType<typeof bridgeApi.installManagedAgent>>);

    const view = await render(<AgentMarketplace/>);
    await view.click(view.button("Install"));
    expect(install).toHaveBeenCalledWith("claude");
    expect(view.button("Uninstall")).not.toBeNull();
    await view.unmount();
  });

  it("uninstalls, and the card flips to Install", async () => {
    vi.spyOn(bridgeApi, "listManagedAgents").mockResolvedValue({
      agents: [agent()],
    } as Awaited<ReturnType<typeof bridgeApi.listManagedAgents>>);
    const uninstall = vi.spyOn(bridgeApi, "uninstallManagedAgent").mockResolvedValue({
      agentId: "codex",
      status: external("codex", "Codex"),
    } as Awaited<ReturnType<typeof bridgeApi.uninstallManagedAgent>>);

    const view = await render(<AgentMarketplace/>);
    await view.click(view.button("Uninstall"));
    expect(uninstall).toHaveBeenCalledWith("codex");
    expect(view.button("Install")).not.toBeNull();
    await view.unmount();
  });

  it("shows the reason when an install fails, and leaves the button", async () => {
    vi.spyOn(bridgeApi, "listManagedAgents").mockResolvedValue({
      agents: [external("claude", "Claude Code")],
    } as Awaited<ReturnType<typeof bridgeApi.listManagedAgents>>);
    vi.spyOn(bridgeApi, "installManagedAgent").mockRejectedValue(new Error("registry unreachable"));

    const view = await render(<AgentMarketplace/>);
    await view.click(view.button("Install"));
    expect(view.text()).toContain("registry unreachable");
    expect(view.button("Install")).not.toBeNull();
    await view.unmount();
  });

  it("never offers Uninstall for a copy Bridge does not own", async () => {
    // The one rule that is not cosmetic: `removable` is the API's answer to
    // "is this Bridge's to delete", and a PATH install is the user's.
    vi.spyOn(bridgeApi, "listManagedAgents").mockResolvedValue({
      agents: [external("claude", "Claude Code"), external("codex", "Codex"), agent({ agentId: "opencode", label: "OpenCode" })],
    } as Awaited<ReturnType<typeof bridgeApi.listManagedAgents>>);

    const view = await render(<AgentMarketplace/>);
    expect(view.buttons().filter(label => label === "Uninstall")).toHaveLength(1);
    expect(view.buttons().filter(label => label === "Install")).toHaveLength(2);
    await view.unmount();
  });
});

describe("MarketplaceScreen", () => {
  it("offers Agents alongside Plugins and Skills, and opens on Agents", async () => {
    vi.spyOn(bridgeApi, "listManagedAgents").mockResolvedValue({
      agents: [agent()],
    } as Awaited<ReturnType<typeof bridgeApi.listManagedAgents>>);
    vi.spyOn(bridgeApi, "marketplaceCatalog").mockResolvedValue(
      { providers: [], services: [] } as unknown as Awaited<ReturnType<typeof bridgeApi.marketplaceCatalog>>,
    );

    const view = await render(<MarketplaceScreen/>);
    const tabs = [...view.host.querySelectorAll(".u-segmented-item")].map(item => item.textContent?.trim());
    expect(tabs).toEqual(["agents", "plugins", "skills"]);
    expect(view.host.querySelector('[data-active="true"]')?.textContent?.trim()).toBe("agents");
    await view.unmount();
  });
});
