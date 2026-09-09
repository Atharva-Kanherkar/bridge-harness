// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { adapterSupportsAgentRole, PermissionsSection } from "./SettingsScreen";
import type { AdapterDescriptor, BridgeEvent, PermissionPolicy } from "../types";

const policy = (autoApproveProviderPermissions: boolean): PermissionPolicy => ({ autoApproveProviderPermissions, workerPromptProposalRoles: [], updatedAt: "2026-08-21T10:00:00Z" });
const ledgerRow = (id: number, body: string): BridgeEvent => ({
  id, source: "approval", kind: "approval.auto_allowed", entityId: "chat", body,
  createdAt: "2026-08-21T10:00:00Z",
});

async function mount(node: React.ReactElement) {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(node));
  return { container, unmount: () => act(async () => root.unmount()) };
}

describe("PermissionsSection", () => {
  it("shows the switch off by default and does not claim to be bypassing", async () => {
    const { container, unmount } = await mount(
      <PermissionsSection policy={policy(false)} autoApprovals={[]} busy={false} onChange={() => undefined} />,
    );
    const toggle = container.querySelector<HTMLButtonElement>('[role="switch"]')!;
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    // The state is the switch, not a word beside it, and never a native
    // checkbox: Settings renders no `input[type=checkbox]` anywhere.
    expect(container.querySelector('input[type="checkbox"]')).toBeNull();
    await unmount();
  });

  /// The issue makes this copy part of the contract: a switch that claims to
  /// silence everything and then still prompts has to say where, up front.
  it("names the host gates that keep asking either way", async () => {
    const { container, unmount } = await mount(
      <PermissionsSection policy={policy(true)} autoApprovals={[]} busy={false} onChange={() => undefined} />,
    );
    expect(container.textContent).toContain("Worker write scope");
    expect(container.textContent).toContain("Browser outward effects");
    expect(container.textContent).toContain("Agent prompt changes");
    expect(container.textContent).toContain("These keep asking either way");
    expect(container.textContent).toContain("Auto-approve provider permissions");
    expect(container.textContent).toContain("Questions and macOS prompts still wait for you");
    expect(container.textContent).not.toContain("Bypass all approvals");
    await unmount();
  });

  it("reports the requested flip rather than applying it locally", async () => {
    // The switch is a security control: it must render from what the host
    // stored, so the component asks and re-renders from the returned policy.
    const onChange = vi.fn();
    const { container, unmount } = await mount(
      <PermissionsSection policy={policy(false)} autoApprovals={[]} busy={false} onChange={onChange} />,
    );
    await act(async () => container.querySelector<HTMLButtonElement>('[role="switch"]')!.click());
    expect(onChange).toHaveBeenCalledWith({ ...policy(false), autoApproveProviderPermissions: true });
    // Still off in the DOM: nothing changed until the host says so.
    expect(container.querySelector('[role="switch"]')!.getAttribute("aria-checked")).toBe("false");
    await unmount();
  });

  it("lists recent auto-approvals so a bypassed approval is auditable", async () => {
    const { container, unmount } = await mount(
      <PermissionsSection
        policy={policy(true)}
        autoApprovals={[ledgerRow(1, "bypass_all granted approval 12"), ledgerRow(2, "bypass_all granted approval 15")]}
        busy={false}
        onChange={() => undefined}
      />,
    );
    expect(container.textContent).toContain("bypass_all granted approval 12");
    expect(container.textContent).toContain("bypass_all granted approval 15");
    expect(container.textContent).not.toContain("Nothing has been auto-approved yet");
    await unmount();
  });

  it("says so plainly when nothing has been auto-approved", async () => {
    const { container, unmount } = await mount(
      <PermissionsSection policy={policy(true)} autoApprovals={[]} busy={false} onChange={() => undefined} />,
    );
    expect(container.textContent).toContain("Nothing has been auto-approved yet");
    await unmount();
  });

  it("cannot be flipped while a save is in flight", async () => {
    const onChange = vi.fn();
    const { container, unmount } = await mount(
      <PermissionsSection policy={policy(false)} autoApprovals={[]} busy onChange={onChange} />,
    );
    const toggle = container.querySelector<HTMLButtonElement>('[role="switch"]')!;
    expect(toggle.disabled).toBe(true);
    await act(async () => toggle.click());
    expect(onChange).not.toHaveBeenCalled();
    await unmount();
  });

  it("keeps every worker proposal role off by default even with provider auto-approval", async () => {
    const { container, unmount } = await mount(
      <PermissionsSection policy={policy(true)} autoApprovals={[]} busy={false} onChange={() => undefined} />,
    );
    const toggles = container.querySelectorAll('[role="switch"][aria-label$="prompt proposals"]');
    expect(toggles.length).toBe(5);
    expect([...toggles].every(item => item.getAttribute("aria-checked") === "false")).toBe(true);
    expect(container.textContent).toContain("You review every change before it is saved to the shared role default");
    await unmount();
  });

  it.each([true, false])("requests one role opt-in change while preserving other grants (enabled=%s)", async enabled => {
    const current: PermissionPolicy = { ...policy(true), workerPromptProposalRoles: enabled ? ["research"] : ["research", "implementation"] };
    const onChange = vi.fn();
    const { container, unmount } = await mount(
      <PermissionsSection policy={current} autoApprovals={[]} busy={false} onChange={onChange} />,
    );
    const toggle = container.querySelector<HTMLButtonElement>('[aria-label="Allow implementation prompt proposals"]')!;
    await act(async () => toggle.click());
    expect(onChange).toHaveBeenCalledWith({ ...current, workerPromptProposalRoles: enabled ? ["research", "implementation"] : ["research"] });
    // The saved host policy owns the visible state even after a requested flip.
    expect(toggle.getAttribute("aria-checked")).toBe(String(!enabled));
    await unmount();
  });

  it("disables every role grant while the host saves", async () => {
    const onChange = vi.fn();
    const { container, unmount } = await mount(
      <PermissionsSection policy={policy(false)} autoApprovals={[]} busy onChange={onChange} />,
    );
    const toggles = [...container.querySelectorAll<HTMLButtonElement>('[role="switch"][aria-label$="prompt proposals"]')];
    expect(toggles.every(item => item.disabled)).toBe(true);
    await act(async () => { for (const toggle of toggles) toggle.click(); });
    expect(onChange).not.toHaveBeenCalled();
    await unmount();
  });
});

describe("adapterSupportsAgentRole", () => {
  const cursor: AdapterDescriptor = {
    id: "cursor", label: "Cursor", available: true, authState: "signed_in", version: "test",
    capabilities: ["messages"], sandboxModes: ["workspace_write", "danger_full_access"],
    unavailableReason: null, models: [], defaultModel: null,
  };

  it("keeps Cursor available for implementation but not roles its adapter rejects", () => {
    expect(adapterSupportsAgentRole(cursor, "implementation")).toBe(true);
    expect(adapterSupportsAgentRole(cursor, "research")).toBe(false);
    expect(adapterSupportsAgentRole(cursor, "verification")).toBe(false);
    expect(adapterSupportsAgentRole(cursor, "planning")).toBe(false);
    expect(adapterSupportsAgentRole(cursor, "documentation")).toBe(false);
    expect(adapterSupportsAgentRole(cursor, "orchestrator")).toBe(false);
  });
});
